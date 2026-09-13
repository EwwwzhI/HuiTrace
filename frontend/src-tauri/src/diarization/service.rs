//! Orchestration: is this meeting diarizable, and if so, do it.
//!
//! Nothing here runs during capture. ADR-0034 makes this a post-hoc pass over a
//! finished recording, so it can fail without touching a recording in progress
//! (`CLAUDE.md` §4).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, bail, Context, Result};
use sqlx::{Row, SqlitePool};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::context::AuthContext;
use crate::database::repositories::speaker_turn::{SpeakerTurn, SpeakerTurnsRepository};
use crate::diarization::models;
use crate::diarization::types::AudioSource;

static ACTIVE_JOBS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// The synchronous acknowledgement of a background request. It deliberately
/// says nothing about model/audio availability: those are asynchronous job
/// outcomes emitted through the normal lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiarizationRequestResult {
    Scheduled,
    AlreadyRunning,
}

/// Whether this meeting can be diarized, and what has already happened.
///
/// Four states, because collapsing any two of them makes the UI lie. In
/// particular an empty result and a pass that never ran are different facts:
/// the first is an answer, the second is an offer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum Availability {
    /// No saved audio, so a pass can never run for this meeting. Offering a
    /// button here would be offering something that cannot work.
    NoAudio,
    /// Audio present, models present, never run.
    Ready,
    /// Models are not on disk yet.
    ModelsMissing,
    /// A pass completed. `turns` may be zero, which means it separated nothing.
    Done {
        diarized_at: String,
        turns: usize,
    },
    Queued,
    Running,
    Failed {
        error: Option<String>,
    },
    Unavailable {
        error: Option<String>,
    },
}

/// Where a meeting's recording lives, if it kept one.
///
/// Saved audio is OPTIONAL: `RecordingSaver::start_accumulation(false)` discards
/// every chunk, so "transcripts only" is a supported mode that leaves no file.
/// ADR-0027's retention goal also means audio can disappear later, which is why
/// this is probed rather than stored — a flag would go stale on deletion.
pub fn meeting_audio(folder_path: Option<&str>) -> Option<PathBuf> {
    let folder = PathBuf::from(folder_path?);
    if !folder.is_dir() {
        return None;
    }
    crate::audio::retranscription::find_audio_file(&folder).ok()
}

async fn source_for_meeting(pool: &SqlitePool, ctx: &AuthContext, meeting_id: &str) -> AudioSource {
    let value: Option<String> = sqlx::query_scalar("SELECT audio_source FROM transcripts WHERE meeting_id = ? AND workspace_id = ? AND audio_source IS NOT NULL LIMIT 1")
        .bind(meeting_id).bind(ctx.tenant_id.as_str()).fetch_optional(pool).await.ok().flatten();
    match value.as_deref() {
        Some("imported") => AudioSource::Imported,
        Some("microphone") => AudioSource::Microphone,
        Some("system") => AudioSource::System,
        _ => AudioSource::Mixed,
    }
}

/// Report the four states for one meeting.
pub async fn availability(
    pool: &SqlitePool,
    ctx: &AuthContext,
    meeting_id: &str,
    folder_path: Option<&str>,
    models_dir: &Path,
) -> Result<Availability> {
    let persisted: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT diarization_status, diarization_error FROM meetings WHERE id = ? AND workspace_id = ?",
    )
    .bind(meeting_id)
    .bind(ctx.tenant_id.as_str())
    .fetch_optional(pool)
    .await?;
    if let Some((status, error)) = persisted {
        if status == "running" || status == "queued" {
            let key = format!("{}:{meeting_id}", ctx.tenant_id);
            if !ACTIVE_JOBS
                .get_or_init(|| Mutex::new(HashSet::new()))
                .lock()
                .expect("diarization registry poisoned")
                .contains(&key)
            {
                // An app restart cannot retain a sidecar job. Make the stale
                // durable marker retryable instead of showing Processing forever.
                set_status(
                    pool,
                    ctx,
                    meeting_id,
                    "failed",
                    Some("Speaker analysis interrupted by application restart"),
                )
                .await?;
                return Ok(Availability::Failed {
                    error: Some("Speaker analysis interrupted by application restart".into()),
                });
            }
        }
        match status.as_str() {
            "queued" => return Ok(Availability::Queued),
            "running" => return Ok(Availability::Running),
            "failed" => return Ok(Availability::Failed { error }),
            "unavailable" => return Ok(Availability::Unavailable { error }),
            _ => {}
        }
    }
    if let Some(at) = SpeakerTurnsRepository::diarized_at(pool, ctx, meeting_id).await? {
        let turns = SpeakerTurnsRepository::list_for_meeting(pool, ctx, meeting_id)
            .await?
            .len();
        return Ok(Availability::Done {
            diarized_at: at,
            turns,
        });
    }
    if meeting_audio(folder_path).is_none() {
        return Ok(Availability::NoAudio);
    }
    match models::status(models_dir).await {
        models::ModelStatus::Available(_) => Ok(Availability::Ready),
        // Corrupted is reported as missing on purpose: from the caller's side
        // the remedy is identical (acquire them), and the detail belongs in the
        // log rather than in a status the UI switches on.
        _ => Ok(Availability::ModelsMissing),
    }
}

