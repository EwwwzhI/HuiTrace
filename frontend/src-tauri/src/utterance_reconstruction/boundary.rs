use super::config::{
    UtteranceReconstructionConfig, CONTINUATION_PREFIXES, STRONG_TERMINAL_PUNCTUATION,
    WEAK_PUNCTUATION,
};
use super::types::{
    AtomicSpan, BoundaryDecision, BoundaryEvidence, BoundaryOutcome, BoundaryReason,
    BoundaryScoreComponents, ProsodicBoundaryEvidence, SpeakerAttribution,
};

pub fn decide_boundary(
    left: &AtomicSpan,
    right: &AtomicSpan,
    left_context: &str,
    utterance_start_ms: i64,
    projected_text_length: usize,
    backchannel_between: bool,
    config: &UtteranceReconstructionConfig,
) -> BoundaryOutcome {
    let gap_ms =
        (left.timing_reliable && right.timing_reliable).then_some(right.start_ms - left.end_ms);
    let same_speaker = speaker_continuity(&left.speaker_attribution, &right.speaker_attribution);
    let speaker_change_confidence = speaker_change_confidence(left, right);
    let speaker_change_reliable = same_speaker == Some(false)
        && left.timing_reliable
        && right.timing_reliable
        && speaker_change_confidence
            .is_some_and(|confidence| confidence >= config.reliable_speaker_change_confidence);
    let strong_terminal_punctuation = ends_with(left.text.trim_end(), STRONG_TERMINAL_PUNCTUATION);
    let weak_punctuation = ends_with(left.text.trim_end(), WEAK_PUNCTUATION);
    let continuation_prefix = CONTINUATION_PREFIXES
        .iter()
        .any(|prefix| right.text.trim_start().starts_with(prefix));
    let projected_duration_ms = right.end_ms.saturating_sub(utterance_start_ms);
    let mixed_attribution =
        left.speaker_attribution.is_mixed() || right.speaker_attribution.is_mixed();
    let overlap = left.overlap || right.overlap || gap_ms.is_some_and(|gap| gap < 0);
    let semantic = super::semantic::evaluate(left_context, &right.text);
    let evidence = BoundaryEvidence {
        left_source_transcript_id: left.source_transcript_ids[0].clone(),
        right_source_transcript_id: right.source_transcript_ids[0].clone(),
        gap_ms,
        same_speaker,
        speaker_change_confidence,
        speaker_change_reliable,
        timing_reliable: left.timing_reliable && right.timing_reliable,
        strong_terminal_punctuation,
        weak_punctuation,
        continuation_prefix,
        projected_duration_ms,
        projected_text_length,
        mixed_attribution,
        overlap,
        backchannel_between,
        semantic,
        prosody: ProsodicBoundaryEvidence { available: false },
    };

    let mut reasons = Vec::new();
    let mut components = BoundaryScoreComponents::default();
    if !evidence.timing_reliable {
        reasons.push(BoundaryReason::UnreliableTiming);
    }
    if mixed_attribution {
        reasons.push(BoundaryReason::MixedAttribution);
    }
    if overlap {
        reasons.push(if gap_ms.is_some_and(|gap| gap < 0) {
            BoundaryReason::OverlappingTimeline
        } else {
            BoundaryReason::Overlap
        });
    }
    if speaker_change_reliable {
        reasons.push(BoundaryReason::SpeakerChanged);
        reasons.push(BoundaryReason::ReliableSpeakerChange);
        return hard_split(evidence, reasons, components);
    }
    if gap_ms.is_some_and(|gap| gap >= config.long_silence_ms) {
        reasons.push(BoundaryReason::LongSilence);
        return hard_split(evidence, reasons, components);
    }
    if evidence.timing_reliable && projected_duration_ms > config.max_utterance_duration_ms {
        reasons.push(BoundaryReason::MaximumDuration);
        return hard_split(evidence, reasons, components);
    }
    if projected_text_length > config.max_text_length {
        reasons.push(BoundaryReason::MaximumLength);
        return hard_split(evidence, reasons, components);
    }

    match same_speaker {
        Some(true) => {
            components.speaker_score += config.same_speaker_score;
            reasons.push(BoundaryReason::SameSpeaker);
        }
        Some(false) => {
            reasons.push(BoundaryReason::SpeakerChanged);
            if speaker_change_confidence
                .is_some_and(|confidence| confidence < config.reliable_speaker_change_confidence)
                || !evidence.timing_reliable
            {
                components.speaker_score += config.ambiguous_speaker_changed_score;
                reasons.push(BoundaryReason::AmbiguousSpeakerChange);
            } else {
                components.speaker_score += config.speaker_changed_score;
            }
        }
        None => reasons.push(BoundaryReason::SpeakerUnknown),
    }
    if gap_ms.is_some_and(|gap| gap <= config.short_gap_ms) {
        components.timing_score += config.short_gap_score;
        reasons.push(BoundaryReason::ShortGap);
    } else if gap_ms.is_some_and(|gap| gap >= config.medium_gap_ms) {
        components.timing_score += config.medium_gap_score;
        reasons.push(BoundaryReason::MediumGap);
    }
    if strong_terminal_punctuation {
        components.punctuation_score += config.terminal_punctuation_score;
        reasons.push(BoundaryReason::StrongTerminalPunctuation);
    }
    if weak_punctuation {
        components.punctuation_score += config.weak_punctuation_score;
        reasons.push(BoundaryReason::WeakPunctuation);
    }
    if continuation_prefix {
        components.semantic_score += config.continuation_prefix_score;
        reasons.push(BoundaryReason::ContinuationPrefix);
    }
    if config.semantic_boundary_enabled {
        if evidence
            .semantic
            .left_completeness
            .is_some_and(|score| score >= config.completeness_split_threshold)
        {
            components.semantic_score += config.completeness_split_score;
            reasons.push(BoundaryReason::SentenceComplete);
        } else if evidence
            .semantic
            .left_completeness
            .is_some_and(|score| score < 0.5)
        {
            components.semantic_score += config.incomplete_merge_score;
            reasons.push(BoundaryReason::SentenceIncomplete);
        }
        if evidence
            .semantic
            .cross_boundary_continuity
            .is_some_and(|score| score >= config.continuity_merge_threshold)
        {
            components.semantic_score += config.continuity_merge_score;
            reasons.push(BoundaryReason::SemanticContinuity);
        }
    }
    if backchannel_between {
        components.structural_score += config.backchannel_bridge_score;
        reasons.push(BoundaryReason::BackchannelBridge);
    }
    let score = components.timing_score
        + components.speaker_score
        + components.punctuation_score
        + components.semantic_score
        + components.structural_score;
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
        score_components: components,
    }
}

fn speaker_change_confidence(left: &AtomicSpan, right: &AtomicSpan) -> Option<f64> {
    match (left.speaker_confidence, right.speaker_confidence) {
        (Some(left), Some(right)) => Some(left.min(right).clamp(0.0, 1.0)),
        _ => None,
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

fn hard_split(
    evidence: BoundaryEvidence,
    reasons: Vec<BoundaryReason>,
    score_components: BoundaryScoreComponents,
) -> BoundaryOutcome {
    BoundaryOutcome {
        evidence,
        score: i32::MAX,
        decision: BoundaryDecision::Split,
        reasons,
        score_components,
    }
}
