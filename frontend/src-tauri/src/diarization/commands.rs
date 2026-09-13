//! Tauri surface for speaker diarization.
//!
//! Thin by design: these resolve identity and paths and delegate. The decisions
//! — is this meeting diarizable, what does an empty result mean, what gets
//! written — live in [`super::service`], where they are testable without a
//! running app.
//!
//! Nothing here runs during capture (`CLAUDE.md` §4). Identity comes from
//! `crate::context::current()`, never from the frontend (`docs/CONTRACTS.md`).

use tauri::{AppHandle, Manager, Runtime};

use crate::diarization::{models, service};
use crate::state::AppState;
use crate::{log_error, log_info};

async fn folder_path_for(
    pool: &sqlx::SqlitePool,
    ctx: &crate::context::AuthContext,
    meeting_id: &str,
) -> Result<Option<String>, String> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT folder_path FROM meetings WHERE id = ? AND workspace_id = ?",
    )
    .bind(meeting_id)
    .bind(ctx.tenant_id.as_str())
    .fetch_optional(pool)
    .await
    .map(Option::flatten)
    .map_err(|e| format!("read meeting: {e}"))
}

fn models_dir<R: Runtime>(app: &AppHandle<R>) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("resolve app data directory: {e}"))?;
    Ok(models::models_dir(&dir))
}

/// Whether this meeting can be diarized, and what has already happened.
///
/// Four distinct states, so the UI can offer a pass, report one that found
/// nothing, or explain that a transcripts-only recording has no audio to work
/// from — rather than collapsing them into one ambiguous message.
#[tauri::command]
pub async fn api_diarization_availability<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<service::Availability, String> {
    let pool = state.db_manager.pool();
    let ctx = crate::context::current();
    let folder = folder_path_for(pool, &ctx, &meeting_id).await?;
    let dir = models_dir(&app)?;

    service::availability(pool, &ctx, &meeting_id, folder.as_deref(), &dir)
        .await
        .map_err(|e| {
            log_error!("diarization availability failed: {e:#}");
            format!("{e:#}")
        })
}

/// Download and verify the diarization models.
///
/// Separate from running a pass on purpose: this is the only part that touches
/// the network, so a user can be asked before ~34 MB is fetched, and a machine
/// that already has them never reaches out at all.
#[tauri::command]
pub async fn api_diarization_download_models<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let dir = models_dir(&app)?;
    models::ensure(&dir, |label, done, total| {
        if total > 0 && done == total {
            log_info!("diarization model fetched: {label} ({done} bytes)");
        }
    })
    .await
    .map(|_| ())
    .map_err(|e| {
        log_error!("diarization model download failed: {e:#}");
        format!("{e:#}")
    })
}

/// Run one post-hoc diarization pass and store the result.
///
/// Returns how many turns were stored. **Zero is a success**, not a failure: it
/// means the pass ran and separated nothing, and `diarized_at` now records that
/// so the UI stops offering a pass that has already been tried.
#[tauri::command]
pub async fn api_diarize_meeting<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<service::DiarizationRequestResult, String> {
    log_info!("api_diarize_meeting called");

    let pool = state.db_manager.pool();
    let ctx = crate::context::current();
    Ok(service::request_offline_diarization(
        app,
        pool.clone(),
        ctx,
        meeting_id,
    ))
}