/// Decode the meeting's audio to what the sidecar accepts and write it beside
/// the caller's temp directory.
///
/// Uses the app's own decoder — the same path retranscription takes — rather
/// than shelling out: it already produces 16 kHz mono with a proper sinc
/// resampler, and the helper refuses anything else instead of resampling.
pub fn prepare_wav(audio: &Path, dest: &Path) -> Result<f64> {
    let decoded = crate::audio::decoder::decode_audio_file(audio)
        .with_context(|| format!("decode {}", audio.display()))?;
    let samples = decoded.to_whisper_format();
    if samples.is_empty() {
        bail!("{} decoded to no audio", audio.display());
    }

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(dest, spec)
        .with_context(|| format!("create {}", dest.display()))?;
    for s in &samples {
        writer.write_sample(*s)?;
    }
    writer.finalize().context("finalize prepared wav")?;
    Ok(samples.len() as f64 / 16_000.0)
}

/// Run one diarization pass and persist it.
///
/// Persists even when the pass separated nothing: the stamp is what
/// distinguishes "ran, inconclusive" from "never ran", and without it the UI
/// would keep offering a pass that has already been tried.
pub async fn diarize_meeting(
    pool: &SqlitePool,
    ctx: &AuthContext,
    meeting_id: &str,
    folder_path: Option<&str>,
    models_dir: &Path,
) -> Result<usize> {
    let audio = meeting_audio(folder_path).ok_or_else(|| {
        anyhow!("this meeting has no saved audio, so speakers cannot be identified")
    })?;
    let segments = crate::diarization::offline::OfflineDiarizationService::default()
        .analyze(&audio, models_dir, AudioSource::Mixed)
        .await?;
    let turns: Vec<SpeakerTurn> = segments
        .into_iter()
        .map(|segment| SpeakerTurn {
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
            speaker_label: display_label_for_key(&segment.speaker_key),
            confidence: segment.speaker_confidence,
            speaker_key: segment.speaker_key,
        })
        .collect();
    SpeakerTurnsRepository::replace_for_meeting_with_source(
        pool,
        ctx,
        meeting_id,
        &turns,
        AudioSource::Mixed,
    )
    .await?;
    if let Some(folder) = folder_path {
        if let Err(error) = sync_transcripts_json(pool, ctx, meeting_id, Path::new(folder)).await {
            log::warn!(
                "speaker transcript JSON sync failed for {}: {:#}",
                meeting_id,
                error
            );
        }
    }
    Ok(turns.len())
}

fn display_label_for_key(key: &str) -> String {
    key.strip_prefix("speaker_")
        .and_then(|number| number.parse::<usize>().ok())
        .map(|number| format!("Speaker {number}"))
        .unwrap_or_else(|| key.to_string())
}

