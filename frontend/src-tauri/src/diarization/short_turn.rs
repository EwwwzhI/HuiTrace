//! Explainable, model-free refinement for short offline transcript turns.
//!
//! Phase 2A intentionally treats text as one signal among several. The module
//! can only select a meeting-local speaker already known to the prototype
//! store; it never creates a speaker identity.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::types::{
    AssignmentMethod, AudioSource, SegmentKind, SpeakerSegment, TranscriptSpeakerAssignment,
    TranscriptTiming,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ShortTurnConfig {
    /// Engineering initial values, kept together so evaluation can tune them.
    pub min_candidate_ms: u64,
    pub very_short_ms: u64,
    pub max_short_turn_ms: u64,
    pub prototype_min_duration_ms: u64,
    pub duration_weight_20_until_ms: u64,
    pub duration_weight_35_until_ms: u64,
    pub duration_weight_55_until_ms: u64,
    pub high_confidence_threshold: f64,
    pub weak_confidence_threshold: f64,
    pub nearby_gap_ms: u64,
    pub lexical_backchannel_enabled: bool,
}

impl Default for ShortTurnConfig {
    fn default() -> Self {
        Self {
            min_candidate_ms: 100,
            very_short_ms: 500,
            max_short_turn_ms: 1_200,
            prototype_min_duration_ms: 1_500,
            duration_weight_20_until_ms: 299,
            duration_weight_35_until_ms: 499,
            duration_weight_55_until_ms: 799,
            high_confidence_threshold: 0.75,
            weak_confidence_threshold: 0.40,
            nearby_gap_ms: 350,
            lexical_backchannel_enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortTurnCandidate {
    pub start_ms: i64,
    pub end_ms: i64,
    pub duration_ms: u64,
    pub text: String,
    pub asr_confidence: Option<f64>,
    pub diarization_speaker: Option<String>,
    pub diarization_confidence: Option<f64>,
    pub audio_source: AudioSource,
    pub overlaps_existing_turn: bool,
    pub true_speaker_overlap: bool,
    pub previous_speaker: Option<String>,
    pub next_speaker: Option<String>,
    pub previous_gap_ms: Option<u64>,
    pub next_gap_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortTurnEvidence {
    pub normalized_text: String,
    pub lexical_backchannel: bool,
    pub base_speaker_confidence: f64,
    pub duration_weight: f64,
    pub effective_speaker_confidence: f64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortTurnDecision {
    pub kind: SegmentKind,
    pub speaker_key: Option<String>,
    pub confidence: f64,
    pub evidence: ShortTurnEvidence,
    pub allow_new_speaker: bool,
}

pub trait BackchannelDetector: Send + Sync {
    fn normalize(&self, text: &str) -> String;
    fn is_backchannel(&self, text: &str) -> bool;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LexicalBackchannelDetector;

impl LexicalBackchannelDetector {
    fn collapse_repeats(text: &str) -> String {
        let mut result = String::new();
        let mut previous = None;
        let mut count = 0usize;
        for ch in text.chars() {
            if previous == Some(ch) {
                count += 1;
                if count <= 2 {
                    result.push(ch);
                }
            } else {
                previous = Some(ch);
                count = 1;
                result.push(ch);
            }
        }
        result
    }
}

impl BackchannelDetector for LexicalBackchannelDetector {
    fn normalize(&self, text: &str) -> String {
        let lowered = text.trim().to_lowercase();
        let without_punctuation: String = lowered
            .chars()
            .map(|ch| {
                if ch.is_alphanumeric() || ch.is_whitespace() {
                    ch
                } else {
                    ' '
                }
            })
            .collect();
        let compact = without_punctuation.split_whitespace().collect::<String>();
        Self::collapse_repeats(&compact)
    }

    fn is_backchannel(&self, text: &str) -> bool {
        matches!(
            self.normalize(text).as_str(),
            "嗯" | "嗯嗯"
                | "啊"
                | "哦"
                | "噢"
                | "对"
                | "是"
                | "好"
                | "好的"
                | "行"
                | "可以"
                | "没错"
                | "对的"
                | "uh"
                | "um"
                | "hmm"
                | "hm"
                | "yeah"
                | "yes"
                | "yep"
                | "right"
                | "okay"
                | "ok"
                | "sure"
                | "oh"
        )
    }
}

pub trait SpeakerPrototypeStore: Send + Sync {
    fn contains(&self, speaker_key: &str) -> bool;

    fn match_existing_speaker(&self, candidate: &ShortTurnCandidate) -> Option<String> {
        candidate
            .diarization_speaker
            .as_deref()
            .filter(|speaker| self.contains(speaker))
            .map(ToOwned::to_owned)
    }

    fn can_update_prototype(
        &self,
        duration_ms: u64,
        confidence: f64,
        overlap: bool,
        kind: &SegmentKind,
        config: &ShortTurnConfig,
    ) -> bool {
        duration_ms >= config.prototype_min_duration_ms
            && confidence >= config.high_confidence_threshold
            && !overlap
            && *kind == SegmentKind::Speech
    }
}

#[derive(Debug, Clone, Default)]
pub struct MeetingSpeakerPrototypeStore {
    speakers: HashSet<String>,
}

impl MeetingSpeakerPrototypeStore {
    pub fn new(speakers: impl IntoIterator<Item = String>) -> Self {
        Self {
            speakers: speakers.into_iter().collect(),
        }
    }
}

impl SpeakerPrototypeStore for MeetingSpeakerPrototypeStore {
    fn contains(&self, speaker_key: &str) -> bool {
        self.speakers.contains(speaker_key)
    }
}

pub struct ShortTurnRefiner<D = LexicalBackchannelDetector> {
    pub config: ShortTurnConfig,
    detector: D,
}

impl Default for ShortTurnRefiner<LexicalBackchannelDetector> {
    fn default() -> Self {
        Self::new(ShortTurnConfig::default(), LexicalBackchannelDetector)
    }
}

impl<D: BackchannelDetector> ShortTurnRefiner<D> {
    pub fn new(config: ShortTurnConfig, detector: D) -> Self {
        Self { config, detector }
    }

    pub fn should_refine(&self, duration_ms: u64) -> bool {
        duration_ms <= self.config.max_short_turn_ms
    }

    pub fn refine(
        &self,
        candidate: &ShortTurnCandidate,
        prototypes: &dyn SpeakerPrototypeStore,
    ) -> ShortTurnDecision {
        let normalized_text = self.detector.normalize(&candidate.text);
        let lexical = self.config.lexical_backchannel_enabled
            && self.detector.is_backchannel(&candidate.text);
        let base = candidate
            .diarization_confidence
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        let weight = duration_weight_with_config(candidate.duration_ms, &self.config);
        let effective = base * weight;
        let asr = candidate
            .asr_confidence
            .unwrap_or(if normalized_text.is_empty() { 0.0 } else { 0.5 });

        let kind =
            if candidate.duration_ms < self.config.min_candidate_ms && normalized_text.is_empty() {
                SegmentKind::Noise
            } else if lexical {
                SegmentKind::Backchannel
            } else if normalized_text.is_empty()
                && candidate.duration_ms <= self.config.very_short_ms
                && base < self.config.weak_confidence_threshold
            {
                SegmentKind::Noise
            } else if normalized_text.is_empty() {
                SegmentKind::Unknown
            } else {
                SegmentKind::Speech
            };

        // Direct diarizer coverage wins over neighbours when it is credible.
        // Duration reduces confidence, but lexical/ASR evidence may make a very
        // short direct assignment usable without pretending it is a prototype.
        let direct_score =
            (effective + if lexical { 0.35 } else { 0.0 } + asr * 0.15).clamp(0.0, 1.0);
        let mut speaker = if base >= self.config.high_confidence_threshold
            || direct_score >= self.config.weak_confidence_threshold
        {
            prototypes.match_existing_speaker(candidate)
        } else {
            None
        };
        let mut reason = if speaker.is_some() {
            "credible direct diarization coverage".to_string()
        } else {
            "insufficient direct speaker evidence".to_string()
        };

        // Continuity is deliberately soft evidence and is disabled during real
        // overlap. It can only select an already known speaker.
        if speaker.is_none()
            && lexical
            && !candidate.true_speaker_overlap
            && candidate.previous_speaker == candidate.next_speaker
        {
            if let Some(neighbour) = candidate.previous_speaker.as_deref() {
                let close = candidate.previous_gap_ms.unwrap_or(u64::MAX)
                    <= self.config.nearby_gap_ms
                    && candidate.next_gap_ms.unwrap_or(u64::MAX) <= self.config.nearby_gap_ms;
                if close && prototypes.contains(neighbour) {
                    speaker = Some(neighbour.to_string());
                    reason = "matching nearby speakers supplied soft continuity evidence".into();
                }
            }
        }

        if matches!(kind, SegmentKind::Noise | SegmentKind::Unknown) && !lexical {
            speaker = None;
            reason = "empty or weak non-lexical short event".into();
        }

        let confidence = if speaker.is_some() {
            direct_score.max(if lexical { 0.45 } else { effective })
        } else if kind == SegmentKind::Noise {
            (1.0 - base).clamp(0.0, 1.0)
        } else {
            direct_score
        };
        let decision = ShortTurnDecision {
            kind,
            speaker_key: speaker,
            confidence,
            evidence: ShortTurnEvidence {
                normalized_text,
                lexical_backchannel: lexical,
                base_speaker_confidence: base,
                duration_weight: weight,
                effective_speaker_confidence: effective,
                reason,
            },
            allow_new_speaker: false,
        };
        log::debug!(
            "short_turn duration={}ms text={:?} base={:?} base_conf={:.2} duration_weight={:.2} lexical_backchannel={} decision={} speaker={:?} final_conf={:.2} reason={}",
            candidate.duration_ms,
            decision.evidence.normalized_text,
            candidate.diarization_speaker,
            base,
            weight,
            lexical,
            decision.kind.as_str(),
            decision.speaker_key,
            decision.confidence,
            decision.evidence.reason
        );
        decision
    }
}

/// Piecewise engineering baseline. Boundary values belong to the higher bucket.
pub fn duration_weight(duration_ms: u64) -> f64 {
    duration_weight_with_config(duration_ms, &ShortTurnConfig::default())
}

pub fn duration_weight_with_config(duration_ms: u64, config: &ShortTurnConfig) -> f64 {
    if duration_ms <= config.duration_weight_20_until_ms {
        0.20
    } else if duration_ms <= config.duration_weight_35_until_ms {
        0.35
    } else if duration_ms <= config.duration_weight_55_until_ms {
        0.55
    } else if duration_ms <= config.max_short_turn_ms {
        0.75
    } else {
        1.0
    }
}

pub fn min_candidate_samples_16khz(config: &ShortTurnConfig) -> usize {
    (config.min_candidate_ms as usize * 16_000) / 1_000
}

pub fn refine_timeline_assignment(
    refiner: &ShortTurnRefiner,
    prototypes: &dyn SpeakerPrototypeStore,
    timing: &TranscriptTiming,
    text: &str,
    asr_confidence: Option<f64>,
    assignment: TranscriptSpeakerAssignment,
    speakers: &[SpeakerSegment],
) -> TranscriptSpeakerAssignment {
    let duration_ms = timing.end_ms.saturating_sub(timing.start_ms) as u64;
    if !refiner.should_refine(duration_ms)
        || assignment.assignment_method == AssignmentMethod::Manual
    {
        return assignment;
    }

    let overlapping: Vec<&SpeakerSegment> = speakers
        .iter()
        .filter(|speaker| speaker.start_ms < timing.end_ms && speaker.end_ms > timing.start_ms)
        .collect();
    let previous = speakers
        .iter()
        .filter(|speaker| speaker.end_ms <= timing.start_ms)
        .max_by_key(|speaker| speaker.end_ms);
    let next = speakers
        .iter()
        .filter(|speaker| speaker.start_ms >= timing.end_ms)
        .min_by_key(|speaker| speaker.start_ms);
    let candidate = ShortTurnCandidate {
        start_ms: timing.start_ms,
        end_ms: timing.end_ms,
        duration_ms,
        text: text.to_string(),
        asr_confidence,
        diarization_speaker: assignment.speaker_key.clone(),
        diarization_confidence: assignment.speaker_confidence,
        audio_source: timing.audio_source.clone(),
        overlaps_existing_turn: !overlapping.is_empty(),
        true_speaker_overlap: overlapping
            .iter()
            .map(|speaker| &speaker.speaker_key)
            .collect::<HashSet<_>>()
            .len()
            > 1,
        previous_speaker: previous.map(|speaker| speaker.speaker_key.clone()),
        next_speaker: next.map(|speaker| speaker.speaker_key.clone()),
        previous_gap_ms: previous
            .map(|speaker| timing.start_ms.saturating_sub(speaker.end_ms) as u64),
        next_gap_ms: next.map(|speaker| speaker.start_ms.saturating_sub(timing.end_ms) as u64),
    };
    let decision = refiner.refine(&candidate, prototypes);
    TranscriptSpeakerAssignment {
        transcript_id: assignment.transcript_id,
        speaker_confidence: decision.speaker_key.as_ref().map(|_| decision.confidence),
        speaker_key: decision.speaker_key,
        speaker_provisional: false,
        speaker_revision: assignment.speaker_revision.saturating_add(1),
        segment_kind: decision.kind,
        audio_source: assignment.audio_source,
        assignment_method: AssignmentMethod::ShortTurnRefinement,
        overlap: assignment.overlap,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ShortTurnMetrics {
    pub short_speech_recall: f64,
    pub noise_false_accept_rate: f64,
    pub backchannel_recall: f64,
    pub backchannel_precision: f64,
    pub speaker_attribution_accuracy: f64,
    pub unknown_rate: f64,
    pub false_new_speaker_rate: f64,
    pub manual_override_violation_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShortTurnEvaluationObservation {
    pub expected_kind: SegmentKind,
    pub expected_speaker: Option<String>,
    pub predicted_kind: SegmentKind,
    pub predicted_speaker: Option<String>,
    pub predicted_is_new_speaker: bool,
    pub manual_override_violated: bool,
}

pub fn compute_metrics(observations: &[ShortTurnEvaluationObservation]) -> ShortTurnMetrics {
    fn ratio(numerator: usize, denominator: usize) -> f64 {
        if denominator == 0 {
            0.0
        } else {
            numerator as f64 / denominator as f64
        }
    }

    let expected_speech = observations
        .iter()
        .filter(|item| {
            matches!(
                item.expected_kind,
                SegmentKind::Speech | SegmentKind::Backchannel
            )
        })
        .count();
    let recalled_speech = observations
        .iter()
        .filter(|item| {
            matches!(
                item.expected_kind,
                SegmentKind::Speech | SegmentKind::Backchannel
            ) && matches!(
                item.predicted_kind,
                SegmentKind::Speech | SegmentKind::Backchannel
            )
        })
        .count();
    let expected_noise = observations
        .iter()
        .filter(|item| item.expected_kind == SegmentKind::Noise)
        .count();
    let accepted_noise = observations
        .iter()
        .filter(|item| {
            item.expected_kind == SegmentKind::Noise
                && (matches!(
                    item.predicted_kind,
                    SegmentKind::Speech | SegmentKind::Backchannel
                ) || item.predicted_speaker.is_some())
        })
        .count();
    let expected_backchannels = observations
        .iter()
        .filter(|item| item.expected_kind == SegmentKind::Backchannel)
        .count();
    let predicted_backchannels = observations
        .iter()
        .filter(|item| item.predicted_kind == SegmentKind::Backchannel)
        .count();
    let correct_backchannels = observations
        .iter()
        .filter(|item| {
            item.expected_kind == SegmentKind::Backchannel
                && item.predicted_kind == SegmentKind::Backchannel
        })
        .count();
    let attributed = observations
        .iter()
        .filter(|item| item.expected_speaker.is_some())
        .count();
    let correct_attribution = observations
        .iter()
        .filter(|item| {
            item.expected_speaker.is_some() && item.expected_speaker == item.predicted_speaker
        })
        .count();

    ShortTurnMetrics {
        short_speech_recall: ratio(recalled_speech, expected_speech),
        noise_false_accept_rate: ratio(accepted_noise, expected_noise),
        backchannel_recall: ratio(correct_backchannels, expected_backchannels),
        backchannel_precision: ratio(correct_backchannels, predicted_backchannels),
        speaker_attribution_accuracy: ratio(correct_attribution, attributed),
        unknown_rate: ratio(
            observations
                .iter()
                .filter(|item| item.predicted_kind == SegmentKind::Unknown)
                .count(),
            observations.len(),
        ),
        false_new_speaker_rate: ratio(
            observations
                .iter()
                .filter(|item| item.predicted_is_new_speaker)
                .count(),
            observations.len(),
        ),
        manual_override_violation_count: observations
            .iter()
            .filter(|item| item.manual_override_violated)
            .count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        duration_ms: u64,
        text: &str,
        speaker: Option<&str>,
        confidence: f64,
    ) -> ShortTurnCandidate {
        ShortTurnCandidate {
            start_ms: 0,
            end_ms: duration_ms as i64,
            duration_ms,
            text: text.into(),
            asr_confidence: Some(0.9),
            diarization_speaker: speaker.map(str::to_string),
            diarization_confidence: Some(confidence),
            audio_source: AudioSource::Mixed,
            overlaps_existing_turn: speaker.is_some(),
            true_speaker_overlap: false,
            previous_speaker: None,
            next_speaker: None,
            previous_gap_ms: None,
            next_gap_ms: None,
        }
    }

    #[test]
    fn duration_weight_boundaries_are_explicit() {
        assert_eq!(duration_weight(299), 0.20);
        assert_eq!(duration_weight(300), 0.35);
        assert_eq!(duration_weight(499), 0.35);
        assert_eq!(duration_weight(500), 0.55);
        assert_eq!(duration_weight(799), 0.55);
        assert_eq!(duration_weight(800), 0.75);
        assert_eq!(duration_weight(1_200), 0.75);
        assert_eq!(duration_weight(1_201), 1.0);
    }

    #[test]
    fn lexical_detector_matches_whole_normalized_utterances_only() {
        let detector = LexicalBackchannelDetector;
        for text in ["嗯。", "嗯嗯", "嗯～", "hmm...", "yeah!", "OH"] {
            assert!(detector.is_backchannel(text), "expected {text:?}");
        }
        assert!(!detector.is_backchannel("对这个问题我们下一步……"));
        assert!(!detector.is_backchannel("oh that is interesting"));
    }

    #[test]
    fn short_turn_never_allows_a_new_speaker() {
        let refiner = ShortTurnRefiner::default();
        let prototypes = MeetingSpeakerPrototypeStore::default();
        let decision = refiner.refine(&candidate(200, "嗯", Some("speaker_99"), 0.95), &prototypes);
        assert!(!decision.allow_new_speaker);
        assert_eq!(decision.speaker_key, None);
    }

    #[test]
    fn direct_credible_evidence_beats_neighbour_continuity() {
        let refiner = ShortTurnRefiner::default();
        let prototypes =
            MeetingSpeakerPrototypeStore::new(["speaker_01".into(), "speaker_02".into()]);
        let mut value = candidate(300, "hmm", Some("speaker_02"), 0.90);
        value.previous_speaker = Some("speaker_01".into());
        value.next_speaker = Some("speaker_01".into());
        value.previous_gap_ms = Some(0);
        value.next_gap_ms = Some(0);
        let decision = refiner.refine(&value, &prototypes);
        assert_eq!(decision.kind, SegmentKind::Backchannel);
        assert_eq!(decision.speaker_key.as_deref(), Some("speaker_02"));
    }

    #[test]
    fn empty_weak_short_event_is_noise_without_a_speaker() {
        let refiner = ShortTurnRefiner::default();
        let prototypes = MeetingSpeakerPrototypeStore::new(["speaker_01".into()]);
        let decision = refiner.refine(&candidate(180, "", None, 0.05), &prototypes);
        assert_eq!(decision.kind, SegmentKind::Noise);
        assert_eq!(decision.speaker_key, None);
    }

    #[test]
    fn prototype_updates_require_long_clean_confident_speech() {
        let store = MeetingSpeakerPrototypeStore::default();
        let config = ShortTurnConfig::default();
        assert!(store.can_update_prototype(1_500, 0.9, false, &SegmentKind::Speech, &config));
        assert!(!store.can_update_prototype(1_200, 0.9, false, &SegmentKind::Speech, &config));
        assert!(!store.can_update_prototype(2_000, 0.9, false, &SegmentKind::Backchannel, &config));
    }

    #[test]
    fn metrics_include_zero_false_new_speaker_and_manual_violations() {
        let metrics = compute_metrics(&[ShortTurnEvaluationObservation {
            expected_kind: SegmentKind::Backchannel,
            expected_speaker: Some("speaker_02".into()),
            predicted_kind: SegmentKind::Backchannel,
            predicted_speaker: Some("speaker_02".into()),
            predicted_is_new_speaker: false,
            manual_override_violated: false,
        }]);
        assert_eq!(metrics.backchannel_recall, 1.0);
        assert_eq!(metrics.backchannel_precision, 1.0);
        assert_eq!(metrics.speaker_attribution_accuracy, 1.0);
        assert_eq!(metrics.false_new_speaker_rate, 0.0);
        assert_eq!(metrics.manual_override_violation_count, 0);
    }

    #[test]
    fn manual_assignment_is_never_refined() {
        let timing = TranscriptTiming {
            id: "manual".into(),
            start_ms: 0,
            end_ms: 200,
            audio_source: AudioSource::Mixed,
        };
        let assignment = TranscriptSpeakerAssignment {
            transcript_id: "manual".into(),
            speaker_key: Some("speaker_01".into()),
            speaker_confidence: Some(1.0),
            speaker_provisional: false,
            speaker_revision: 7,
            segment_kind: SegmentKind::Speech,
            audio_source: AudioSource::Mixed,
            assignment_method: AssignmentMethod::Manual,
            overlap: false,
        };
        let result = refine_timeline_assignment(
            &ShortTurnRefiner::default(),
            &MeetingSpeakerPrototypeStore::new(["speaker_01".into()]),
            &timing,
            "hmm",
            Some(0.9),
            assignment.clone(),
            &[],
        );
        assert_eq!(result, assignment);
    }
}
