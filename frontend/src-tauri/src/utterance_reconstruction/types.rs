use serde::{Deserialize, Serialize};

use crate::diarization::types::SegmentKind;

pub const ALGORITHM_VERSION: &str = "utterance-reconstruction-v1";

/// Reserved word-level seam for V2. V1 never manufactures these timings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimedWord {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub confidence: Option<f64>,
    pub source_chunk_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpeakerAttribution {
    Single { speaker_key: String },
    Mixed { speaker_keys: Vec<String> },
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

/// The smallest V1 text-bearing unit. It always maps to exactly one raw row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtomicSpan {
    pub start_ms: i64,
    pub end_ms: i64,
    pub timing_reliable: bool,
    pub text: String,
    pub speaker_attribution: SpeakerAttribution,
    pub source_transcript_ids: Vec<String>,
    pub asr_confidence: Option<f64>,
    pub speaker_confidence: Option<f64>,
    pub overlap: bool,
    pub segment_kind: SegmentKind,
    pub short_turn_confidence: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryDecision {
    Split,
    Merge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryReason {
    SameSpeaker,
    SpeakerChanged,
    SpeakerUnknown,
    ShortGap,
    MediumGap,
    LongSilence,
    StrongTerminalPunctuation,
    WeakPunctuation,
    ContinuationPrefix,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoundaryEvidence {
    pub left_source_transcript_id: String,
    pub right_source_transcript_id: String,
    pub gap_ms: Option<i64>,
    pub same_speaker: Option<bool>,
    pub strong_terminal_punctuation: bool,
    pub weak_punctuation: bool,
    pub continuation_prefix: bool,
    pub projected_duration_ms: i64,
    pub projected_text_length: usize,
    pub mixed_attribution: bool,
    pub overlap: bool,
    pub backchannel_between: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoundaryOutcome {
    pub evidence: BoundaryEvidence,
    pub score: i32,
    pub decision: BoundaryDecision,
    pub reasons: Vec<BoundaryReason>,
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
    pub reconstruction_confidence: f64,
    pub reconstruction_reasons: Vec<BoundaryReason>,
    pub overlap: bool,
    pub mixed: bool,
    pub embedded_events: Vec<ReconstructedEvent>,
    pub algorithm_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReconstructionResult {
    pub meeting_id: String,
    pub algorithm_version: String,
    pub utterances: Vec<ReconstructedUtterance>,
    /// Events that could not safely be embedded in a surrounding utterance.
    pub events: Vec<ReconstructedEvent>,
    pub boundaries: Vec<BoundaryOutcome>,
}
