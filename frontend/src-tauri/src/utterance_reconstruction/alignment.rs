use std::collections::BTreeMap;

use crate::database::repositories::speaker_turn::SpeakerTurn;

use super::config::UtteranceReconstructionConfig;
use super::types::{
    AlignmentReason, SpeakerAttributionSource, SpeakerCandidate, TimedWord, WordSpeakerAssignment,
    WordSpeakerStatus,
};

pub fn align_words(
    words: &[TimedWord],
    turns: &[SpeakerTurn],
    manual_speaker: Option<&str>,
    force_overlap: bool,
    config: &UtteranceReconstructionConfig,
) -> Vec<WordSpeakerAssignment> {
    words
        .iter()
        .cloned()
        .map(|word| align_word(word, turns, manual_speaker, force_overlap, config))
        .collect()
}

fn align_word(
    word: TimedWord,
    turns: &[SpeakerTurn],
    manual_speaker: Option<&str>,
    force_overlap: bool,
    config: &UtteranceReconstructionConfig,
) -> WordSpeakerAssignment {
    let tolerance = config.effective_alignment_tolerance_ms();
    let evidence_start = word.start_ms.saturating_sub(tolerance);
    let evidence_end = word.end_ms.max(word.start_ms).saturating_add(tolerance);
    let evidence_end = evidence_end.max(evidence_start + 1);
    let duration = evidence_end - evidence_start;
    let mut by_speaker: BTreeMap<String, Vec<(i64, i64)>> = BTreeMap::new();
    for turn in turns {
        let start = turn.start_ms.max(evidence_start);
        let end = turn.end_ms.min(evidence_end);
        if end > start && !turn.speaker_key.is_empty() {
            by_speaker
                .entry(turn.speaker_key.clone())
                .or_default()
                .push((start, end));
        }
    }
    let mut candidates = by_speaker
        .into_iter()
        .map(|(speaker_key, intervals)| {
            let overlap_ms = union_duration(intervals);
            SpeakerCandidate {
                speaker_key,
                overlap_ms,
                overlap_ratio: overlap_ms as f64 / duration as f64,
            }
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .overlap_ratio
            .total_cmp(&left.overlap_ratio)
            .then(left.speaker_key.cmp(&right.speaker_key))
    });

    let best = candidates
        .first()
        .map(|candidate| candidate.overlap_ratio)
        .unwrap_or(0.0);
    let second = candidates
        .get(1)
        .map(|candidate| candidate.overlap_ratio)
        .unwrap_or(0.0);
    let margin = best - second;
    let simultaneous = force_overlap
        || candidates.get(1).is_some_and(|second_candidate| {
            turns_overlap_in_evidence(
                turns,
                &candidates[0].speaker_key,
                &second_candidate.speaker_key,
                evidence_start,
                evidence_end,
                config.true_overlap_min_ms.max(1),
            )
        });

    if simultaneous && candidates.len() > 1 {
        return assignment(
            word,
            None,
            WordSpeakerStatus::Mixed,
            best,
            candidates,
            AlignmentReason::TrueOverlap,
        );
    }
    if force_overlap {
        return assignment(
            word,
            None,
            WordSpeakerStatus::Mixed,
            best,
            candidates,
            AlignmentReason::TrueOverlap,
        );
    }
    if let Some(speaker_key) = manual_speaker.filter(|key| !key.is_empty()) {
        return assignment(
            word,
            Some(speaker_key.to_string()),
            WordSpeakerStatus::Assigned,
            best,
            candidates,
            AlignmentReason::ManualAssignment,
        );
    }
    if candidates.is_empty() {
        return assignment(
            word,
            None,
            WordSpeakerStatus::Unknown,
            0.0,
            candidates,
            AlignmentReason::NoSpeakerEvidence,
        );
    }
    if best < config.assignment_min_overlap_ratio {
        return assignment(
            word,
            None,
            WordSpeakerStatus::Ambiguous,
            best,
            candidates,
            AlignmentReason::InsufficientOverlap,
        );
    }
    if margin < config.assignment_min_margin {
        return assignment(
            word,
            None,
            WordSpeakerStatus::Ambiguous,
            best,
            candidates,
            AlignmentReason::InsufficientMargin,
        );
    }
    let speaker_key = candidates[0].speaker_key.clone();
    assignment(
        word,
        Some(speaker_key),
        WordSpeakerStatus::Assigned,
        best,
        candidates,
        AlignmentReason::DominantTemporalOverlap,
    )
}

fn assignment(
    word: TimedWord,
    speaker_key: Option<String>,
    status: WordSpeakerStatus,
    best_overlap_ratio: f64,
    candidates: Vec<SpeakerCandidate>,
    reason: AlignmentReason,
) -> WordSpeakerAssignment {
    let (attribution_source, assignment_reliability) = match &reason {
        AlignmentReason::ManualAssignment => (SpeakerAttributionSource::Manual, None),
        AlignmentReason::DominantTemporalOverlap => (
            SpeakerAttributionSource::LexicalTemporalOverlap,
            Some(best_overlap_ratio),
        ),
        AlignmentReason::NoSpeakerEvidence => (SpeakerAttributionSource::Unknown, None),
        AlignmentReason::InsufficientOverlap
        | AlignmentReason::InsufficientMargin
        | AlignmentReason::TrueOverlap => (SpeakerAttributionSource::LexicalTemporalOverlap, None),
    };
    WordSpeakerAssignment {
        word,
        speaker_key,
        status,
        attribution_source,
        assignment_reliability,
        best_overlap_ratio,
        candidates,
        reasons: vec![reason],
    }
}

fn union_duration(mut intervals: Vec<(i64, i64)>) -> i64 {
    intervals.sort_unstable();
    let mut total = 0;
    let mut current: Option<(i64, i64)> = None;
    for (start, end) in intervals {
        match current {
            Some((current_start, current_end)) if start <= current_end => {
                current = Some((current_start, current_end.max(end)));
            }
            Some((current_start, current_end)) => {
                total += current_end - current_start;
                current = Some((start, end));
            }
            None => current = Some((start, end)),
        }
    }
    if let Some((start, end)) = current {
        total += end - start;
    }
    total
}

fn turns_overlap_in_evidence(
    turns: &[SpeakerTurn],
    left_key: &str,
    right_key: &str,
    evidence_start: i64,
    evidence_end: i64,
    minimum_overlap_ms: i64,
) -> bool {
    turns
        .iter()
        .filter(|turn| turn.speaker_key == left_key)
        .any(|left| {
            turns
                .iter()
                .filter(|turn| turn.speaker_key == right_key)
                .any(|right| {
                    left.end_ms.min(right.end_ms).min(evidence_end)
                        - left.start_ms.max(right.start_ms).max(evidence_start)
                        >= minimum_overlap_ms
                })
        })
}
