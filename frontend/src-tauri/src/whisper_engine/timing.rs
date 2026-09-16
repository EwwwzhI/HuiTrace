//! Optional metadata only: failure here must never fail inference or rewrite text.
use anyhow::{anyhow, Result};
use whisper_rs::{WhisperContext, WhisperState};

#[derive(Debug, Clone)]
pub struct WhisperTimedToken {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: Option<i64>,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct WhisperTranscriptionResult {
    pub text: String,
    pub confidence: f32,
    pub is_partial: bool,
    pub tokens: Vec<WhisperTimedToken>,
}

// whisper.cpp timestamp_to_sample(t) = t * WHISPER_SAMPLE_RATE / 100.
// Checked multiplication also rejects corrupt/overflowing native metadata.
fn milliseconds(ticks: i64) -> Result<i64> {
    ticks
        .checked_mul(10)
        .ok_or_else(|| anyhow!("timestamp overflow"))
}

fn is_lexical(id: i32, eot: i32) -> bool {
    id >= 0 && id < eot
}

pub(super) fn extract_tokens(
    ctx: &WhisperContext,
    state: &WhisperState,
) -> Result<Vec<WhisperTimedToken>> {
    let mut tokens = Vec::new();
    for segment in 0..state.full_n_segments()? {
        let mut segment_tokens = Vec::new();
        for index in 0..state.full_n_tokens(segment)? {
            let data = state.full_get_token_data(segment, index)?;
            // Matches whisper.cpp's lexical vocabulary boundary, including multilingual models.
            if !is_lexical(data.id, ctx.token_eot()) {
                continue;
            }
            // Do not use lossy UTF-8: an incomplete BPE byte fragment invalidates
            // this chunk's metadata, while the segment-based ASR text survives.
            let text = ctx.token_to_cstr(data.id)?.to_str()?.to_owned();
            if text.is_empty() {
                continue;
            }
            segment_tokens.push(WhisperTimedToken {
                text,
                start_ms: milliseconds(data.t0)?,
                end_ms: Some(milliseconds(data.t1)?),
                confidence: (data.p.is_finite() && (0.0..=1.0).contains(&data.p))
                    .then_some(f64::from(data.p)),
            });
        }
        // The existing authoritative text path trims segments and joins them
        // with one space. Mirror ONLY that whitespace in the metadata.
        if let Some(first) = segment_tokens.first_mut() {
            first.text = first.text.trim_start().to_owned();
            if !tokens.is_empty() {
                first.text.insert(0, ' ');
            }
        }
        if let Some(last) = segment_tokens.last_mut() {
            last.text = last.text.trim_end().to_owned();
        }
        tokens.extend(segment_tokens);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_units_are_centiseconds_with_checked_conversion() {
        assert_eq!(milliseconds(123).unwrap(), 1230);
        assert!(milliseconds(i64::MAX).is_err());
    }

    #[test]
    fn single_timestamp_ending_is_metadata_only_and_never_lexical() {
        // Multilingual vocabulary: EOT, SOT, language/task/no-timestamps and
        // timestamp IDs are all outside the lexical prefix. No token string
        // heuristics that could accidentally reject ordinary literal text.
        for eot in [50256, 50257] {
            assert!(is_lexical(eot - 1, eot));
            for id in [-1, eot, eot + 1, eot + 10, eot + 106, eot + 1500] {
                assert!(!is_lexical(id, eot));
            }
        }
    }

    #[test]
    fn incomplete_unicode_is_rejected_instead_of_lossily_rewritten() {
        let fragment = c"\xe4\xb8";
        assert!(fragment.to_str().is_err());
    }
}
