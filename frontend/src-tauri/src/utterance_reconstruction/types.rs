use serde::{Deserialize, Serialize};

use crate::audio::transcription::TimingSource;
use crate::diarization::types::SegmentKind;

pub const ALGORITHM_VERSION: &str = "utterance-reconstruction-v1-frozen";
pub const ALGORITHM_VERSION_V3: &str = "utterance-reconstruction-v3-semantic-baseline";
pub const BOUNDARY_POLICY_VERSION_V1_FROZEN: &str = "boundary-policy-v1-frozen";
pub const BOUNDARY_POLICY_VERSION_V3: &str = "boundary-policy-v3-semantic-baseline";
pub const SEMANTIC_MODEL_VERSION_V1: &str = "deterministic-semantic-baseline-v1";
pub const ALIGNMENT_VERSION_V1: &str = "lexical-temporal-alignment-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconstructionTimingMode {
    ChunkFallback,
    NativeLexicalTiming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryPolicy {
    V1Frozen,
    V3SemanticBaseline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconstructionProfile {
    pub algorithm_version: String,
    pub timing_mode: ReconstructionTimingMode,
    pub boundary_policy: BoundaryPolicy,
    pub boundary_policy_version: String,
    pub semantic_model_version: Option<String>,
    pub alignment_version: Option<String>,
}

/// Reserved word-level seam for timing-aware reconstruction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimedWord {
    pub id: String,
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub confidence: Option<f64>,
    pub source_chunk_id: String,
    pub lexical_index: usize,
    pub token_start_index: usize,
    pub token_end_index: usize,
    pub timing_source: TimingSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLexicalRange {
    pub source_transcript_id: String,
    pub lexical_start_index: usize,
    pub lexical_end_index: usize,
    pub token_start_index: usize,
    pub token_end_index: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeakerCandidate {
    pub speaker_key: String,
    pub overlap_ms: i64,
    pub overlap_ratio: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WordSpeakerStatus {
    Assigned,
    Ambiguous,
    Mixed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignmentReason {
    DominantTemporalOverlap,
    InsufficientOverlap,
    InsufficientMargin,
    TrueOverlap,
    NoSpeakerEvidence,
    ManualAssignment,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WordSpeakerAssignment {
    pub word: TimedWord,
    pub speaker_key: Option<String>,
    pub status: WordSpeakerStatus,
    pub confidence: Option<f64>,
    pub best_overlap_ratio: f64,
    pub candidates: Vec<SpeakerCandidate>,
    pub reasons: Vec<AlignmentReason>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlignmentDiagnostic {
    pub source_transcript_id: String,
    pub word_id: String,
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub timing_source: TimingSource,
    pub candidates: Vec<SpeakerCandidate>,
    pub status: WordSpeakerStatus,
    pub speaker_key: Option<String>,
    pub best_overlap_ratio: f64,
    pub reasons: Vec<AlignmentReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingDiagnostic {
    pub source_transcript_id: String,
    pub valid: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReconstructionMetrics {
    pub total_chunks: usize,
    pub chunks_with_timing: usize,
    pub valid_timing_chunks: usize,
    pub timing_coverage: f64,
    pub valid_timing_rate: f64,
    pub total_lexical_units: usize,
    pub assigned_lexical_units: usize,
    pub word_assignment_coverage: f64,
    pub ambiguous_count: usize,
    pub ambiguous_rate: f64,
    pub mixed_count: usize,
    pub mixed_rate: f64,
    pub cross_speaker_raw_chunk_count: usize,
    /// Diagnostic-only: the Candidate emitted more than one assigned speaker for the raw
    /// chunk. This is resolution coverage, not correctness or handoff recall.
    pub resolved_cross_speaker_chunk_count: usize,
    /// Diagnostic coverage metric. Ground Truth evaluation is required before
    /// this value may be interpreted as a correct resolution rate.
    pub resolved_cross_speaker_chunk_rate: f64,
    pub chunk_fallback_count: usize,
    pub chunk_fallback_rate: f64,
    pub lexical_preservation_failure_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpeakerAttribution {
    Single { speaker_key: String },
    Mixed { speaker_keys: Vec<String> },
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeakerAttributionSource {
    Manual,
    LexicalTemporalOverlap,
    ChunkTemporalOverlap,
    PersistedFallback,
    Unknown,
}

impl SpeakerAttribution {
    pub fn single_key(&self) -> Option<&str> {
        match self {
            Self::Single { speaker_key } => Some(speaker_key),
            Self::Mixed { .. } | Self::Unknown => None,
        }
    }

    pub fn is_mixed(&self) -> bool {
        matches!(self, Self::Mixed { .. })
    }
}

/// The smallest reconstruction text-bearing unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtomicSpan {
    pub start_ms: i64,
    pub end_ms: i64,
    pub timing_reliable: bool,
    pub text: String,
    pub speaker_attribution: SpeakerAttribution,
    pub source_transcript_ids: Vec<String>,
    pub asr_confidence: Option<f64>,
    pub speaker_assignment_reliability: Option<f64>,
    pub speaker_attribution_source: SpeakerAttributionSource,
    pub overlap: bool,
    pub segment_kind: SegmentKind,
    pub short_turn_confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lexical_range: Option<SourceLexicalRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryDecision {
    Split,
    Merge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryDecisionSource {
    HardSafetyConstraint,
    ReliableSpeakerHandoff,
    FrozenPolicyHardSplit,
    ScoredDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryReason {
    SameSpeaker,
    SpeakerChanged,
    ReliableSpeakerChange,
    AmbiguousSpeakerChange,
    SpeakerUnknown,
    ShortGap,
    MediumGap,
    LongSilence,
    StrongTerminalPunctuation,
    WeakPunctuation,
    ContinuationPrefix,
    SentenceComplete,
    SentenceIncomplete,
    SemanticContinuity,
    MixedAttribution,
    Overlap,
    OverlappingTimeline,
    UnreliableTiming,
    MaximumDuration,
    MaximumLength,
    BackchannelBridge,
    ScoreThreshold,
    BelowScoreThreshold,
}

/// Model-ready semantic seam. V3 currently fills this with the deterministic
/// baseline in `semantic.rs`; `None` means that no trustworthy feature exists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticBoundaryEvidence {
    pub baseline_version: String,
    pub left_completeness: Option<f32>,
    pub cross_boundary_continuity: Option<f32>,
}

/// Reserved for future acoustic/prosodic features. An unavailable signal is
/// represented explicitly instead of manufacturing a score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProsodicBoundaryEvidence {
    pub available: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundaryScoreComponents {
    pub timing_score: i32,
    pub speaker_score: i32,
    pub punctuation_score: i32,
    pub semantic_score: i32,
    pub structural_score: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoundaryEvidence {
    pub left_source_transcript_id: String,
    pub right_source_transcript_id: String,
    pub gap_ms: Option<i64>,
    pub same_speaker: Option<bool>,
    pub speaker_change_reliability: Option<f64>,
    pub speaker_change_reliable: bool,
    pub left_speaker_attribution_source: SpeakerAttributionSource,
    pub right_speaker_attribution_source: SpeakerAttributionSource,
    pub timing_reliable: bool,
    pub strong_terminal_punctuation: bool,
    pub weak_punctuation: bool,
    pub continuation_prefix: bool,
    pub projected_duration_ms: i64,
    pub projected_text_length: usize,
    pub mixed_attribution: bool,
    pub overlap: bool,
    pub backchannel_between: bool,
    pub semantic: Option<SemanticBoundaryEvidence>,
    pub prosody: ProsodicBoundaryEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoundaryOutcome {
    pub evidence: BoundaryEvidence,
    pub score: i32,
    pub decision: BoundaryDecision,
    pub decision_source: BoundaryDecisionSource,
    pub reasons: Vec<BoundaryReason>,
    pub score_components: BoundaryScoreComponents,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReconstructedEvent {
    pub id: String,
    pub meeting_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    pub speaker_attribution: SpeakerAttribution,
    pub source_transcript_ids: Vec<String>,
    pub kind: SegmentKind,
    pub confidence: Option<f64>,
    pub overlap: bool,
    pub algorithm_version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_ranges: Vec<SourceLexicalRange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReconstructedUtterance {
    pub id: String,
    pub meeting_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker_attribution: SpeakerAttribution,
    pub text: String,
    pub source_transcript_ids: Vec<String>,
    pub mean_asr_confidence: Option<f64>,
    pub reconstruction_reasons: Vec<BoundaryReason>,
    pub overlap: bool,
    pub mixed: bool,
    pub embedded_events: Vec<ReconstructedEvent>,
    pub algorithm_version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_ranges: Vec<SourceLexicalRange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReconstructionResult {
    pub meeting_id: String,
    pub algorithm_version: String,
    pub profile: ReconstructionProfile,
    pub utterances: Vec<ReconstructedUtterance>,
    /// Events that could not safely be embedded in a surrounding utterance.
    pub events: Vec<ReconstructedEvent>,
    pub boundaries: Vec<BoundaryOutcome>,
    pub config_version: String,
    pub config_hash: String,
    pub config: super::config::UtteranceReconstructionConfig,
    pub metrics: ReconstructionMetrics,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alignment_diagnostics: Vec<AlignmentDiagnostic>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timing_diagnostics: Vec<TimingDiagnostic>,
}
