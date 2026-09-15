use std::collections::{BTreeMap, HashSet};

use sha2::{Digest, Sha256};

use crate::diarization::types::SegmentKind;

use super::boundary::decide_boundary;
use super::config::UtteranceReconstructionConfig;
use super::normalizer::join_text;
use super::types::{
    AtomicSpan, BoundaryDecision, BoundaryOutcome, BoundaryReason, ReconstructedEvent,
    ReconstructedUtterance, ReconstructionMetrics, ReconstructionResult, SourceLexicalRange,
    SpeakerAttribution, ALGORITHM_VERSION,
};

pub fn reconstruct(
    meeting_id: &str,
    spans: &[AtomicSpan],
    config: &UtteranceReconstructionConfig,
) -> ReconstructionResult {
    reconstruct_with_details(
        meeting_id,
        spans,
        config,
        ALGORITHM_VERSION,
        ReconstructionMetrics::default(),
        Vec::new(),
        Vec::new(),
    )
}

pub(crate) fn reconstruct_with_details(
    meeting_id: &str,
    spans: &[AtomicSpan],
    config: &UtteranceReconstructionConfig,
    algorithm_version: &str,
    metrics: ReconstructionMetrics,
    alignment_diagnostics: Vec<super::types::AlignmentDiagnostic>,
    timing_diagnostics: Vec<super::types::TimingDiagnostic>,
) -> ReconstructionResult {
    let bridges = find_backchannel_bridges(spans, config);
    let skipped = bridges.keys().copied().collect::<HashSet<_>>();
    let main_indices = (0..spans.len())
        .filter(|index| !skipped.contains(index))
        .collect::<Vec<_>>();
    let mut utterances = Vec::new();
    let mut boundaries = Vec::new();
    let mut current: Option<UtteranceBuilder> = None;

    for (position, index) in main_indices.iter().copied().enumerate() {
        let span = &spans[index];
        if current.is_none() {
            current = Some(UtteranceBuilder::new(span));
            continue;
        }
        let builder = current.as_ref().expect("builder exists");
        let previous_index = main_indices[position - 1];
        let previous = &spans[previous_index];
        let backchannel_between = bridges
            .values()
            .any(|bridge| bridge.left_index == previous_index && bridge.right_index == index);
        let projected_text = join_text(&builder.text, &span.text);
        let outcome = decide_boundary(
            previous,
            span,
            &builder.text,
            builder.start_ms,
            projected_text.chars().count(),
            backchannel_between,
            config,
        );
        trace_boundary(&outcome);
        boundaries.push(outcome.clone());
        if outcome.decision == BoundaryDecision::Merge {
            current
                .as_mut()
                .expect("builder exists")
                .merge(span, &outcome.reasons);
        } else {
            utterances.push(
                current
                    .take()
                    .expect("builder exists")
                    .finish(meeting_id, algorithm_version),
            );
            current = Some(UtteranceBuilder::new(span));
        }
    }
    if let Some(builder) = current {
        utterances.push(builder.finish(meeting_id, algorithm_version));
    }

    let mut unembedded_events = Vec::new();
    for (event_index, bridge) in bridges {
        let event = reconstructed_event(meeting_id, &spans[event_index], algorithm_version);
        if let Some(utterance) = utterances.iter_mut().find(|utterance| {
            utterance
                .source_transcript_ids
                .contains(&spans[bridge.left_index].source_transcript_ids[0])
                && utterance
                    .source_transcript_ids
                    .contains(&spans[bridge.right_index].source_transcript_ids[0])
        }) {
            utterance.embedded_events.push(event);
            utterance.embedded_events.sort_by(|left, right| {
                left.start_ms
                    .cmp(&right.start_ms)
                    .then(left.id.cmp(&right.id))
            });
        } else {
            unembedded_events.push(event);
        }
    }

    ReconstructionResult {
        meeting_id: meeting_id.to_string(),
        algorithm_version: algorithm_version.to_string(),
        utterances,
        events: unembedded_events,
        boundaries,
        config_version: super::config::CONFIG_VERSION.to_string(),
        config_hash: config_hash(config),
        config: config.clone(),
        metrics,
        alignment_diagnostics,
        timing_diagnostics,
    }
}