/// Start an enhancement pass without coupling meeting persistence to model
/// availability or sidecar health. ASR and the saved meeting are already
/// complete when this is called; every error becomes diagnosable state instead
/// of a failed recording/import.
pub fn request_offline_diarization<R: Runtime>(
    app: AppHandle<R>,
    pool: SqlitePool,
    ctx: AuthContext,
    meeting_id: String,
) -> DiarizationRequestResult {
    let registry = ACTIVE_JOBS.get_or_init(|| Mutex::new(HashSet::new()));
    let job_key = format!("{}:{meeting_id}", ctx.tenant_id);
    if !registry
        .lock()
        .expect("diarization registry poisoned")
        .insert(job_key.clone())
    {
        return DiarizationRequestResult::AlreadyRunning;
    }
    tauri::async_runtime::spawn(async move {
        let registry = ACTIVE_JOBS.get_or_init(|| Mutex::new(HashSet::new()));
        let _guard = ActiveJobGuard {
            key: job_key,
            registry,
        };
        let emit = |status: &str, error: Option<String>, turns: Option<usize>| {
            let _ = app.emit("diarization-status-changed", serde_json::json!({"meeting_id": meeting_id, "status": status, "error": error, "turns": turns}));
        };
        let _ = set_status(&pool, &ctx, &meeting_id, "queued", None).await;
        emit("queued", None, None);
        let folder: Option<String> = match sqlx::query_scalar(
            "SELECT folder_path FROM meetings WHERE id = ? AND workspace_id = ?",
        )
        .bind(&meeting_id)
        .bind(ctx.tenant_id.as_str())
        .fetch_optional(&pool)
        .await
        {
            Ok(value) => value.flatten(),
            Err(error) => {
                let message = error.to_string();
                let _ = set_status(&pool, &ctx, &meeting_id, "failed", Some(&message)).await;
                log::warn!(
                    "speaker analysis could not read meeting {}: {}",
                    meeting_id,
                    error
                );
                emit("failed", Some(message), None);
                return;
            }
        };
        let Some(audio) = meeting_audio(folder.as_deref()) else {
            let _ = set_status(
                &pool,
                &ctx,
                &meeting_id,
                "unavailable",
                Some("No saved audio available for speaker analysis"),
            )
            .await;
            emit(
                "unavailable",
                Some("No saved audio available for speaker analysis".into()),
                None,
            );
            return;
        };
        let models_dir = match app.path().app_data_dir() {
            Ok(dir) => models::models_dir(&dir),
            Err(error) => {
                let _ = set_status(
                    &pool,
                    &ctx,
                    &meeting_id,
                    "failed",
                    Some(&format!("Resolve diarization model directory: {error}")),
                )
                .await;
                emit("failed", Some(error.to_string()), None);
                return;
            }
        };
        if !matches!(
            models::status(&models_dir).await,
            models::ModelStatus::Available(_)
        ) {
            let _ = set_status(
                &pool,
                &ctx,
                &meeting_id,
                "unavailable",
                Some("Diarization models are not installed"),
            )
            .await;
            emit(
                "unavailable",
                Some("Diarization models are not installed".into()),
                None,
            );
            return;
        }
        let source = source_for_meeting(&pool, &ctx, &meeting_id).await;
        let _ = set_status(&pool, &ctx, &meeting_id, "running", None).await;
        emit("running", None, None);
        match crate::diarization::offline::OfflineDiarizationService::default()
            .analyze(&audio, &models_dir, source.clone())
            .await
        {
            Ok(segments) => {
                let turns: Vec<SpeakerTurn> = segments
                    .into_iter()
                    .map(|segment| SpeakerTurn {
                        start_ms: segment.start_ms,
                        end_ms: segment.end_ms,
                        speaker_label: display_label_for_key(&segment.speaker_key),
                        confidence: segment.speaker_confidence,
                        speaker_key: segment.speaker_key,
                    })
                    .collect();
                if let Err(error) = SpeakerTurnsRepository::replace_for_meeting_with_source(
                    &pool,
                    &ctx,
                    &meeting_id,
                    &turns,
                    source,
                )
                .await
                {
                    let _ =
                        set_status(&pool, &ctx, &meeting_id, "failed", Some(&error.to_string()))
                            .await;
                    log::warn!(
                        "speaker analysis persistence failed for {}: {:#}",
                        meeting_id,
                        error
                    );
                    emit("failed", Some(error.to_string()), None);
                } else if let Some(folder) = folder.as_deref() {
                    if let Err(error) =
                        sync_transcripts_json(&pool, &ctx, &meeting_id, Path::new(folder)).await
                    {
                        log::warn!(
                            "speaker transcript JSON sync failed for {}: {:#}",
                            meeting_id,
                            error
                        );
                    }
                    emit("completed", None, Some(turns.len()));
                } else {
                    emit("completed", None, Some(turns.len()));
                }
            }
            Err(error) => {
                let _ =
                    set_status(&pool, &ctx, &meeting_id, "failed", Some(&error.to_string())).await;
                log::warn!("speaker analysis failed for {}: {:#}", meeting_id, error);
                emit("failed", Some(error.to_string()), None);
            }
        }
    });
    DiarizationRequestResult::Scheduled
}

