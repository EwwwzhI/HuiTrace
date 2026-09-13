//! Explainable, model-free refinement for short offline transcript turns.
//!
//! Phase 2A intentionally treats text as one signal among several. The module
//! can only select a meeting-local speaker already known to the prototype
//! store; it never creates a speaker identity.

use std::collections::{BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use super::types::{
    AssignmentMethod, AudioSource, SegmentKind, SpeakerSegment, TranscriptSpeakerAssignment,
    TranscriptTiming,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortTurnCandidateSource {
    Transcript,
    DiarizerTurn,
    VadEvent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShortCandidateVadConfig {
    /// Engineering baseline; benchmark results, not model assumptions, tune it.
    pub min_speech_ms: u64,
    pub redemption_ms: u32,
    pub max_candidate_ms: u64,
}

impl Default for ShortCandidateVadConfig {
    fn default() -> Self {
        Self {
            min_speech_ms: 100,
            redemption_ms: 250,
            max_candidate_ms: 1_200,
        }
    }
}

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

#[derive(Debug, Clone, PartialEq)]
pub struct SpeakerAcceptanceTurn {
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker_key: String,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SpeakerAcceptanceEvidence {
    pub total_duration_ms: u64,
    pub turn_count: usize,
    pub longest_turn_ms: u64,
    pub available_confidence_count: usize,
    pub mean_available_confidence: Option<f64>,
    pub overlap_ratio: f64,
}

/// Pure production policy shared by persistence and the benchmark. Unknown
/// confidence is never promoted to perfect confidence.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeakerAcceptancePolicy {
    pub config: ShortTurnConfig,
    pub max_overlap_ratio: f64,
    pub confident_min_longest_turn_ms: u64,
    pub missing_confidence_min_total_ms: u64,
    pub missing_confidence_min_turns: usize,
}

impl Default for SpeakerAcceptancePolicy {
    fn default() -> Self {
        Self {
            config: ShortTurnConfig::default(),
            max_overlap_ratio: 0.50,
            confident_min_longest_turn_ms: 1_200,
            missing_confidence_min_total_ms: 4_000,
            missing_confidence_min_turns: 2,
        }
    }
}

impl SpeakerAcceptancePolicy {
    pub fn evidence_for(
        &self,
        turns: &[SpeakerAcceptanceTurn],
        speaker_key: &str,
    ) -> SpeakerAcceptanceEvidence {
        let owned: Vec<_> = turns
            .iter()
            .filter(|turn| turn.speaker_key == speaker_key)
            .collect();
        let total_duration_ms = owned
            .iter()
            .map(|turn| turn.end_ms.saturating_sub(turn.start_ms) as u64)
            .sum();
        let longest_turn_ms = owned
            .iter()
            .map(|turn| turn.end_ms.saturating_sub(turn.start_ms) as u64)
            .max()
            .unwrap_or(0);
        let confidences: Vec<_> = owned.iter().filter_map(|turn| turn.confidence).collect();
        let mean_available_confidence = (!confidences.is_empty())
            .then(|| confidences.iter().sum::<f64>() / confidences.len() as f64);
        let overlap_ms: u64 = owned
            .iter()
            .map(|turn| {
                turns
                    .iter()
                    .filter(|other| other.speaker_key != speaker_key)
                    .map(|other| {
                        overlap_ms(turn.start_ms, turn.end_ms, other.start_ms, other.end_ms) as u64
                    })
                    .sum::<u64>()
                    .min(turn.end_ms.saturating_sub(turn.start_ms) as u64)
            })
            .sum();
        SpeakerAcceptanceEvidence {
            total_duration_ms,
            turn_count: owned.len(),
            longest_turn_ms,
            available_confidence_count: confidences.len(),
            mean_available_confidence,
            overlap_ratio: if total_duration_ms == 0 {
                0.0
            } else {
                overlap_ms as f64 / total_duration_ms as f64
            },
        }
    }

    pub fn qualifies(&self, evidence: &SpeakerAcceptanceEvidence) -> bool {
        if evidence.overlap_ratio > self.max_overlap_ratio {
            return false;
        }
        match evidence.mean_available_confidence {
            Some(confidence) => {
                evidence.total_duration_ms >= self.config.prototype_min_duration_ms
                    && evidence.longest_turn_ms >= self.confident_min_longest_turn_ms
                    && confidence >= self.config.high_confidence_threshold
            }
            None => {
                evidence.total_duration_ms >= self.missing_confidence_min_total_ms
                    && evidence.turn_count >= self.missing_confidence_min_turns
                    && evidence.longest_turn_ms >= self.config.prototype_min_duration_ms
            }
        }
    }

    pub fn accepted_speaker_keys(&self, turns: &[SpeakerAcceptanceTurn]) -> BTreeSet<String> {
        let keys: BTreeSet<_> = turns.iter().map(|turn| turn.speaker_key.clone()).collect();
        keys.into_iter()
            .filter(|key| self.qualifies(&self.evidence_for(turns, key)))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortTurnCandidate {
    pub start_ms: i64,
    pub end_ms: i64,
    pub duration_ms: u64,
    #[serde(default)]
    pub candidate_sources: Vec<ShortTurnCandidateSource>,
    #[serde(default)]
    pub transcript_ids: Vec<String>,
    pub text: String,
    pub asr_confidence: Option<f64>,
    pub diarization_speaker: Option<String>,
    pub diarization_confidence: Option<f64>,
    #[serde(default)]
    pub diarization_coverage_ratio: Option<f64>,
    #[serde(default)]
    pub vad_confidence: Option<f64>,
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
    pub asr_confidence_present: bool,
    pub diarization_confidence_present: bool,
    pub base_speaker_confidence: f64,
    pub duration_weight: f64,
    pub effective_speaker_confidence: f64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortTurnDecision {
    pub kind: SegmentKind,
    pub kind_confidence: f64,
    pub speaker_key: Option<String>,
    pub speaker_confidence: Option<f64>,
    pub evidence: ShortTurnEvidence,
    pub allow_new_speaker: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptCandidateInput {
    pub timing: TranscriptTiming,
    pub text: String,
    pub asr_confidence: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VadEventCandidateInput {
    pub start_ms: i64,
    pub end_ms: i64,
    /// Only set when the VAD exposes a genuine score. Estimated constants stay `None`.
    pub confidence: Option<f64>,
    pub audio_source: AudioSource,
}

#[derive(Debug, Clone)]
pub struct ShortTurnCandidateExtractor {
    pub config: ShortTurnConfig,
}

impl Default for ShortTurnCandidateExtractor {
    fn default() -> Self {
        Self {
            config: ShortTurnConfig::default(),
        }
    }
}

impl ShortTurnCandidateExtractor {
    pub fn extract(
        &self,
        transcripts: &[TranscriptCandidateInput],
        speakers: &[SpeakerSegment],
        vad_events: &[VadEventCandidateInput],
    ) -> Vec<ShortTurnCandidate> {
        let mut seeds = Vec::new();
        for transcript in transcripts {
            let duration = transcript
                .timing
                .end_ms
                .saturating_sub(transcript.timing.start_ms) as u64;
            if self.in_candidate_range(duration) {
                seeds.push(self.candidate_for_window(
                    transcript.timing.start_ms,
                    transcript.timing.end_ms,
                    vec![ShortTurnCandidateSource::Transcript],
                    vec![transcript.timing.id.clone()],
                    transcript.text.clone(),
                    transcript.asr_confidence,
                    None,
                    transcript.timing.audio_source.clone(),
                    speakers,
                ));
            }
        }
        for speaker in speakers {
            let duration = speaker.end_ms.saturating_sub(speaker.start_ms) as u64;
            if self.in_candidate_range(duration) {
                let transcript_ids = transcripts
                    .iter()
                    .filter(|t| {
                        windows_intersect(
                            speaker.start_ms,
                            speaker.end_ms,
                            t.timing.start_ms,
                            t.timing.end_ms,
                        )
                    })
                    .map(|t| t.timing.id.clone())
                    .collect();
                let mut candidate = self.candidate_for_window(
                    speaker.start_ms,
                    speaker.end_ms,
                    vec![ShortTurnCandidateSource::DiarizerTurn],
                    transcript_ids,
                    String::new(),
                    None,
                    None,
                    speaker.audio_source.clone(),
                    speakers,
                );
                // A diarizer-turn seed is direct evidence about that exact
                // turn. Do not let a longer enclosing turn win an equal-overlap
                // tie and erase the embedded speaker before refinement.
                candidate.diarization_speaker = Some(speaker.speaker_key.clone());
                candidate.diarization_confidence = speaker.speaker_confidence;
                candidate.diarization_coverage_ratio = Some(1.0);
                seeds.push(candidate);
            }
        }
        for event in vad_events {
            let duration = event.end_ms.saturating_sub(event.start_ms) as u64;
            if self.in_candidate_range(duration) {
                let transcript_ids = transcripts
                    .iter()
                    .filter(|t| {
                        windows_intersect(
                            event.start_ms,
                            event.end_ms,
                            t.timing.start_ms,
                            t.timing.end_ms,
                        )
                    })
                    .map(|t| t.timing.id.clone())
                    .collect();
                seeds.push(self.candidate_for_window(
                    event.start_ms,
                    event.end_ms,
                    vec![ShortTurnCandidateSource::VadEvent],
                    transcript_ids,
                    String::new(),
                    None,
                    event.confidence,
                    event.audio_source.clone(),
                    speakers,
                ));
            }
        }
        merge_candidates(seeds)
    }

    fn in_candidate_range(&self, duration_ms: u64) -> bool {
        duration_ms >= self.config.min_candidate_ms && duration_ms <= self.config.max_short_turn_ms
    }

    #[allow(clippy::too_many_arguments)]
    fn candidate_for_window(
        &self,
        start_ms: i64,
        end_ms: i64,
        sources: Vec<ShortTurnCandidateSource>,
        transcript_ids: Vec<String>,
        text: String,
        asr_confidence: Option<f64>,
        vad_confidence: Option<f64>,
        audio_source: AudioSource,
        speakers: &[SpeakerSegment],
    ) -> ShortTurnCandidate {
        let timing = TranscriptTiming {
            id: transcript_ids.first().cloned().unwrap_or_default(),
            start_ms,
            end_ms,
            audio_source: audio_source.clone(),
        };
        let overlapping: Vec<&SpeakerSegment> = speakers
            .iter()
            .filter(|s| windows_intersect(start_ms, end_ms, s.start_ms, s.end_ms))
            .collect();
        let dominant = overlapping.iter().copied().max_by(|left, right| {
            overlap_ms(start_ms, end_ms, left.start_ms, left.end_ms)
                .cmp(&overlap_ms(start_ms, end_ms, right.start_ms, right.end_ms))
                .then_with(|| left.speaker_key.cmp(&right.speaker_key).reverse())
        });
        let duration_ms = end_ms.saturating_sub(start_ms).max(1) as u64;
        let coverage = dominant.map(|speaker| {
            overlap_ms(start_ms, end_ms, speaker.start_ms, speaker.end_ms) as f64
                / duration_ms as f64
        });
        let previous = speakers
            .iter()
            .filter(|speaker| speaker.end_ms <= start_ms)
            .max_by_key(|speaker| speaker.end_ms);
        let next = speakers
            .iter()
            .filter(|speaker| speaker.start_ms >= end_ms)
            .min_by_key(|speaker| speaker.start_ms);
        ShortTurnCandidate {
            start_ms,
            end_ms,
            duration_ms,
            candidate_sources: sources,
            transcript_ids,
            text,
            asr_confidence,
            diarization_speaker: dominant.map(|speaker| speaker.speaker_key.clone()),
            diarization_confidence: dominant.and_then(|speaker| speaker.speaker_confidence),
            diarization_coverage_ratio: coverage,
            vad_confidence,
            audio_source,
            overlaps_existing_turn: !overlapping.is_empty(),
            true_speaker_overlap: super::timeline::has_true_speaker_overlap(&timing, speakers),
            previous_speaker: previous.map(|speaker| speaker.speaker_key.clone()),
            next_speaker: next.map(|speaker| speaker.speaker_key.clone()),
            previous_gap_ms: previous.map(|speaker| start_ms.saturating_sub(speaker.end_ms) as u64),
            next_gap_ms: next.map(|speaker| speaker.start_ms.saturating_sub(end_ms) as u64),
        }
    }
}

fn windows_intersect(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> bool {
    a_start < b_end && a_end > b_start
}

fn overlap_ms(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> i64 {
    a_end.min(b_end).saturating_sub(a_start.max(b_start)).max(0)
}

fn should_merge(left: &ShortTurnCandidate, right: &ShortTurnCandidate) -> bool {
    let intersection = overlap_ms(left.start_ms, left.end_ms, right.start_ms, right.end_ms) as f64;
    if intersection <= 0.0 {
        return false;
    }
    let union = left.end_ms.max(right.end_ms) - left.start_ms.min(right.start_ms);
    let shorter = left.duration_ms.min(right.duration_ms).max(1) as f64;
    intersection / union.max(1) as f64 >= 0.5 || intersection / shorter >= 0.8
}

pub fn merge_candidates(mut candidates: Vec<ShortTurnCandidate>) -> Vec<ShortTurnCandidate> {
    candidates.sort_by_key(|candidate| (candidate.start_ms, candidate.end_ms));
    let mut merged: Vec<ShortTurnCandidate> = Vec::new();
    for candidate in candidates {
        if let Some(current) = merged
            .last_mut()
            .filter(|current| should_merge(current, &candidate))
        {
            current.start_ms = current.start_ms.min(candidate.start_ms);
            current.end_ms = current.end_ms.max(candidate.end_ms);
            current.duration_ms = current.end_ms.saturating_sub(current.start_ms) as u64;
            let mut sources: BTreeSet<_> = current.candidate_sources.iter().copied().collect();
            sources.extend(candidate.candidate_sources.iter().copied());
            current.candidate_sources = sources.into_iter().collect();
            current.transcript_ids.extend(candidate.transcript_ids);
            current.transcript_ids.sort();
            current.transcript_ids.dedup();
            if current.text.is_empty() && !candidate.text.is_empty() {
                current.text = candidate.text;
                current.asr_confidence = candidate.asr_confidence;
            } else if current.asr_confidence.is_none() {
                current.asr_confidence = candidate.asr_confidence;
            }
            if candidate.diarization_confidence.unwrap_or(-1.0)
                > current.diarization_confidence.unwrap_or(-1.0)
            {
                current.diarization_speaker = candidate.diarization_speaker;
                current.diarization_confidence = candidate.diarization_confidence;
            }
            current.diarization_coverage_ratio = max_optional(
                current.diarization_coverage_ratio,
                candidate.diarization_coverage_ratio,
            );
            current.vad_confidence = max_optional(current.vad_confidence, candidate.vad_confidence);
            current.overlaps_existing_turn |= candidate.overlaps_existing_turn;
            current.true_speaker_overlap |= candidate.true_speaker_overlap;
        } else {
            merged.push(candidate);
        }
    }
    merged
}

fn max_optional(left: Option<f64>, right: Option<f64>) -> Option<f64> {
    match (left, right) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (value @ Some(_), None) | (None, value @ Some(_)) => value,
        (None, None) => None,
    }
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
        let coverage = candidate
            .diarization_coverage_ratio
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        let effective = base * weight * coverage;

        let strong_direct_speech = normalized_text.is_empty()
            && base >= self.config.high_confidence_threshold
            && coverage >= 0.75;
        let kind =
            if candidate.duration_ms < self.config.min_candidate_ms && normalized_text.is_empty() {
                SegmentKind::Noise
            } else if lexical {
                SegmentKind::Backchannel
            } else if strong_direct_speech {
                SegmentKind::Speech
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

        // Identity uses speaker evidence only. Lexical evidence can classify a
        // backchannel but can never manufacture a speaker attribution.
        let direct_score = effective;
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

        let kind_confidence = match kind {
            SegmentKind::Backchannel => {
                if candidate.asr_confidence.is_some() {
                    0.85
                } else {
                    0.60
                }
            }
            SegmentKind::Noise => (1.0 - base).clamp(0.0, 1.0),
            SegmentKind::Speech => candidate
                .asr_confidence
                .unwrap_or(base * coverage)
                .clamp(0.0, 1.0),
            SegmentKind::Unknown | SegmentKind::NonSpeechVocalization => 0.0,
        };
        let speaker_confidence = speaker.as_ref().map(|_| direct_score.max(0.40));
        let decision = ShortTurnDecision {
            kind,
            kind_confidence,
            speaker_key: speaker,
            speaker_confidence,
            evidence: ShortTurnEvidence {
                normalized_text,
                lexical_backchannel: lexical,
                asr_confidence_present: candidate.asr_confidence.is_some(),
                diarization_confidence_present: candidate.diarization_confidence.is_some(),
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
            decision.speaker_confidence.unwrap_or_default(),
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
        candidate_sources: vec![ShortTurnCandidateSource::Transcript],
        transcript_ids: vec![timing.id.clone()],
        text: text.to_string(),
        asr_confidence,
        diarization_speaker: assignment.speaker_key.clone(),
        diarization_confidence: assignment.speaker_confidence,
        diarization_coverage_ratio: Some(1.0),
        vad_confidence: None,
        audio_source: timing.audio_source.clone(),
        overlaps_existing_turn: !overlapping.is_empty(),
        true_speaker_overlap: assignment.overlap
            || super::timeline::has_true_speaker_overlap(timing, speakers),
        previous_speaker: previous.map(|speaker| speaker.speaker_key.clone()),
        next_speaker: next.map(|speaker| speaker.speaker_key.clone()),
        previous_gap_ms: previous
            .map(|speaker| timing.start_ms.saturating_sub(speaker.end_ms) as u64),
        next_gap_ms: next.map(|speaker| speaker.start_ms.saturating_sub(timing.end_ms) as u64),
    };
    let decision = refiner.refine(&candidate, prototypes);
    TranscriptSpeakerAssignment {
        transcript_id: assignment.transcript_id,
        speaker_confidence: decision.speaker_confidence,
        speaker_key: decision.speaker_key,
        speaker_provisional: false,
        speaker_revision: assignment.speaker_revision,
        segment_kind: decision.kind,
        audio_source: assignment.audio_source,
        assignment_method: AssignmentMethod::ShortTurnRefinement,
        overlap: assignment.overlap,
    }
}

pub fn refine_assignment_with_candidate(
    refiner: &ShortTurnRefiner,
    prototypes: &dyn SpeakerPrototypeStore,
    candidate: &ShortTurnCandidate,
    assignment: TranscriptSpeakerAssignment,
) -> TranscriptSpeakerAssignment {
    if assignment.assignment_method == AssignmentMethod::Manual {
        return assignment;
    }
    let decision = refiner.refine(candidate, prototypes);
    TranscriptSpeakerAssignment {
        transcript_id: assignment.transcript_id,
        speaker_key: decision.speaker_key,
        speaker_confidence: decision.speaker_confidence,
        speaker_provisional: false,
        speaker_revision: assignment.speaker_revision,
        segment_kind: decision.kind,
        audio_source: assignment.audio_source,
        assignment_method: AssignmentMethod::ShortTurnRefinement,
        overlap: assignment.overlap,
    }
}

pub fn apply_revision_semantics(
    previous: &TranscriptSpeakerAssignment,
    mut proposed: TranscriptSpeakerAssignment,
) -> TranscriptSpeakerAssignment {
    let confidence_equal = match (previous.speaker_confidence, proposed.speaker_confidence) {
        (Some(a), Some(b)) => (a - b).abs() < 1e-9,
        (None, None) => true,
        _ => false,
    };
    let changed = previous.speaker_key != proposed.speaker_key
        || !confidence_equal
        || previous.segment_kind != proposed.segment_kind;
    proposed.speaker_revision = if changed {
        previous.speaker_revision.saturating_add(1)
    } else {
        previous.speaker_revision
    };
    proposed
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum DurationBucket {
    Ms100To300,
    Ms300To500,
    Ms500To800,
    Ms800To1200,
    Ms1200To1500Control,
}

impl DurationBucket {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ms100To300 => "100-300ms",
            Self::Ms300To500 => "300-500ms",
            Self::Ms500To800 => "500-800ms",
            Self::Ms800To1200 => "800-1200ms",
            Self::Ms1200To1500Control => "1200-1500ms-control",
        }
    }

    pub fn for_duration(duration_ms: u64) -> Option<Self> {
        match duration_ms {
            100..300 => Some(Self::Ms100To300),
            300..500 => Some(Self::Ms300To500),
            500..800 => Some(Self::Ms500To800),
            800..1200 => Some(Self::Ms800To1200),
            1200..=1500 => Some(Self::Ms1200To1500Control),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BucketedShortTurnMetrics {
    pub sample_count: usize,
    #[serde(flatten)]
    pub metrics: ShortTurnMetrics,
}

pub fn compute_duration_bucket_metrics(
    observations: &[(u64, ShortTurnEvaluationObservation)],
) -> std::collections::BTreeMap<&'static str, BucketedShortTurnMetrics> {
    let mut result = std::collections::BTreeMap::new();
    for bucket in [
        DurationBucket::Ms100To300,
        DurationBucket::Ms300To500,
        DurationBucket::Ms500To800,
        DurationBucket::Ms800To1200,
        DurationBucket::Ms1200To1500Control,
    ] {
        let items: Vec<_> = observations
            .iter()
            .filter(|(duration, _)| DurationBucket::for_duration(*duration) == Some(bucket))
            .map(|(_, observation)| observation.clone())
            .collect();
        result.insert(
            bucket.label(),
            BucketedShortTurnMetrics {
                sample_count: items.len(),
                metrics: compute_metrics(&items),
            },
        );
    }
    result
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateRecallObservation {
    pub expected_short_event: bool,
    pub found_sources: Vec<ShortTurnCandidateSource>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CandidateRecallMetrics {
    pub sample_count: usize,
    pub transcript: f64,
    pub diarizer_turn: f64,
    pub vad_event: f64,
    pub union: f64,
}

pub fn compute_candidate_recall(
    observations: &[CandidateRecallObservation],
) -> CandidateRecallMetrics {
    let expected: Vec<_> = observations
        .iter()
        .filter(|observation| observation.expected_short_event)
        .collect();
    let ratio = |source: Option<ShortTurnCandidateSource>| {
        if expected.is_empty() {
            0.0
        } else {
            expected
                .iter()
                .filter(|observation| match source {
                    Some(source) => observation.found_sources.contains(&source),
                    None => !observation.found_sources.is_empty(),
                })
                .count() as f64
                / expected.len() as f64
        }
    };
    CandidateRecallMetrics {
        sample_count: expected.len(),
        transcript: ratio(Some(ShortTurnCandidateSource::Transcript)),
        diarizer_turn: ratio(Some(ShortTurnCandidateSource::DiarizerTurn)),
        vad_event: ratio(Some(ShortTurnCandidateSource::VadEvent)),
        union: ratio(None),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CandidateMatchConfig {
    /// Engineering evaluation values. These are intentionally stricter than
    /// "any overlap" so a 5 ms boundary touch cannot inflate recall.
    pub min_iou: f64,
    pub min_ground_truth_coverage: f64,
    pub center_tolerance_ms: i64,
}

impl Default for CandidateMatchConfig {
    fn default() -> Self {
        Self {
            min_iou: 0.30,
            min_ground_truth_coverage: 0.75,
            center_tolerance_ms: 200,
        }
    }
}

pub fn candidate_matches_ground_truth(
    candidate_start_ms: i64,
    candidate_end_ms: i64,
    ground_truth_start_ms: i64,
    ground_truth_end_ms: i64,
    config: &CandidateMatchConfig,
) -> bool {
    let intersection = overlap_ms(
        candidate_start_ms,
        candidate_end_ms,
        ground_truth_start_ms,
        ground_truth_end_ms,
    );
    if intersection <= 0 {
        return false;
    }
    let union =
        candidate_end_ms.max(ground_truth_end_ms) - candidate_start_ms.min(ground_truth_start_ms);
    let ground_truth_duration = ground_truth_end_ms
        .saturating_sub(ground_truth_start_ms)
        .max(1);
    let iou = intersection as f64 / union.max(1) as f64;
    let ground_truth_coverage = intersection as f64 / ground_truth_duration as f64;
    let candidate_center = (candidate_start_ms + candidate_end_ms) / 2;
    let ground_truth_center = (ground_truth_start_ms + ground_truth_end_ms) / 2;
    iou >= config.min_iou
        || (ground_truth_coverage >= config.min_ground_truth_coverage
            && (candidate_center - ground_truth_center).abs() <= config.center_tolerance_ms)
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaterializationObservation {
    pub expected_visible: bool,
    pub predicted_visible: bool,
    pub embedded: bool,
    pub expected_speaker: Option<String>,
    pub predicted_speaker: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MaterializationMetrics {
    pub sample_count: usize,
    pub materialization_precision: f64,
    pub materialization_recall: f64,
    pub false_embedded_event_rate: f64,
    pub embedded_event_speaker_accuracy: f64,
}

pub fn compute_materialization_metrics(
    observations: &[MaterializationObservation],
) -> MaterializationMetrics {
    let ratio = |numerator: usize, denominator: usize| {
        if denominator == 0 {
            0.0
        } else {
            numerator as f64 / denominator as f64
        }
    };
    let predicted = observations
        .iter()
        .filter(|item| item.predicted_visible)
        .count();
    let expected = observations
        .iter()
        .filter(|item| item.expected_visible)
        .count();
    let true_positive = observations
        .iter()
        .filter(|item| item.expected_visible && item.predicted_visible)
        .count();
    let embedded_predictions = observations
        .iter()
        .filter(|item| item.embedded && item.predicted_visible)
        .count();
    let false_embedded = observations
        .iter()
        .filter(|item| item.embedded && item.predicted_visible && !item.expected_visible)
        .count();
    let attributed_embedded = observations
        .iter()
        .filter(|item| item.embedded && item.expected_visible && item.expected_speaker.is_some())
        .count();
    let correct_embedded_speaker = observations
        .iter()
        .filter(|item| {
            item.embedded
                && item.expected_visible
                && item.expected_speaker.is_some()
                && item.expected_speaker == item.predicted_speaker
        })
        .count();
    MaterializationMetrics {
        sample_count: observations.len(),
        materialization_precision: ratio(true_positive, predicted),
        materialization_recall: ratio(true_positive, expected),
        false_embedded_event_rate: ratio(false_embedded, embedded_predictions),
        embedded_event_speaker_accuracy: ratio(correct_embedded_speaker, attributed_embedded),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SpeakerAcceptanceMetrics {
    pub sample_count: usize,
    pub transcript_assignment_false_new_speaker_rate: f64,
    pub visible_meeting_speaker_false_new_speaker_rate: f64,
}

pub fn compute_speaker_acceptance_metrics(
    transcript_false_new: &[bool],
    visible_false_new: &[bool],
) -> SpeakerAcceptanceMetrics {
    let ratio = |values: &[bool]| {
        if values.is_empty() {
            0.0
        } else {
            values.iter().filter(|value| **value).count() as f64 / values.len() as f64
        }
    };
    SpeakerAcceptanceMetrics {
        sample_count: transcript_false_new.len().max(visible_false_new.len()),
        transcript_assignment_false_new_speaker_rate: ratio(transcript_false_new),
        visible_meeting_speaker_false_new_speaker_rate: ratio(visible_false_new),
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
            candidate_sources: vec![ShortTurnCandidateSource::Transcript],
            transcript_ids: vec!["t".into()],
            text: text.into(),
            asr_confidence: Some(0.9),
            diarization_speaker: speaker.map(str::to_string),
            diarization_confidence: Some(confidence),
            diarization_coverage_ratio: Some(1.0),
            vad_confidence: None,
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

    fn speaker(start_ms: i64, end_ms: i64, key: &str) -> SpeakerSegment {
        SpeakerSegment {
            start_ms,
            end_ms,
            speaker_key: key.into(),
            speaker_confidence: Some(0.9),
            audio_source: AudioSource::Mixed,
            provisional: false,
            revision: 1,
            segment_kind: SegmentKind::Speech,
            assignment_method: AssignmentMethod::Diarization,
            overlap: false,
        }
    }

    #[test]
    fn extractor_uses_canonical_overlap_semantics_for_handoffs() {
        let extractor = ShortTurnCandidateExtractor::default();
        let transcripts = [TranscriptCandidateInput {
            timing: TranscriptTiming {
                id: "t".into(),
                start_ms: 200,
                end_ms: 900,
                audio_source: AudioSource::Mixed,
            },
            text: "hello".into(),
            asr_confidence: Some(0.9),
        }];
        let handoff = extractor.extract(
            &transcripts,
            &[
                speaker(0, 600, "speaker_01"),
                speaker(600, 1_100, "speaker_02"),
            ],
            &[],
        );
        assert!(!handoff[0].true_speaker_overlap);
        let overlap = extractor.extract(
            &transcripts,
            &[
                speaker(0, 800, "speaker_01"),
                speaker(500, 1_100, "speaker_02"),
            ],
            &[],
        );
        assert!(overlap[0].true_speaker_overlap);
    }

    #[test]
    fn extractor_recovers_and_merges_all_three_sources() {
        let extractor = ShortTurnCandidateExtractor::default();
        let results = extractor.extract(
            &[TranscriptCandidateInput {
                timing: TranscriptTiming {
                    id: "t".into(),
                    start_ms: 100,
                    end_ms: 500,
                    audio_source: AudioSource::Mixed,
                },
                text: "嗯".into(),
                asr_confidence: Some(0.8),
            }],
            &[speaker(120, 510, "speaker_02")],
            &[VadEventCandidateInput {
                start_ms: 90,
                end_ms: 520,
                confidence: None,
                audio_source: AudioSource::Mixed,
            }],
        );
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].candidate_sources,
            vec![
                ShortTurnCandidateSource::Transcript,
                ShortTurnCandidateSource::DiarizerTurn,
                ShortTurnCandidateSource::VadEvent,
            ]
        );
    }

    #[test]
    fn adjacent_distinct_events_are_not_merged() {
        let mut first = candidate(200, "", Some("speaker_01"), 0.9);
        first.start_ms = 0;
        first.end_ms = 200;
        first.candidate_sources = vec![ShortTurnCandidateSource::DiarizerTurn];
        let mut second = candidate(200, "", Some("speaker_02"), 0.9);
        second.start_ms = 250;
        second.end_ms = 450;
        second.candidate_sources = vec![ShortTurnCandidateSource::VadEvent];
        assert_eq!(merge_candidates(vec![first, second]).len(), 2);
    }

    #[test]
    fn benchmark_matching_rejects_boundary_scrapes_and_accepts_real_coverage() {
        let config = CandidateMatchConfig::default();
        assert!(!candidate_matches_ground_truth(0, 105, 100, 300, &config));
        assert!(candidate_matches_ground_truth(90, 290, 100, 300, &config));
    }

    #[test]
    fn production_acceptance_policy_is_pure_and_requires_corroboration() {
        let policy = SpeakerAcceptancePolicy::default();
        let one_unknown = vec![SpeakerAcceptanceTurn {
            start_ms: 0,
            end_ms: 2_000,
            speaker_key: "speaker_01".into(),
            confidence: None,
        }];
        assert!(policy.accepted_speaker_keys(&one_unknown).is_empty());
        let corroborated = vec![
            SpeakerAcceptanceTurn {
                start_ms: 0,
                end_ms: 2_000,
                speaker_key: "speaker_01".into(),
                confidence: None,
            },
            SpeakerAcceptanceTurn {
                start_ms: 3_000,
                end_ms: 5_000,
                speaker_key: "speaker_01".into(),
                confidence: None,
            },
        ];
        assert_eq!(
            policy.accepted_speaker_keys(&corroborated),
            BTreeSet::from(["speaker_01".to_string()])
        );
    }

    #[test]
    fn missing_asr_confidence_is_diagnostic_not_a_neutral_score() {
        let mut value = candidate(250, "oh", None, 0.0);
        value.asr_confidence = None;
        value.diarization_confidence = None;
        let decision = ShortTurnRefiner::default().refine(
            &value,
            &MeetingSpeakerPrototypeStore::new(["speaker_01".into()]),
        );
        assert!(!decision.evidence.asr_confidence_present);
        assert_eq!(decision.speaker_key, None);
        assert_eq!(decision.speaker_confidence, None);
    }

    #[test]
    fn identical_revision_result_does_not_increment() {
        let previous = TranscriptSpeakerAssignment {
            transcript_id: "t".into(),
            speaker_key: Some("speaker_01".into()),
            speaker_confidence: Some(0.7),
            speaker_provisional: false,
            speaker_revision: 4,
            segment_kind: SegmentKind::Speech,
            audio_source: AudioSource::Mixed,
            assignment_method: AssignmentMethod::Diarization,
            overlap: false,
        };
        assert_eq!(
            apply_revision_semantics(&previous, previous.clone()).speaker_revision,
            4
        );
        let mut changed = previous.clone();
        changed.segment_kind = SegmentKind::Backchannel;
        assert_eq!(
            apply_revision_semantics(&previous, changed).speaker_revision,
            5
        );
    }
}