#[derive(Debug, Clone, Copy)]
struct BackchannelBridge {
    left_index: usize,
    right_index: usize,
}

fn find_backchannel_bridges(
    spans: &[AtomicSpan],
    config: &UtteranceReconstructionConfig,
) -> BTreeMap<usize, BackchannelBridge> {
    let mut bridges = BTreeMap::new();
    for index in 1..spans.len().saturating_sub(1) {
        let left = &spans[index - 1];
        let event = &spans[index];
        let right = &spans[index + 1];
        if event.segment_kind != SegmentKind::Backchannel
            || event.short_turn_confidence.unwrap_or_default() < config.backchannel_min_confidence
            || event.end_ms.saturating_sub(event.start_ms) > config.backchannel_max_duration_ms
            || !left.timing_reliable
            || !event.timing_reliable
            || !right.timing_reliable
            || left.overlap
            || event.overlap
            || right.overlap
            || left.speaker_attribution.is_mixed()
            || event.speaker_attribution.is_mixed()
            || right.speaker_attribution.is_mixed()
        {
            continue;
        }
        let (Some(left_speaker), Some(event_speaker), Some(right_speaker)) = (
            left.speaker_attribution.single_key(),
            event.speaker_attribution.single_key(),
            right.speaker_attribution.single_key(),
        ) else {
            continue;
        };
        let left_gap = event.start_ms - left.end_ms;
        let right_gap = right.start_ms - event.end_ms;
        if left_speaker == right_speaker
            && left_speaker != event_speaker
            && (0..=config.backchannel_max_side_gap_ms).contains(&left_gap)
            && (0..=config.backchannel_max_side_gap_ms).contains(&right_gap)
        {
            bridges.insert(
                index,
                BackchannelBridge {
                    left_index: index - 1,
                    right_index: index + 1,
                },
            );
        }
    }
    bridges
}

struct UtteranceBuilder {
    start_ms: i64,
    end_ms: i64,
    text: String,
    attribution: SpeakerAttribution,
    source_ids: Vec<String>,
    reasons: Vec<BoundaryReason>,
    overlap: bool,
    mixed: bool,
    confidence_sum: f64,
    confidence_count: usize,
    source_ranges: Vec<SourceLexicalRange>,
}

impl UtteranceBuilder {
    fn new(span: &AtomicSpan) -> Self {
        Self {
            start_ms: span.start_ms,
            end_ms: span.end_ms,
            text: span.text.clone(),
            attribution: span.speaker_attribution.clone(),
            source_ids: span.source_transcript_ids.clone(),
            reasons: Vec::new(),
            overlap: span.overlap,
            mixed: span.speaker_attribution.is_mixed(),
            confidence_sum: span.asr_confidence.unwrap_or_default(),
            confidence_count: usize::from(span.asr_confidence.is_some()),
            source_ranges: span.lexical_range.clone().into_iter().collect(),
        }
    }

    fn merge(&mut self, span: &AtomicSpan, reasons: &[BoundaryReason]) {
        self.end_ms = self.end_ms.max(span.end_ms);
        self.text = join_text(&self.text, &span.text);
        if self.attribution != span.speaker_attribution {
            self.attribution = SpeakerAttribution::Unknown;
        }
        self.source_ids.extend(span.source_transcript_ids.clone());
        self.reasons.extend(reasons.iter().cloned());
        self.overlap |= span.overlap;
        self.mixed |= span.speaker_attribution.is_mixed();
        if let Some(confidence) = span.asr_confidence {
            self.confidence_sum += confidence;
            self.confidence_count += 1;
        }
        self.source_ranges.extend(span.lexical_range.clone());
    }