struct ActiveJobGuard {
    key: String,
    registry: &'static Mutex<HashSet<String>>,
}
impl Drop for ActiveJobGuard {
    fn drop(&mut self) {
        self.registry
            .lock()
            .expect("diarization registry poisoned")
            .remove(&self.key);
    }
}

async fn sync_transcripts_json(
    pool: &SqlitePool,
    ctx: &AuthContext,
    meeting_id: &str,
    folder: &Path,
) -> Result<()> {
    let rows = sqlx::query(
        "SELECT id, transcript, timestamp, audio_start_time, audio_end_time, duration, speaker_id, speaker_confidence, speaker_provisional, speaker_revision, segment_kind, audio_source, speaker_assignment_method, speaker_overlap FROM transcripts WHERE meeting_id = ? AND workspace_id = ? ORDER BY audio_start_time ASC",
    ).bind(meeting_id).bind(ctx.tenant_id.as_str()).fetch_all(pool).await?;
    let segments = rows
        .into_iter()
        .map(|row| crate::api::TranscriptSegment {
            id: row.get("id"),
            text: row.get("transcript"),
            timestamp: row.get("timestamp"),
            audio_start_time: row.get("audio_start_time"),
            audio_end_time: row.get("audio_end_time"),
            duration: row.get("duration"),
            speaker_id: row.get("speaker_id"),
            speaker_confidence: row.get("speaker_confidence"),
            speaker_provisional: Some(row.get::<i64, _>("speaker_provisional") != 0),
            speaker_revision: Some(row.get("speaker_revision")),
            segment_kind: row.get("segment_kind"),
            audio_source: row.get("audio_source"),
            speaker_assignment_method: Some(row.get("speaker_assignment_method")),
            speaker_overlap: Some(row.get::<i64, _>("speaker_overlap") != 0),
        })
        .collect::<Vec<_>>();
    crate::audio::common::write_transcripts_json(folder, &segments)
}

async fn set_status(
    pool: &SqlitePool,
    ctx: &AuthContext,
    meeting_id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<()> {
    sqlx::query("UPDATE meetings SET diarization_status = ?, diarization_error = ? WHERE id = ? AND workspace_id = ?")
        .bind(status).bind(error).bind(meeting_id).bind(ctx.tenant_id.as_str()).execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A transcripts-only meeting has no folder at all.
    #[test]
    fn no_folder_means_no_audio() {
        assert!(meeting_audio(None).is_none());
        assert!(meeting_audio(Some("")).is_none());
        assert!(meeting_audio(Some("C:/definitely/not/a/real/folder")).is_none());
    }

    /// A folder that exists but kept no audio is the "transcripts only" case,
    /// and it must be distinguishable from a folder we simply cannot find.
    #[test]
    fn a_folder_without_audio_is_still_no_audio() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("transcripts.json"), b"{}").expect("write");
        assert!(meeting_audio(dir.path().to_str()).is_none());
    }

    /// The exact JSON the frontend receives. Asserted rather than assumed
    /// because serde's `rename_all` on an enum renames the VARIANTS but leaves
    /// struct-variant fields alone -- so the tag is `modelsMissing` while the
    /// field beside it is `diarized_at`. A UI written to the plausible-looking
    /// `diarizedAt` would render `undefined` and still typecheck.
    #[test]
    fn the_wire_shape_the_ui_is_written_against() {
        assert_eq!(
            serde_json::to_string(&Availability::Done {
                diarized_at: "2026-08-09T10:00:00Z".into(),
                turns: 3,
            })
            .expect("serialize"),
            r#"{"status":"done","diarized_at":"2026-08-09T10:00:00Z","turns":3}"#
        );
        assert_eq!(
            serde_json::to_string(&Availability::NoAudio).expect("serialize"),
            r#"{"status":"noAudio"}"#
        );
        assert_eq!(
            serde_json::to_string(&Availability::Ready).expect("serialize"),
            r#"{"status":"ready"}"#
        );
        assert_eq!(
            serde_json::to_string(&Availability::ModelsMissing).expect("serialize"),
            r#"{"status":"modelsMissing"}"#
        );
    }

    #[test]
    fn a_folder_with_audio_resolves_to_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let audio = dir.path().join("audio.mp4");
        std::fs::write(&audio, b"not really audio, but the probe is by name").expect("write");
        assert_eq!(meeting_audio(dir.path().to_str()), Some(audio));
    }
}
