use super::config::{
    UtteranceReconstructionConfig, CONTINUATION_PREFIXES, STRONG_TERMINAL_PUNCTUATION,
    WEAK_PUNCTUATION,
};
use super::types::{
    AtomicSpan, BoundaryDecision, BoundaryEvidence, BoundaryOutcome, BoundaryReason,
    SpeakerAttribution,
};

pub fn decide_boundary(
    left: &AtomicSpan,
    right: &AtomicSpan,
    utterance_start_ms: i64,
    projected_text_length: usize,
    backchannel_between: bool,
    config: &UtteranceReconstructionConfig,
) -> BoundaryOutcome {
    let gap_ms =
        (left.timing_reliable && right.timing_reliable).then_some(right.start_ms - left.end_ms);
    let same_speaker = speaker_continuity(&left.speaker_attribution, &right.speaker_attribution);
    let strong_terminal_punctuation = ends_with(left.text.trim_end(), STRONG_TERMINAL_PUNCTUATION);
    let weak_punctuation = ends_with(left.text.trim_end(), WEAK_PUNCTUATION);
    let continuation_prefix = CONTINUATION_PREFIXES
        .iter()
        .any(|prefix| right.text.trim_start().starts_with(prefix));
    let projected_duration_ms = right.end_ms.saturating_sub(utterance_start_ms);
    let mixed_attribution =
        left.speaker_attribution.is_mixed() || right.speaker_attribution.is_mixed();
    let overlap = left.overlap || right.overlap || gap_ms.is_some_and(|gap| gap < 0);
    let evidence = BoundaryEvidence {
        left_source_transcript_id: left.source_transcript_ids[0].clone(),
        right_source_transcript_id: right.source_transcript_ids[0].clone(),
        gap_ms,
        same_speaker,
        strong_terminal_punctuation,
        weak_punctuation,
        continuation_prefix,
        projected_duration_ms,
        projected_text_length,
        mixed_attribution,
        overlap,
        backchannel_between,
    };

    let mut reasons = Vec::new();
    if !left.timing_reliable || !right.timing_reliable {
        reasons.push(BoundaryReason::UnreliableTiming);
        return hard_split(evidence, reasons);
    }
    if mixed_attribution {
        reasons.push(BoundaryReason::MixedAttribution);
        return hard_split(evidence, reasons);
    }
    if overlap {
        reasons.push(if gap_ms.is_some_and(|gap| gap < 0) {
            BoundaryReason::OverlappingTimeline
        } else {
            BoundaryReason::Overlap
        });
        return hard_split(evidence, reasons);
    }
    if same_speaker == Some(false) {
        reasons.push(BoundaryReason::SpeakerChanged);
        return hard_split(evidence, reasons);
    }
    if gap_ms.is_some_and(|gap| gap >= config.long_silence_ms) {
        reasons.push(BoundaryReason::LongSilence);
        return hard_split(evidence, reasons);
    }
    if projected_duration_ms > config.max_utterance_duration_ms {
        reasons.push(BoundaryReason::MaximumDuration);
        return hard_split(evidence, reasons);
    }
    if projected_text_length > config.max_text_length {
        reasons.push(BoundaryReason::MaximumLength);
        return hard_split(evidence, reasons);
    }

    let mut score = 0;
    match same_speaker {
        Some(true) => {
            score += config.same_speaker_score;
            reasons.push(BoundaryReason::SameSpeaker);
        }
        Some(false) => {
            score += config.speaker_changed_score;
            reasons.push(BoundaryReason::SpeakerChanged);
        }
        None => reasons.push(BoundaryReason::SpeakerUnknown),
    }
    if gap_ms.is_some_and(|gap| gap <= config.short_gap_ms) {
        score += config.short_gap_score;
        reasons.push(BoundaryReason::ShortGap);
    } else if gap_ms.is_some_and(|gap| gap >= config.medium_gap_ms) {
        score += config.medium_gap_score;
        reasons.push(BoundaryReason::MediumGap);
    }
    if strong_terminal_punctuation {
        score += config.terminal_punctuation_score;
        reasons.push(BoundaryReason::StrongTerminalPunctuation);
    }
    if weak_punctuation {
        score += config.weak_punctuation_score;
        reasons.push(BoundaryReason::WeakPunctuation);
    }
    if continuation_prefix {
        score += config.continuation_prefix_score;
        reasons.push(BoundaryReason::ContinuationPrefix);
    }
    if backchannel_between {
        score += config.backchannel_bridge_score;
        reasons.push(BoundaryReason::BackchannelBridge);
    }
    let decision = if score >= config.split_score_threshold {
        reasons.push(BoundaryReason::ScoreThreshold);
        BoundaryDecision::Split
    } else {
        reasons.push(BoundaryReason::BelowScoreThreshold);
        BoundaryDecision::Merge
    };
    BoundaryOutcome {
        evidence,
        score,
        decision,
        reasons,
    }
}

fn speaker_continuity(left: &SpeakerAttribution, right: &SpeakerAttribution) -> Option<bool> {
    match (left.single_key(), right.single_key()) {
        (Some(left), Some(right)) => Some(left == right),
        _ => None,
    }
}

fn ends_with(text: &str, punctuation: &[char]) -> bool {
    text.chars()
        .next_back()
        .is_some_and(|character| punctuation.contains(&character))
}

fn hard_split(evidence: BoundaryEvidence, reasons: Vec<BoundaryReason>) -> BoundaryOutcome {
    BoundaryOutcome {
        evidence,
        score: i32::MAX,
        decision: BoundaryDecision::Split,
        reasons,
    }
}
