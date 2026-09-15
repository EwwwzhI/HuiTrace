// audio/transcription/provider.rs
//
// Defines the unified TranscriptionProvider trait and common types for all
// transcription engines (Whisper, Parakeet, future providers).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ============================================================================
// TRANSCRIPTION PROVIDER TRAIT & ERROR TYPES
// ============================================================================

/// Granular error types for transcription operations
#[derive(Debug, Clone)]
pub enum TranscriptionError {
    ModelNotLoaded,
    AudioTooShort { samples: usize, minimum: usize },
    EngineFailed(String),
    UnsupportedLanguage(String),
}

impl std::fmt::Display for TranscriptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ModelNotLoaded => write!(f, "No transcription model is loaded"),
            Self::AudioTooShort { samples, minimum } => write!(
                f,
                "Audio too short: {} samples (minimum {})",
                samples, minimum
            ),
            Self::EngineFailed(msg) => write!(f, "Transcription engine failed: {}", msg),
            Self::UnsupportedLanguage(lang) => {
                write!(f, "Language '{}' is not supported by this provider", lang)
            }
        }
    }
}

impl std::error::Error for TranscriptionError {}

/// Unified transcription result across all providers
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptionCapabilities {
    pub segment_timestamps: bool,
    pub token_timestamps: bool,
    pub word_timestamps: bool,
    pub token_confidence: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimingSource {
    NativeTokenEmission,
    NativeToken,
    NativeWord,
    NativeSegment,
    ForcedAlignment,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimedToken {
    pub text: String,
    /// Provider-native time relative to the input ASR chunk.
    pub start_ms: i64,
    /// `None` for point-like emission timestamps such as Parakeet TDT/RNN-T.
    pub end_ms: Option<i64>,
    pub confidence: Option<f64>,
    pub timing_source: TimingSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptTiming {
    pub provider: String,
    pub capabilities: TranscriptionCapabilities,
    pub tokens: Vec<TimedToken>,
}

/// Unified transcription result across all providers.
#[derive(Debug, Clone)]
pub struct TranscriptResult {
    pub text: String,
    pub confidence: Option<f32>, // None if provider doesn't support confidence scores
    pub is_partial: bool,
    pub timing: Option<TranscriptTiming>,
}

/// Trait for transcription providers (Whisper, Parakeet, future providers)
#[async_trait]
pub trait TranscriptionProvider: Send + Sync {
    /// Transcribe audio samples to text
    ///
    /// # Arguments
    /// * `audio` - Audio samples (16kHz mono, f32 format)
    /// * `language` - Optional language hint (e.g., "en", "es", "fr")
    ///
    /// # Returns
    /// * `TranscriptResult` with text, optional confidence, and partial flag
    async fn transcribe(
        &self,
        audio: Vec<f32>,
        language: Option<String>,
    ) -> std::result::Result<TranscriptResult, TranscriptionError>;

    /// Check if a model is currently loaded
    async fn is_model_loaded(&self) -> bool;

    /// Get the name of the currently loaded model
    async fn get_current_model(&self) -> Option<String>;

    /// Get the provider name (for logging/debugging)
    fn provider_name(&self) -> &'static str;

    fn capabilities(&self) -> TranscriptionCapabilities;
}
