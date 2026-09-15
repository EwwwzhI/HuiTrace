// audio/transcription/whisper_provider.rs
//
// Whisper transcription provider implementation.

use super::provider::{
    TranscriptResult, TranscriptionCapabilities, TranscriptionError, TranscriptionProvider,
};
use async_trait::async_trait;
use std::sync::Arc;

/// Whisper transcription provider (wraps WhisperEngine)
pub struct WhisperProvider {
    engine: Arc<crate::whisper_engine::WhisperEngine>,
}

impl WhisperProvider {
    pub fn new(engine: Arc<crate::whisper_engine::WhisperEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl TranscriptionProvider for WhisperProvider {
    async fn transcribe(
        &self,
        audio: Vec<f32>,
        language: Option<String>,
    ) -> std::result::Result<TranscriptResult, TranscriptionError> {
        match self
            .engine
            .transcribe_audio_with_confidence(audio, language)
            .await
        {
            Ok((text, confidence, is_partial)) => Ok(TranscriptResult {
                text: text.trim().to_string(),
                confidence: Some(confidence),
                is_partial,
                timing: None,
            }),
            Err(e) => Err(TranscriptionError::EngineFailed(e.to_string())),
        }
    }

    async fn is_model_loaded(&self) -> bool {
        self.engine.is_model_loaded().await
    }

    async fn get_current_model(&self) -> Option<String> {
        self.engine.get_current_model().await
    }

    fn provider_name(&self) -> &'static str {
        "Whisper"
    }

    fn capabilities(&self) -> TranscriptionCapabilities {
        whisper_capabilities()
    }
}

pub fn whisper_capabilities() -> TranscriptionCapabilities {
    // `no_timestamps=true` is intentionally retained to prevent the known
    // chunk-skipping regression. Token timing is therefore not advertised.
    TranscriptionCapabilities::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_policy_remains_text_only() {
        assert_eq!(whisper_capabilities(), TranscriptionCapabilities::default());
    }
}