/// This meeting's stored speaker turns.
#[tauri::command]
pub async fn api_get_speaker_turns(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<crate::database::repositories::speaker_turn::SpeakerTurn>, String> {
    let ctx = crate::context::current();
    crate::database::repositories::speaker_turn::SpeakerTurnsRepository::list_for_meeting(
        state.db_manager.pool(),
        &ctx,
        &meeting_id,
    )
    .await
    .map_err(|e| format!("{e:#}"))
}

/// List the meeting-local speakers. Keys stay stable when a display name is
/// edited; this endpoint never attempts voice identification.
#[tauri::command]
pub async fn api_get_meeting_speakers(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<crate::database::repositories::speaker_turn::SpeakerProfile>, String> {
    let ctx = crate::context::current();
    crate::database::repositories::speaker_turn::SpeakerTurnsRepository::list_speakers(
        state.db_manager.pool(),
        &ctx,
        &meeting_id,
    )
    .await
    .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn api_rename_meeting_speaker(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    speaker_key: String,
    display_name: String,
) -> Result<(), String> {
    let ctx = crate::context::current();
    crate::database::repositories::speaker_turn::SpeakerTurnsRepository::rename_speaker(
        state.db_manager.pool(),
        &ctx,
        &meeting_id,
        &speaker_key,
        &display_name,
    )
    .await
    .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn api_assign_transcript_speaker(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    transcript_id: String,
    speaker_key: Option<String>,
) -> Result<(), String> {
    let ctx = crate::context::current();
    let pool = state.db_manager.pool();
    let result = if let Some(key) = speaker_key {
        let valid: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM speakers WHERE meeting_id = ? AND workspace_id = ? AND speaker_key = ?",
        )
        .bind(&meeting_id)
        .bind(ctx.tenant_id.as_str())
        .bind(&key)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("validate speaker: {e}"))?;
        if valid.is_none() {
            return Err("speaker does not belong to this meeting".into());
        }
        sqlx::query("UPDATE transcripts SET speaker_id = ?, speaker_assignment_method = 'manual', speaker_provisional = 0 WHERE id = ? AND meeting_id = ? AND workspace_id = ?")
            .bind(key).bind(transcript_id).bind(meeting_id).bind(ctx.tenant_id.as_str()).execute(pool).await
    } else {
        return api_restore_transcript_speaker_assignment(state, meeting_id, transcript_id).await;
    };
    let result = result.map_err(|e| format!("{e:#}"))?;
    if result.rows_affected() != 1 {
        return Err("transcript does not belong to this meeting".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn api_restore_transcript_speaker_assignment(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    transcript_id: String,
) -> Result<(), String> {
    use crate::diarization::types::{
        AssignmentMethod, AudioSource, SegmentKind, SpeakerSegment, TranscriptTiming,
    };
    use sqlx::Row;
    let ctx = crate::context::current();
    let pool = state.db_manager.pool();
    let row = sqlx::query("SELECT transcript, audio_start_time, audio_end_time, COALESCE(audio_source, 'mixed') AS audio_source FROM transcripts WHERE id = ? AND meeting_id = ? AND workspace_id = ?")
        .bind(&transcript_id).bind(&meeting_id).bind(ctx.tenant_id.as_str()).fetch_optional(pool).await.map_err(|e| format!("read transcript: {e}"))?.ok_or("transcript does not belong to this meeting")?;
    let text: String = row.get("transcript");
    let start: Option<f64> = row.get("audio_start_time");
    let end: Option<f64> = row.get("audio_end_time");
    let source = match row.get::<String, _>("audio_source").as_str() {
        "imported" => AudioSource::Imported,
        "microphone" => AudioSource::Microphone,
        "system" => AudioSource::System,
        _ => AudioSource::Mixed,
    };
    let (Some(start), Some(end)) = (start, end) else {
        return Err("transcript has no timing for automatic assignment".into());
    };
    let turns =
        crate::database::repositories::speaker_turn::SpeakerTurnsRepository::list_for_meeting(
            pool,
            &ctx,
            &meeting_id,
        )
        .await
        .map_err(|e| format!("read speakers: {e:#}"))?;
    let speakers = turns
        .iter()
        .map(|t| SpeakerSegment {
            start_ms: t.start_ms,
            end_ms: t.end_ms,
            speaker_key: t.speaker_key.clone(),
            speaker_confidence: t.confidence,
            audio_source: source.clone(),
            provisional: false,
            revision: 1,
            segment_kind: SegmentKind::Speech,
            assignment_method: AssignmentMethod::Diarization,
            overlap: false,
        })
        .collect::<Vec<_>>();
    let timing = TranscriptTiming {
        id: transcript_id.clone(),
        start_ms: (start * 1000.0).round() as i64,
        end_ms: (end * 1000.0).round() as i64,
        audio_source: source,
    };
    let assignment = crate::diarization::timeline::reconcile_transcript(
        std::slice::from_ref(&timing),
        &speakers,
    )
    .pop()
    .expect("one assignment");
    let profiles =
        crate::database::repositories::speaker_turn::SpeakerTurnsRepository::list_speakers(
            pool,
            &ctx,
            &meeting_id,
        )
        .await
        .map_err(|e| format!("read speaker profiles: {e:#}"))?;
    let prototypes = crate::diarization::short_turn::MeetingSpeakerPrototypeStore::new(
        profiles.into_iter().map(|profile| profile.speaker_key),
    );
    let assignment = crate::diarization::short_turn::refine_timeline_assignment(
        &crate::diarization::short_turn::ShortTurnRefiner::default(),
        &prototypes,
        &timing,
        &text,
        None,
        assignment,
        &speakers,
    );
    let result = sqlx::query("UPDATE transcripts SET speaker_id = ?, speaker_confidence = ?, speaker_provisional = ?, speaker_revision = ?, segment_kind = ?, audio_source = ?, speaker_assignment_method = ?, speaker_overlap = ? WHERE id = ? AND meeting_id = ? AND workspace_id = ?")
        .bind(assignment.speaker_key).bind(assignment.speaker_confidence).bind(assignment.speaker_provisional as i64).bind(assignment.speaker_revision).bind(assignment.segment_kind.as_str()).bind(assignment.audio_source.as_str()).bind(assignment.assignment_method.as_str()).bind(assignment.overlap as i64).bind(&transcript_id).bind(&meeting_id).bind(ctx.tenant_id.as_str()).execute(pool).await.map_err(|e| format!("restore assignment: {e}"))?;
    if result.rows_affected() != 1 {
        return Err("transcript does not belong to this meeting".into());
    }
    Ok(())
}
