//! Orchestration: is this meeting diarizable, and if so, do it.
//!
//! Nothing here runs during capture. ADR-0034 makes this a post-hoc pass over a
//! finished recording, so it can fail without touching a recording in progress
//! (`CLAUDE.md` §4).

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use sqlx::{Row, SqlitePool};
use tauri::{AppHandle, Manager, Runtime};

use crate::context::AuthContext;
use crate::database::repositories::speaker_turn::{SpeakerTurn, SpeakerTurnsRepository};
use crate::diarization::models;

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
    Done { diarized_at: String, turns: usize },
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

/// Report the four states for one meeting.
pub async fn availability(
    pool: &SqlitePool,
    ctx: &AuthContext,
    meeting_id: &str,
    folder_path: Option<&str>,
    models_dir: &Path,
) -> Result<Availability> {
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
        .analyze(&audio, models_dir)
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
    SpeakerTurnsRepository::replace_for_meeting(pool, ctx, meeting_id, &turns).await?;
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
pub fn schedule_offline_diarization<R: Runtime>(
    app: AppHandle<R>,
    pool: SqlitePool,
    ctx: AuthContext,
    meeting_id: String,
) {
    tauri::async_runtime::spawn(async move {
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
                log::warn!(
                    "speaker analysis could not read meeting {}: {}",
                    meeting_id,
                    error
                );
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
            return;
        }
        let _ = set_status(&pool, &ctx, &meeting_id, "running", None).await;
        match crate::diarization::offline::OfflineDiarizationService::default()
            .analyze(&audio, &models_dir)
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
                if let Err(error) =
                    SpeakerTurnsRepository::replace_for_meeting(&pool, &ctx, &meeting_id, &turns)
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
                }
            }
            Err(error) => {
                let _ =
                    set_status(&pool, &ctx, &meeting_id, "failed", Some(&error.to_string())).await;
                log::warn!("speaker analysis failed for {}: {:#}", meeting_id, error);
            }
        }
    });
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
