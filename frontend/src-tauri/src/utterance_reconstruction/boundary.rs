use super::config::{
    UtteranceReconstructionConfig, CONTINUATION_PREFIXES, STRONG_TERMINAL_PUNCTUATION,
    WEAK_PUNCTUATION,
};
use super::types::{
    AtomicSpan, BoundaryDecision, BoundaryDecisionSource, BoundaryEvidence, BoundaryOutcome,
    BoundaryPolicy, BoundaryReason, BoundaryScoreComponents, ProsodicBoundaryEvidence,
    SpeakerAttribution, SpeakerAttributionSource,
};

#[allow(clippy::too_many_arguments)]
pub fn decide_boundary(
    policy: BoundaryPolicy,
    left: &AtomicSpan,
    right: &AtomicSpan,
    left_context: &str,
    utterance_start_ms: i64,
    projected_text_length: usize,
    backchannel_between: bool,
    config: &UtteranceReconstructionConfig,
) -> BoundaryOutcome {
    match policy {
        BoundaryPolicy::V1Frozen => decide_v1_frozen(
            left,
            right,
            utterance_start_ms,
            projected_text_length,
            backchannel_between,
            config,
        ),
        BoundaryPolicy::V3SemanticBaseline => decide_v3(
            left,
            right,
            left_context,
            utterance_start_ms,
            projected_text_length,
            backchannel_between,
            config,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn evidence(
    left: &AtomicSpan,
    right: &AtomicSpan,
    left_context: &str,
    utterance_start_ms: i64,
    projected_text_length: usize,
    backchannel_between: bool,
    semantic_enabled: bool,
    reliable_threshold: f64,
) -> BoundaryEvidence {
    let timing_reliable = left.timing_reliable && right.timing_reliable;
    let gap_ms = timing_reliable.then_some(right.start_ms - left.end_ms);
    let same_speaker = speaker_continuity(&left.speaker_attribution, &right.speaker_attribution);
    let speaker_change_reliability = match (
        left.speaker_assignment_reliability,
        right.speaker_assignment_reliability,
    ) {
        (Some(left), Some(right)) => Some(left.min(right).clamp(0.0, 1.0)),
        _ => None,
    };
    let speaker_change_reliable = same_speaker == Some(false)
        && timing_reliable
        && assignment_is_trustworthy(left, reliable_threshold)
        && assignment_is_trustworthy(right, reliable_threshold);
    let strong_terminal_punctuation = ends_with(left.text.trim_end(), STRONG_TERMINAL_PUNCTUATION);
    let weak_punctuation = ends_with(left.text.trim_end(), WEAK_PUNCTUATION);
    let continuation_prefix = CONTINUATION_PREFIXES
        .iter()
        .any(|prefix| right.text.trim_start().starts_with(prefix));
    BoundaryEvidence {
        left_source_transcript_id: left.source_transcript_ids[0].clone(),
        right_source_transcript_id: right.source_transcript_ids[0].clone(),
        gap_ms,
        same_speaker,
        speaker_change_reliability,
        speaker_change_reliable,
        left_speaker_attribution_source: left.speaker_attribution_source,
        right_speaker_attribution_source: right.speaker_attribution_source,
        timing_reliable,
        strong_terminal_punctuation,
        weak_punctuation,
        continuation_prefix,
        projected_duration_ms: right.end_ms.saturating_sub(utterance_start_ms),
        projected_text_length,
        mixed_attribution: left.speaker_attribution.is_mixed()
            || right.speaker_attribution.is_mixed(),
        overlap: left.overlap || right.overlap || gap_ms.is_some_and(|gap| gap < 0),
        backchannel_between,
        semantic: semantic_enabled.then(|| super::semantic::evaluate(left_context, &right.text)),
        prosody: ProsodicBoundaryEvidence { available: false },
    }
}

fn decide_v1_frozen(
    left: &AtomicSpan,
    right: &AtomicSpan,
    utterance_start_ms: i64,
    projected_text_length: usize,
    backchannel_between: bool,
    config: &UtteranceReconstructionConfig,
) -> BoundaryOutcome {
    // Exact boundary behavior from 416b807. Keep V3 semantics and reliability out.
    let evidence = evidence(
        left,
        right,
        &left.text,
        utterance_start_ms,
        projected_text_length,
        backchannel_between,
        false,
        config.reliable_speaker_change_threshold,
    );
    let mut reasons = Vec::new();
    let mut components = BoundaryScoreComponents::default();

    if !evidence.timing_reliable {
        reasons.push(BoundaryReason::UnreliableTiming);
        return hard_split(
            evidence,
            reasons,
            components,
            BoundaryDecisionSource::FrozenPolicyHardSplit,
        );
    }
    if evidence.mixed_attribution {
        reasons.push(BoundaryReason::MixedAttribution);
        return hard_split(
            evidence,
            reasons,
            components,
            BoundaryDecisionSource::FrozenPolicyHardSplit,
        );
    }
    if evidence.overlap {
        reasons.push(if evidence.gap_ms.is_some_and(|gap| gap < 0) {
            BoundaryReason::OverlappingTimeline
        } else {
            BoundaryReason::Overlap
        });
        return hard_split(
            evidence,
            reasons,
            components,
            BoundaryDecisionSource::FrozenPolicyHardSplit,
        );
    }
    if evidence.same_speaker == Some(false) {
        reasons.push(BoundaryReason::SpeakerChanged);
        return hard_split(
            evidence,
            reasons,
            components,
            BoundaryDecisionSource::FrozenPolicyHardSplit,
        );
    }
    if evidence
        .gap_ms
        .is_some_and(|gap| gap >= config.long_silence_ms)
    {
        reasons.push(BoundaryReason::LongSilence);
        return hard_split(
            evidence,
            reasons,
            components,
            BoundaryDecisionSource::FrozenPolicyHardSplit,
        );
    }
    if evidence.projected_duration_ms > config.max_utterance_duration_ms {
        reasons.push(BoundaryReason::MaximumDuration);
        return hard_split(
            evidence,
            reasons,
            components,
            BoundaryDecisionSource::FrozenPolicyHardSplit,
        );
    }
    if projected_text_length > config.max_text_length {
        reasons.push(BoundaryReason::MaximumLength);
        return hard_split(
            evidence,
            reasons,
            components,
            BoundaryDecisionSource::FrozenPolicyHardSplit,
        );
    }

    if evidence.same_speaker == Some(true) {
        components.speaker_score += config.same_speaker_score;
        reasons.push(BoundaryReason::SameSpeaker);
    } else {
        reasons.push(BoundaryReason::SpeakerUnknown);
    }
    apply_legacy_scores(
        &evidence,
        backchannel_between,
        config,
        &mut reasons,
        &mut components,
    );
    scored(evidence, reasons, components, config)
}

#[allow(clippy::too_many_arguments)]
fn decide_v3(
    left: &AtomicSpan,
    right: &AtomicSpan,
    left_context: &str,
    utterance_start_ms: i64,
    projected_text_length: usize,
    backchannel_between: bool,
    config: &UtteranceReconstructionConfig,
) -> BoundaryOutcome {
    let evidence = evidence(
        left,
        right,
        left_context,
        utterance_start_ms,
        projected_text_length,
        backchannel_between,
        config.semantic_boundary_enabled,
        config.reliable_speaker_change_threshold,
    );
    let mut reasons = Vec::new();
    let mut components = BoundaryScoreComponents::default();

    if !evidence.timing_reliable {
        reasons.push(BoundaryReason::UnreliableTiming);
    }
    if evidence.mixed_attribution {
        reasons.push(BoundaryReason::MixedAttribution);
    }
    if evidence.overlap {
        reasons.push(if evidence.gap_ms.is_some_and(|gap| gap < 0) {
            BoundaryReason::OverlappingTimeline
        } else {
            BoundaryReason::Overlap
        });
    }
    if evidence.speaker_change_reliable {
        reasons.extend([
            BoundaryReason::SpeakerChanged,
            BoundaryReason::ReliableSpeakerChange,
        ]);
        return hard_split(
            evidence,
            reasons,
            components,
            BoundaryDecisionSource::ReliableSpeakerHandoff,
        );
    }
    if evidence
        .gap_ms
        .is_some_and(|gap| gap >= config.long_silence_ms)
    {
        reasons.push(BoundaryReason::LongSilence);
        return hard_split(
            evidence,
            reasons,
            components,
            BoundaryDecisionSource::HardSafetyConstraint,
        );
    }
    if evidence.timing_reliable && evidence.projected_duration_ms > config.max_utterance_duration_ms
    {
        reasons.push(BoundaryReason::MaximumDuration);
        return hard_split(
            evidence,
            reasons,
            components,
            BoundaryDecisionSource::HardSafetyConstraint,
        );
    }
    if projected_text_length > config.max_text_length {
        reasons.push(BoundaryReason::MaximumLength);
        return hard_split(
            evidence,
            reasons,
            components,
            BoundaryDecisionSource::HardSafetyConstraint,
        );
    }

    match evidence.same_speaker {
        Some(true) => {
            components.speaker_score += config.same_speaker_score;
            reasons.push(BoundaryReason::SameSpeaker);
        }
        Some(false) => {
            components.speaker_score += config.ambiguous_speaker_changed_score;
            reasons.extend([
                BoundaryReason::SpeakerChanged,
                BoundaryReason::AmbiguousSpeakerChange,
            ]);
        }
        None => reasons.push(BoundaryReason::SpeakerUnknown),
    }
    apply_timing_score(&evidence, config, &mut reasons, &mut components);

    // Raw features remain observable, but only one language-scoring path runs.
    if evidence.strong_terminal_punctuation {
        reasons.push(BoundaryReason::StrongTerminalPunctuation);
    }
    if evidence.weak_punctuation {
        reasons.push(BoundaryReason::WeakPunctuation);
    }
    if evidence.continuation_prefix {
        reasons.push(BoundaryReason::ContinuationPrefix);
    }
    if config.semantic_boundary_enabled {
        let semantic = evidence
            .semantic
            .as_ref()
            .expect("enabled semantic evidence");
        if semantic
            .left_completeness
            .is_some_and(|score| score >= config.completeness_split_threshold)
        {
            components.semantic_score += config.completeness_split_score;
            reasons.push(BoundaryReason::SentenceComplete);
        } else if semantic.left_completeness.is_some_and(|score| score < 0.5) {
            components.semantic_score += config.incomplete_merge_score;
            reasons.push(BoundaryReason::SentenceIncomplete);
        }
        if semantic
            .cross_boundary_continuity
            .is_some_and(|score| score >= config.continuity_merge_threshold)
        {
            components.semantic_score += config.continuity_merge_score;
            reasons.push(BoundaryReason::SemanticContinuity);
        }
    } else {
        apply_legacy_language_scores(&evidence, config, &mut components);
    }
    if backchannel_between {
        components.structural_score += config.backchannel_bridge_score;
        reasons.push(BoundaryReason::BackchannelBridge);
    }
    scored(evidence, reasons, components, config)
}

fn apply_legacy_scores(
    evidence: &BoundaryEvidence,
    backchannel_between: bool,
    config: &UtteranceReconstructionConfig,
    reasons: &mut Vec<BoundaryReason>,
    components: &mut BoundaryScoreComponents,
) {
    apply_timing_score(evidence, config, reasons, components);
    if evidence.strong_terminal_punctuation {
        reasons.push(BoundaryReason::StrongTerminalPunctuation);
    }
    if evidence.weak_punctuation {
        reasons.push(BoundaryReason::WeakPunctuation);
    }
    if evidence.continuation_prefix {
        reasons.push(BoundaryReason::ContinuationPrefix);
    }
    apply_legacy_language_scores(evidence, config, components);
    if backchannel_between {
        components.structural_score += config.backchannel_bridge_score;
        reasons.push(BoundaryReason::BackchannelBridge);
    }
}

fn apply_timing_score(
    evidence: &BoundaryEvidence,
    config: &UtteranceReconstructionConfig,
    reasons: &mut Vec<BoundaryReason>,
    components: &mut BoundaryScoreComponents,
) {
    if evidence
        .gap_ms
        .is_some_and(|gap| gap <= config.short_gap_ms)
    {
        components.timing_score += config.short_gap_score;
        reasons.push(BoundaryReason::ShortGap);
    } else if evidence
        .gap_ms
        .is_some_and(|gap| gap >= config.medium_gap_ms)
    {
        components.timing_score += config.medium_gap_score;
        reasons.push(BoundaryReason::MediumGap);
    }
}

fn apply_legacy_language_scores(
    evidence: &BoundaryEvidence,
    config: &UtteranceReconstructionConfig,
    components: &mut BoundaryScoreComponents,
) {
    if evidence.strong_terminal_punctuation {
        components.punctuation_score += config.terminal_punctuation_score;
    }
    if evidence.weak_punctuation {
        components.punctuation_score += config.weak_punctuation_score;
    }
    if evidence.continuation_prefix {
        components.semantic_score += config.continuation_prefix_score;
    }
}

fn assignment_is_trustworthy(span: &AtomicSpan, threshold: f64) -> bool {
    match span.speaker_attribution_source {
        SpeakerAttributionSource::Manual => true,
        SpeakerAttributionSource::LexicalTemporalOverlap
        | SpeakerAttributionSource::ChunkTemporalOverlap => span
            .speaker_assignment_reliability
            .is_some_and(|reliability| reliability >= threshold),
        SpeakerAttributionSource::PersistedFallback | SpeakerAttributionSource::Unknown => false,
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

fn scored(
    evidence: BoundaryEvidence,
    mut reasons: Vec<BoundaryReason>,
    score_components: BoundaryScoreComponents,
    config: &UtteranceReconstructionConfig,
) -> BoundaryOutcome {
    let score = score_components.timing_score
        + score_components.speaker_score
        + score_components.punctuation_score
        + score_components.semantic_score
        + score_components.structural_score;
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
        decision_source: BoundaryDecisionSource::ScoredDecision,
        reasons,
        score_components,
    }
}

fn hard_split(
    evidence: BoundaryEvidence,
    reasons: Vec<BoundaryReason>,
    score_components: BoundaryScoreComponents,
    decision_source: BoundaryDecisionSource,
) -> BoundaryOutcome {
    BoundaryOutcome {
        evidence,
        score: i32::MAX,
        decision: BoundaryDecision::Split,
        decision_source,
        reasons,
        score_components,
    }
}
