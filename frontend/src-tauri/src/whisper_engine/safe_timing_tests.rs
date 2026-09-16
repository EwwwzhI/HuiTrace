// Frozen pre-change text path, copied from the parent revision. Keep independent
// of metadata extraction so the audio regression compares against a real baseline.
use super::*;
impl WhisperEngine {
    async fn safe_text_baseline(
        &self,
        audio_data: Vec<f32>,
        language: Option<String>,
    ) -> Result<(String, f32, bool)> {
        let ctx_lock = self.current_context.read().await;
        let ctx = ctx_lock
            .as_ref()
            .ok_or_else(|| anyhow!("No model loaded. Please load a model first."))?;

        // Get adaptive configuration based on hardware
        let hardware_profile = crate::audio::HardwareProfile::detect();
        let adaptive_config = hardware_profile.get_whisper_config();

        // ADAPTIVE parameters - optimized for current hardware
        let mut params = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: adaptive_config.beam_size as i32,
            patience: 1.0,
        });

        // Configure with adaptive settings
        // If language is "auto" or None, use automatic language detection (pass None)
        // If language is "auto-translate", enable translation to English
        // Otherwise, use the specified language code
        let (language_code, should_translate) = match language.as_deref() {
            Some("auto") | None => (None, false),
            Some("auto-translate") => (None, true),
            Some(lang) => (Some(lang), false),
        };
        params.set_language(language_code);
        params.set_translate(should_translate);

        // CRITICAL: Disable timestamp tokens to prevent whisper.cpp chunking heuristics
        // The "single timestamp ending - skip entire chunk" optimization incorrectly discards
        // complete, valid transcriptions. Disabling timestamps forces whisper to return ALL text.
        params.set_no_timestamps(true); // Prevent timestamp-based segment skipping
        params.set_token_timestamps(true); // Keep for any timestamp-aware features

        // PERFORMANCE: Disable ALL whisper.cpp internal printing
        // This reduces C library log spam significantly
        params.set_print_special(false); // Don't print special tokens
        params.set_print_progress(false); // Don't print progress
        params.set_print_realtime(false); // Don't print realtime info
        params.set_print_timestamps(false); // Don't print timestamps

        // Additional suppression to reduce C library verbosity
        params.set_suppress_blank(true);
        params.set_suppress_non_speech_tokens(true);
        params.set_temperature(adaptive_config.temperature);
        params.set_max_initial_ts(1.0);
        params.set_entropy_thold(2.4);
        params.set_logprob_thold(-1.0);
        // BALANCED FIX: Lowered from 0.75 to 0.55 to allow quiet speech detection
        // Previous value was too aggressive and rejected valid quiet speech
        // 0.55 is balanced - prevents hallucinations while preserving quiet speech
        params.set_no_speech_thold(0.55);
        params.set_max_len(200);
        params.set_single_segment(false);

        // Set thread count based on hardware (if supported by whisper.cpp)
        if let Some(_max_threads) = adaptive_config.max_threads {
            // Note: whisper.cpp may or may not expose thread control through params
            // Removed debug log to reduce I/O overhead in transcription hot path
        }

        let duration_seconds = audio_data.len() as f64 / 16000.0;
        let is_partial = duration_seconds < 15.0; // Consider chunks under 15s as partial

        // PERFORMANCE: Suppress verbose C library logs during transcription
        // This hides whisper_full_with_state debug logs and beam search details
        let (num_segments, state) = {
            // let _suppressor = crate::whisper_engine::StderrSuppressor::new();

            let mut state = ctx.create_state()?;
            state.full(params, &audio_data)?;
            let num_segments = state.full_n_segments();

            (num_segments, state)
            // Suppressor dropped here, stderr restored
        };
        let mut result = String::new();
        let mut total_confidence = 0.0;
        let mut segment_count = 0;

        let num_segments = num_segments?;
        for i in 0..num_segments {
            let segment_text = match state.full_get_segment_text_lossy(i) {
                Ok(text) => text,
                Err(_) => continue,
            };

            // Calculate confidence based on segment length and duration (simplified approach)
            let segment_length = segment_text.len() as f32;
            let segment_confidence = if segment_length > 0.0 {
                (segment_length / 100.0).min(0.9) + 0.1 // 0.1 to 1.0 confidence based on text length
            } else {
                0.1
            };
            total_confidence += segment_confidence;
            segment_count += 1;

            let cleaned_text = segment_text.trim();
            if !cleaned_text.is_empty() {
                if !result.is_empty() {
                    result.push(' ');
                }
                result.push_str(cleaned_text);
            }
        }

        let final_result = result.trim().to_string();
        let cleaned_result = Self::clean_repetitive_text(&final_result);

        let avg_confidence = if segment_count > 0 {
            total_confidence / segment_count as f32
        } else {
            0.0
        };

        Ok((cleaned_result, avg_confidence, is_partial))
    }
}

#[tokio::test]
#[ignore = "requires WHISPER_TEST_MODELS_DIR with pinned ggml-small.bin (see fixture README)"]
async fn e978c55_audio_preserves_safe_baseline_and_tail() {
    let directory =
        std::env::var_os("WHISPER_TEST_MODELS_DIR").expect("WHISPER_TEST_MODELS_DIR required");
    let engine = WhisperEngine::new_with_models_dir(Some(directory.into())).unwrap();
    engine.discover_models().await.unwrap();
    engine.load_model("small").await.unwrap();
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/whisper/jfk.wav");
    let mut reader = hound::WavReader::open(path).unwrap();
    assert_eq!(reader.spec().sample_rate, 16000);
    assert_eq!(reader.spec().channels, 1);
    let audio: Vec<f32> = reader
        .samples::<i16>()
        .map(|v| f32::from(v.unwrap()) / 32768.0)
        .collect();
    let baseline = engine
        .safe_text_baseline(audio.clone(), Some("en".into()))
        .await
        .unwrap();
    let native = engine
        .transcribe_audio_with_timing(audio.clone(), Some("en".into()))
        .await
        .unwrap();
    assert_eq!(native.text, baseline.0);
    assert_eq!(native.confidence, baseline.1);
    assert_eq!(native.is_partial, baseline.2);
    let words = native
        .text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>();
    assert!(
        words.contains("ask not what your country can do for you"),
        "{words}"
    );
    assert!(
        words.ends_with("ask what you can do for your country"),
        "{words}"
    );
    let result = crate::audio::transcription::whisper_provider::transcript_result_from_native(
        native,
        (audio.len() / 16) as i64,
    );
    assert_eq!(result.text, baseline.0);
    assert!(
        result.timing.is_some(),
        "fixture should exercise validated native timing, not just fallback"
    );
}

#[test]
fn e978c55_decoder_policy_is_locked_in_both_production_paths() {
    let source = include_str!("whisper_engine.rs");
    assert_eq!(source.matches("params.set_no_timestamps(true)").count(), 2);
    assert_eq!(
        source.matches("params.set_token_timestamps(true)").count(),
        2
    );
    assert!(!source.contains("set_no_timestamps(false)"));
}
