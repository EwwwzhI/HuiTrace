mod alignment;
mod assembler;
mod boundary;
mod config;
mod normalizer;
mod timing;
mod types;

pub use assembler::reconstruct;
pub use config::UtteranceReconstructionConfig;
pub use normalizer::normalize_timeline;
pub use types::*;

use tauri::State;

use crate::context;
use crate::database::repositories::meeting::MeetingsRepository;
use crate::database::repositories::short_turn_event::ShortTurnEventsRepository;
use crate::database::repositories::speaker_turn::SpeakerTurnsRepository;
use crate::state::AppState;

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
    let v1_spans = normalize_timeline(&transcripts, &speaker_turns, &short_turn_events);
    let enhanced = timing::enhance_timeline(&transcripts, &speaker_turns, &v1_spans, &config);
    if enhanced.valid_timing_chunks == 0 {
        let mut result = reconstruct(&meeting_id, &v1_spans, &config);
        result.metrics = enhanced.metrics;
        result.timing_diagnostics = enhanced.timing_diagnostics;
        return Ok(result);
    }
    Ok(assembler::reconstruct_with_details(
        &meeting_id,
        &enhanced.spans,
        &config,
        ALGORITHM_VERSION_V2,
        enhanced.metrics,
        enhanced.alignment_diagnostics,
        enhanced.timing_diagnostics,
    ))
}

#[cfg(test)]
mod tests;
