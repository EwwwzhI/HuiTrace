mod alignment;
mod assembler;
mod boundary;
mod config;
mod normalizer;
mod semantic;
mod timing;
pub(crate) use timing::validate_lexical_timing;
mod types;

pub use assembler::reconstruct;
pub use config::{UtteranceReconstructionConfig, CONFIG_VERSION, FROZEN_V1_CONFIG_VERSION};
pub use normalizer::{normalize_timeline_v1_frozen, normalize_timeline_v3};
pub use types::*;

use tauri::State;

use crate::context;
use crate::database::models::Transcript;
use crate::database::repositories::meeting::MeetingsRepository;
use crate::database::repositories::short_turn_event::ShortTurnEventsRepository;
use crate::database::repositories::speaker_turn::{SpeakerTurn, SpeakerTurnsRepository};
use crate::diarization::short_turn_event::ShortTurnEvent;
use crate::state::AppState;

pub fn reconstruct_v1_with_config(
    meeting_id: &str,
    transcripts: &[Transcript],
    speaker_turns: &[SpeakerTurn],
    short_turn_events: &[ShortTurnEvent],
    _config: &UtteranceReconstructionConfig,
) -> ReconstructionResult {
    let frozen_config = UtteranceReconstructionConfig::frozen_v1();
    let spans = normalize_timeline_v1_frozen(transcripts, speaker_turns, short_turn_events);
    reconstruct(meeting_id, &spans, &frozen_config)
}

pub fn reconstruct_v3_with_config(
    meeting_id: &str,
    transcripts: &[Transcript],
    speaker_turns: &[SpeakerTurn],
    short_turn_events: &[ShortTurnEvent],
    config: &UtteranceReconstructionConfig,
) -> ReconstructionResult {
    let chunk_spans = normalize_timeline_v3(transcripts, speaker_turns, short_turn_events, config);
    let enhanced = timing::enhance_timeline(transcripts, speaker_turns, &chunk_spans, config);
    let timing_mode = if enhanced.valid_timing_chunks == 0 {
        ReconstructionTimingMode::ChunkFallback
    } else if enhanced.metrics.chunk_fallback_count == 0 {
        ReconstructionTimingMode::NativeLexicalTiming
    } else {
        ReconstructionTimingMode::Hybrid
    };
    let spans = if enhanced.valid_timing_chunks == 0 {
        &chunk_spans
    } else {
        &enhanced.spans
    };
    assembler::reconstruct_with_details(
        meeting_id,
        spans,
        config,
        ReconstructionProfile {
            algorithm_version: ALGORITHM_VERSION_V3.to_string(),
            timing_mode,
            boundary_policy: BoundaryPolicy::V3SemanticBaseline,
            boundary_policy_version: BOUNDARY_POLICY_VERSION_V3.to_string(),
            semantic_model_version: config
                .semantic_boundary_enabled
                .then(|| SEMANTIC_MODEL_VERSION_V1.to_string()),
            alignment_version: matches!(
                timing_mode,
                ReconstructionTimingMode::NativeLexicalTiming | ReconstructionTimingMode::Hybrid
            )
            .then(|| ALIGNMENT_VERSION_V1.to_string()),
        },
        enhanced.metrics,
        enhanced.alignment_diagnostics,
        enhanced.timing_diagnostics,
    )
}

/// Derived-on-read reconstruction. Raw transcript rows remain the evidence source of truth.
#[tauri::command]
pub async fn api_get_reconstructed_utterances(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<ReconstructionResult, String> {
    let ctx = context::current();
    let pool = state.db_manager.pool();
    let (transcripts, speaker_turns, short_turn_events) = tokio::join!(
        MeetingsRepository::get_all_meeting_transcripts(pool, &ctx, &meeting_id),
        SpeakerTurnsRepository::list_accepted_turns_for_meeting(pool, &ctx, &meeting_id),
        ShortTurnEventsRepository::list_for_meeting(pool, &ctx, &meeting_id),
    );
    let transcripts =
        transcripts.map_err(|error| format!("load raw transcripts for reconstruction: {error}"))?;
    let speaker_turns = speaker_turns
        .map_err(|error| format!("load speaker turns for reconstruction: {error:#}"))?;
    let short_turn_events = short_turn_events
        .map_err(|error| format!("load short-turn events for reconstruction: {error:#}"))?;
    let config = UtteranceReconstructionConfig::default();
    Ok(reconstruct_v3_with_config(
        &meeting_id,
        &transcripts,
        &speaker_turns,
        &short_turn_events,
        &config,
    ))
}

#[cfg(test)]
mod tests;
