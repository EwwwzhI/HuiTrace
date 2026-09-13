//! Domain types shared by capture, offline diarization and transcript fusion.
//!
//! `AudioSource` deliberately is not `audio::recording_state::DeviceType`.
//! The latter says where a PCM chunk was captured; this module describes the
//! audio represented by an analysed timeline or transcript assignment.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioSource {
    Microphone,
    System,
    Imported,
    Mixed,
}

impl AudioSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Microphone => "microphone",
            Self::System => "system",
            Self::Imported => "imported",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentKind {
    Speech,
    Unknown,
}

impl SegmentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Speech => "speech",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentMethod {
    Diarization,
    Manual,
}

impl AssignmentMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Diarization => "diarization",
            Self::Manual => "manual",
        }
    }
}

/// A contiguous, diarizer-owned section of the meeting timeline. Times are
/// recording-relative milliseconds, matching `speaker_turns` and VAD.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeakerSegment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker_key: String,
    pub speaker_confidence: Option<f64>,
    pub audio_source: AudioSource,
    pub provisional: bool,
    pub revision: i64,
    pub segment_kind: SegmentKind,
    pub assignment_method: AssignmentMethod,
    pub overlap: bool,
}

/// Timestamp-only input for the pure timeline reconciler.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptTiming {
    pub id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub audio_source: AudioSource,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptSpeakerAssignment {
    pub transcript_id: String,
    pub speaker_key: Option<String>,
    pub speaker_confidence: Option<f64>,
    pub speaker_provisional: bool,
    pub speaker_revision: i64,
    pub segment_kind: SegmentKind,
    pub audio_source: AudioSource,
    pub assignment_method: AssignmentMethod,
    pub overlap: bool,
}
