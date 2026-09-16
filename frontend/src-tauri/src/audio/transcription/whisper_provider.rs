// audio/transcription/whisper_provider.rs
//
// Whisper transcription provider implementation.

use super::provider::{
    TimedToken, TimingSource, TranscriptResult, TranscriptTiming, TranscriptionCapabilities,
    TranscriptionError, TranscriptionProvider,
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
        let duration_ms = (audio.len() / 16) as i64;
        match self
            .engine
            .transcribe_audio_with_timing(audio, language)
            .await
        {
            Ok(native) => Ok(transcript_result_from_native(native, duration_ms)),
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
    // Decoder timestamp tokens remain disabled; lexical timing is computed
    // independently by whisper.cpp. This is BPE timing, not word timing.
    TranscriptionCapabilities {
        token_timestamps: true,
        token_confidence: true,
        ..TranscriptionCapabilities::default()
    }
}

pub(crate) fn transcript_result_from_native(
    native: crate::whisper_engine::WhisperTranscriptionResult,
    chunk_duration_ms: i64,
) -> TranscriptResult {
    let tokens = native
        .tokens
        .into_iter()
        .filter(|token| !token.text.is_empty())
        .map(|token| TimedToken {
            text: token.text,
            start_ms: token.start_ms,
            end_ms: token.end_ms,
            confidence: token.confidence,
            timing_source: TimingSource::NativeToken,
        })
        .collect::<Vec<_>>();
    // One centisecond accommodates native timestamp quantization only. Starts
    // and ends must be strictly monotonic (zero jitter tolerance).
    let valid = crate::utterance_reconstruction::validate_lexical_timing(
        &native.text,
        &tokens,
        chunk_duration_ms,
        10,
    )
    .is_ok();
    TranscriptResult {
        text: native.text,
        confidence: Some(native.confidence),
        is_partial: native.is_partial,
        timing: valid.then(|| TranscriptTiming {
            provider: "Whisper".into(),
            capabilities: whisper_capabilities(),
            tokens,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::whisper_engine::{WhisperTimedToken, WhisperTranscriptionResult};

    fn native(text: &str, parts: &[&str]) -> WhisperTranscriptionResult {
        WhisperTranscriptionResult {
            text: text.into(),
            confidence: 0.75,
            is_partial: true,
            tokens: parts
                .iter()
                .enumerate()
                .map(|(i, text)| WhisperTimedToken {
                    text: (*text).into(),
                    start_ms: i as i64 * 100,
                    end_ms: Some((i as i64 + 1) * 100),
                    confidence: Some(0.9),
                })
                .collect(),
        }
    }

    #[test]
    fn preserves_bpe_spaces_punctuation_unicode_and_partial_text() {
        for (text, parts) in [
            ("Hello, world!", vec![" Hello", ",", " world", "!"]),
            (
                "这个 module 的 latency。",
                vec!["这", "个", " mod", "ule", " 的", " latency", "。"],
            ),
            ("café déjà", vec![" café", " déjà"]),
            ("a  b", vec![" a", " b"]),
        ] {
            let result = transcript_result_from_native(native(text, &parts), 2000);
            assert_eq!(result.text, text);
            assert_eq!(result.confidence, Some(0.75));
            assert!(result.is_partial);
            let timing = result.timing.expect("valid timing");
            assert!(timing
                .tokens
                .iter()
                .all(|t| t.timing_source == TimingSource::NativeToken));
        }
    }

    #[test]
    fn every_metadata_failure_preserves_authoritative_text() {
        let baseline = native(
            "First sentence. Final sentence!",
            &["First sentence.", " Final sentence!"],
        );
        let mut cases = Vec::new();
        let mut bad = baseline.clone();
        bad.tokens.clear();
        cases.push(bad);
        let mut bad = baseline.clone();
        bad.tokens.pop();
        cases.push(bad);
        let mut bad = baseline.clone();
        bad.tokens[0].text = "Changed".into();
        cases.push(bad);
        let mut bad = baseline.clone();
        bad.tokens[0].start_ms = -1;
        cases.push(bad);
        let mut bad = baseline.clone();
        bad.tokens[1].start_ms = 0;
        bad.tokens[0].start_ms = 50;
        cases.push(bad);
        let mut bad = baseline.clone();
        bad.tokens[1].end_ms = Some(99);
        cases.push(bad);
        let mut bad = baseline.clone();
        bad.tokens[1].end_ms = Some(2011);
        cases.push(bad);
        let mut bad = baseline.clone();
        bad.tokens[1].end_ms = None;
        cases.push(bad);
        let mut bad = baseline.clone();
        bad.tokens[0].text = "First sentence".into();
        cases.push(bad);
        for bad in cases {
            let result = transcript_result_from_native(bad, 2000);
            assert_eq!(result.text, baseline.text);
            assert_eq!(result.confidence, Some(baseline.confidence));
            assert_eq!(result.is_partial, baseline.is_partial);
            assert!(result.timing.is_none());
        }
    }

    #[test]
    fn bounds_quantization_extreme_duration_and_short_audio() {
        let mut value = native("word", &["word"]);
        value.tokens[0].end_ms = Some(1010);
        assert!(transcript_result_from_native(value.clone(), 1000)
            .timing
            .is_some());
        assert!(transcript_result_from_native(value.clone(), 999)
            .timing
            .is_none());
        value.tokens[0].end_ms = Some(10_001);
        assert!(transcript_result_from_native(value.clone(), 30_000)
            .timing
            .is_none());
        assert!(transcript_result_from_native(value, 0).timing.is_none());
    }

    #[test]
    fn timestamp_policy_exposes_bpe_intervals_only() {
        assert!(whisper_capabilities().token_timestamps);
        assert!(whisper_capabilities().token_confidence);
        assert!(!whisper_capabilities().word_timestamps);
        assert!(!whisper_capabilities().segment_timestamps);
    }
}
