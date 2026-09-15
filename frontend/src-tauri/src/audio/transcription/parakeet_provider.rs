// audio/transcription/parakeet_provider.rs
//
// Parakeet transcription provider implementation.

use super::provider::{
    TimedToken, TimingSource, TranscriptResult, TranscriptTiming, TranscriptionCapabilities,
    TranscriptionError, TranscriptionProvider,
};
use async_trait::async_trait;
use log::warn;
use std::sync::Arc;

/// Parakeet transcription provider (wraps ParakeetEngine)
pub struct ParakeetProvider {
    engine: Arc<crate::parakeet_engine::ParakeetEngine>,
}

impl ParakeetProvider {
    pub fn new(engine: Arc<crate::parakeet_engine::ParakeetEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl TranscriptionProvider for ParakeetProvider {
    async fn transcribe(
        &self,
        audio: Vec<f32>,
        language: Option<String>,
    ) -> std::result::Result<TranscriptResult, TranscriptionError> {
        // Log language preference warning if set (Parakeet doesn't support it yet)
        if let Some(ref lang) = language {
            warn!(
                "Parakeet doesn't support language preference '{}' yet - transcribing in default language",
                lang
            );
        }

        match self.engine.transcribe_audio_with_timing(audio).await {
            Ok(result) => Ok(transcript_result_from_native(result)),
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
        "Parakeet"
    }

    fn capabilities(&self) -> TranscriptionCapabilities {
        parakeet_capabilities()
    }
}

pub fn parakeet_capabilities() -> TranscriptionCapabilities {
    TranscriptionCapabilities {
        segment_timestamps: false,
        token_timestamps: true,
        word_timestamps: false,
        token_confidence: false,
    }
}

pub fn transcript_result_from_native(
    result: crate::parakeet_engine::model::TimestampedResult,
) -> TranscriptResult {
    let surface_pieces = surface_token_pieces(&result.tokens);
    let tokens = (surface_pieces.len() == result.timestamps.len())
        .then(|| {
            surface_pieces
                .into_iter()
                .zip(result.timestamps)
                .map(|(text, seconds)| {
                    checked_seconds_to_ms(seconds).map(|start_ms| TimedToken {
                        text,
                        start_ms,
                        end_ms: None,
                        confidence: None,
                        timing_source: TimingSource::NativeTokenEmission,
                    })
                })
                .collect::<Option<Vec<_>>>()
        })
        .flatten();
    TranscriptResult {
        text: result.text.trim().to_string(),
        confidence: None,
        is_partial: false,
        timing: tokens.map(|tokens| TranscriptTiming {
            provider: "parakeet".to_string(),
            capabilities: parakeet_capabilities(),
            tokens,
        }),
    }
}

fn checked_seconds_to_ms(seconds: f32) -> Option<i64> {
    let milliseconds = f64::from(seconds) * 1_000.0;
    (milliseconds.is_finite() && milliseconds >= i64::MIN as f64 && milliseconds <= i64::MAX as f64)
        .then(|| milliseconds.round() as i64)
}

/// Attribute every change in the incremental native decoder output to the
/// token that caused it. Earlier pieces may be shortened when whitespace
/// normalization changes at a token boundary; final concatenation is exact.
fn surface_token_pieces(native_tokens: &[String]) -> Vec<String> {
    let mut pieces = vec![String::new(); native_tokens.len()];
    let mut previous = String::new();
    for index in 0..native_tokens.len() {
        let current = crate::parakeet_engine::model::decode_token_text(&native_tokens[..=index]);
        let common = common_prefix_bytes(&previous, &current);
        let mut remove = previous.len().saturating_sub(common);
        for piece in pieces[..index].iter_mut().rev() {
            if remove == 0 {
                break;
            }
            let take = remove.min(piece.len());
            let mut keep = piece.len() - take;
            while !piece.is_char_boundary(keep) {
                keep = keep.saturating_sub(1);
            }
            let removed = piece.len() - keep;
            piece.truncate(keep);
            remove = remove.saturating_sub(removed);
        }
        pieces[index].push_str(&current[common..]);
        previous = current;
    }
    pieces
}

fn common_prefix_bytes(left: &str, right: &str) -> usize {
    left.char_indices()
        .zip(right.char_indices())
        .take_while(|((_, left), (_, right))| left == right)
        .map(|((index, character), _)| index + character.len_utf8())
        .last()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_pieces_reproduce_native_decoder_text() {
        let tokens = vec![" hello".to_string(), " world".to_string()];
        assert_eq!(
            surface_token_pieces(&tokens).concat(),
            crate::parakeet_engine::model::decode_token_text(&tokens)
        );
    }

    #[test]
    fn native_emission_timestamps_are_preserved_in_milliseconds() {
        let result =
            transcript_result_from_native(crate::parakeet_engine::model::TimestampedResult {
                text: "hello world".into(),
                timestamps: vec![0.08, 0.16],
                tokens: vec![" hello".into(), " world".into()],
            });
        let timing = result.timing.expect("valid native timing");
        assert_eq!(timing.tokens[0].start_ms, 80);
        assert_eq!(timing.tokens[1].start_ms, 160);
        assert!(timing.tokens.iter().all(|token| {
            token.end_ms.is_none() && token.timing_source == TimingSource::NativeTokenEmission
        }));
    }

    #[test]
    fn invalid_native_timestamp_disables_timing_without_losing_text() {
        let result =
            transcript_result_from_native(crate::parakeet_engine::model::TimestampedResult {
                text: "hello".into(),
                timestamps: vec![f32::NAN],
                tokens: vec![" hello".into()],
            });
        assert_eq!(result.text, "hello");
        assert!(result.timing.is_none());
    }
}