    fn finish(self, meeting_id: &str, algorithm_version: &str) -> ReconstructedUtterance {
        ReconstructedUtterance {
            id: stable_id(
                "utterance",
                meeting_id,
                &self.source_ids,
                &self.source_ranges,
                algorithm_version,
            ),
            meeting_id: meeting_id.to_string(),
            start_ms: self.start_ms,
            end_ms: self.end_ms,
            speaker_attribution: self.attribution,
            text: self.text,
            source_transcript_ids: self.source_ids,
            mean_asr_confidence: (self.confidence_count > 0)
                .then(|| (self.confidence_sum / self.confidence_count as f64).clamp(0.0, 1.0)),
            reconstruction_reasons: self.reasons,
            overlap: self.overlap,
            mixed: self.mixed,
            embedded_events: Vec::new(),
            algorithm_version: algorithm_version.to_string(),
            source_ranges: self.source_ranges,
        }
    }
}

fn reconstructed_event(
    meeting_id: &str,
    span: &AtomicSpan,
    algorithm_version: &str,
) -> ReconstructedEvent {
    let source_ranges = span.lexical_range.clone().into_iter().collect::<Vec<_>>();
    ReconstructedEvent {
        id: stable_id(
            "event",
            meeting_id,
            &span.source_transcript_ids,
            &source_ranges,
            algorithm_version,
        ),
        meeting_id: meeting_id.to_string(),
        start_ms: span.start_ms,
        end_ms: span.end_ms,
        text: span.text.clone(),
        speaker_attribution: span.speaker_attribution.clone(),
        source_transcript_ids: span.source_transcript_ids.clone(),
        kind: span.segment_kind.clone(),
        confidence: span.short_turn_confidence.or(span.asr_confidence),
        overlap: span.overlap,
        algorithm_version: algorithm_version.to_string(),
        source_ranges,
    }
}

fn stable_id(
    prefix: &str,
    meeting_id: &str,
    source_ids: &[String],
    source_ranges: &[SourceLexicalRange],
    algorithm_version: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(algorithm_version.as_bytes());
    hasher.update([0]);
    hasher.update(meeting_id.as_bytes());
    for source_id in source_ids {
        hasher.update([0]);
        hasher.update(source_id.as_bytes());
    }
    for range in source_ranges {
        hasher.update([0]);
        hasher.update(range.source_transcript_id.as_bytes());
        hasher.update((range.lexical_start_index as u64).to_le_bytes());
        hasher.update((range.lexical_end_index as u64).to_le_bytes());
        hasher.update((range.token_start_index as u64).to_le_bytes());
        hasher.update((range.token_end_index as u64).to_le_bytes());
    }
    let digest = format!("{:x}", hasher.finalize());
    format!("{prefix}_{}", &digest[..24])
}

fn config_hash(config: &UtteranceReconstructionConfig) -> String {
    let encoded = serde_json::to_vec(config).expect("reconstruction config is serializable");
    format!("sha256:{:x}", Sha256::digest(encoded))
}

#[cfg(debug_assertions)]
fn trace_boundary(outcome: &BoundaryOutcome) {
    log::debug!(
        target: "utterance_reconstruction",
        "boundary left={} right={} gap_ms={:?} timing_reliable={} same_speaker={:?} speaker_confidence={:?} speaker_reliable={} completeness={:?} continuity={:?} timing_score={} speaker_score={} punctuation_score={} semantic_score={} structural_score={} final_score={} decision={:?} reasons={:?}",
        outcome.evidence.left_source_transcript_id,
        outcome.evidence.right_source_transcript_id,
        outcome.evidence.gap_ms,
        outcome.evidence.timing_reliable,
        outcome.evidence.same_speaker,
        outcome.evidence.speaker_change_confidence,
        outcome.evidence.speaker_change_reliable,
        outcome.evidence.semantic.left_completeness,
        outcome.evidence.semantic.cross_boundary_continuity,
        outcome.score_components.timing_score,
        outcome.score_components.speaker_score,
        outcome.score_components.punctuation_score,
        outcome.score_components.semantic_score,
        outcome.score_components.structural_score,
        outcome.score,
        outcome.decision,
        outcome.reasons
    );
}

#[cfg(not(debug_assertions))]
fn trace_boundary(_: &BoundaryOutcome) {}
