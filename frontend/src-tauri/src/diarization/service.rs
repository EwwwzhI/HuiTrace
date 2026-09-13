//! Orchestration: is this meeting diarizable, and if so, do it.
//!
//! Nothing here runs during capture. ADR-0034 makes this a post-hoc pass over a
//! finished recording, so it can fail without touching a recording in progress
//! (`CLAUDE.md` §4).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{anyhow, bail, Context, Result};
use sqlx::{Row, SqlitePool};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::context::AuthContext;
use crate::database::repositories::speaker_turn::{SpeakerTurn, SpeakerTurnsRepository};
use crate::diarization::models;
use crate::diarization::types::AudioSource;

#[cfg(test)]
use crate::diarization::backend::DiarizationBackend;
#[cfg(test)]
use crate::diarization::types::SpeakerSegment;

static ACTIVE_JOBS: OnceLock<DiarizationJobRegistry> = OnceLock::new();

#[derive(Clone, Default)]
struct DiarizationJobRegistry {
    active: Arc<Mutex<HashSet<String>>>,
}

impl DiarizationJobRegistry {
    fn try_start(&self, key: String) -> Option<ActiveJobGuard> {
        if !self
            .active
            .lock()
            .expect("diarization registry poisoned")
            .insert(key.clone())
        {
            return None;
        }
        Some(ActiveJobGuard {
            key,
            registry: self.clone(),
        })
    }

    fn contains(&self, key: &str) -> bool {
        self.active
            .lock()
            .expect("diarization registry poisoned")
            .contains(key)
    }

    fn request(&self, key: String) -> (DiarizationRequestResult, Option<ActiveJobGuard>) {
        match self.try_start(key) {
            Some(guard) => (DiarizationRequestResult::Scheduled, Some(guard)),
            None => (DiarizationRequestResult::AlreadyRunning, None),
        }
    }
}

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
#[serde(rename_all = "camelCase")]
pub struct Availability {
    pub result: Option<DiarizationResult>,
    pub job: DiarizationJobState,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DiarizationResult {
    pub diarized_at: String,
    pub turns: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum DiarizationJobState {
    Idle,
    Queued,
    Running,
    Failed { error: Option<String> },
    Unavailable { error: Option<String> },
    ModelsMissing,
    NoAudio,
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

pub async fn resolve_meeting_audio_source(
    pool: &SqlitePool,
    ctx: &AuthContext,
    meeting_id: &str,
) -> AudioSource {
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
    let result =
        if let Some(at) = SpeakerTurnsRepository::diarized_at(pool, ctx, meeting_id).await? {
            Some(DiarizationResult {
                turns: SpeakerTurnsRepository::list_for_meeting(pool, ctx, meeting_id)
                    .await?
                    .len(),
                diarized_at: at,
            })
        } else {
            None
        };
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
                .get_or_init(DiarizationJobRegistry::default)
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
                return Ok(Availability {
                    result,
                    job: DiarizationJobState::Failed {
                        error: Some("Speaker analysis interrupted by application restart".into()),
                    },
                });
            }
        }
        match status.as_str() {
            "queued" => {
                return Ok(Availability {
                    result,
                    job: DiarizationJobState::Queued,
                })
            }
            "running" => {
                return Ok(Availability {
                    result,
                    job: DiarizationJobState::Running,
                })
            }
            "failed" => {
                return Ok(Availability {
                    result,
                    job: DiarizationJobState::Failed { error },
                })
            }
            "unavailable" => {
                return Ok(Availability {
                    result,
                    job: DiarizationJobState::Unavailable { error },
                })
            }
            _ => {}
        }
    }
    if result.is_some() {
        return Ok(Availability {
            result,
            job: DiarizationJobState::Idle,
        });
    }
    if meeting_audio(folder_path).is_none() {
        return Ok(Availability {
            result,
            job: DiarizationJobState::NoAudio,
        });
    }
    match models::status(models_dir).await {
        models::ModelStatus::Available(_) => Ok(Availability {
            result,
            job: DiarizationJobState::Idle,
        }),
        // Corrupted is reported as missing on purpose: from the caller's side
        // the remedy is identical (acquire them), and the detail belongs in the
        // log rather than in a status the UI switches on.
        _ => Ok(Availability {
            result,
            job: DiarizationJobState::ModelsMissing,
        }),
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

#[cfg(test)]
async fn execute_backend_job<B, F>(
    pool: &SqlitePool,
    ctx: &AuthContext,
    meeting_id: &str,
    prepared_wav: &Path,
    model_paths: &models::ModelPaths,
    source: AudioSource,
    backend: &B,
    mut observe_status: F,
) -> Result<usize>
where
    B: DiarizationBackend,
    F: FnMut(&str),
{
    set_status(pool, ctx, meeting_id, "queued", None).await?;
    observe_status("queued");
    set_status(pool, ctx, meeting_id, "running", None).await?;
    observe_status("running");
    let result = backend.diarize(prepared_wav, model_paths).await;
    match result {
        Ok(segments) => {
            let turns = speaker_turns_from_segments(segments);
            SpeakerTurnsRepository::replace_for_meeting_with_source(
                pool, ctx, meeting_id, &turns, source,
            )
            .await?;
            observe_status("completed");
            Ok(turns.len())
        }
        Err(error) => {
            set_status(pool, ctx, meeting_id, "failed", Some(&error.to_string())).await?;
            observe_status("failed");
            Err(error)
        }
    }
}

#[cfg(test)]
fn speaker_turns_from_segments(segments: Vec<SpeakerSegment>) -> Vec<SpeakerTurn> {
    segments
        .into_iter()
        .map(|segment| SpeakerTurn {
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
            speaker_label: display_label_for_key(&segment.speaker_key),
            confidence: segment.speaker_confidence,
            speaker_key: segment.speaker_key,
        })
        .collect()
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
    let registry = ACTIVE_JOBS.get_or_init(DiarizationJobRegistry::default);
    let job_key = format!("{}:{meeting_id}", ctx.tenant_id);
    let (request_result, guard) = registry.request(job_key.clone());
    let Some(_guard) = guard else {
        return request_result;
    };
    tauri::async_runtime::spawn(async move {
        let _guard = _guard;
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
        let source = resolve_meeting_audio_source(&pool, &ctx, &meeting_id).await;
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
    request_result
}

struct ActiveJobGuard {
    key: String,
    registry: DiarizationJobRegistry,
}
impl Drop for ActiveJobGuard {
    fn drop(&mut self) {
        self.registry
            .active
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
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use sqlx::migrate::Migrator;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use tokio::sync::Semaphore;

    use crate::context::{RequestId, Role, TenantId, UserId};
    use crate::diarization::types::{AssignmentMethod, SegmentKind};

    static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

    #[derive(Clone)]
    enum FakeOutcome {
        Success(Vec<SpeakerSegment>),
        Failure(String),
    }

    #[derive(Clone)]
    struct FakeDiarizationBackend {
        outcomes: Arc<Mutex<VecDeque<FakeOutcome>>>,
        gate: Option<Arc<Semaphore>>,
        delay: Duration,
        call_count: Arc<AtomicUsize>,
    }

    impl FakeDiarizationBackend {
        fn new(outcomes: impl IntoIterator<Item = FakeOutcome>) -> Self {
            Self {
                outcomes: Arc::new(Mutex::new(outcomes.into_iter().collect())),
                gate: None,
                delay: Duration::ZERO,
                call_count: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn gated(mut self, gate: Arc<Semaphore>) -> Self {
            self.gate = Some(gate);
            self
        }

        fn delayed(mut self, delay: Duration) -> Self {
            self.delay = delay;
            self
        }

        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl DiarizationBackend for FakeDiarizationBackend {
        fn name(&self) -> &'static str {
            "fake"
        }

        async fn diarize(
            &self,
            _wav: &Path,
            _models: &models::ModelPaths,
        ) -> Result<Vec<SpeakerSegment>> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            if let Some(gate) = &self.gate {
                gate.acquire().await.expect("gate open").forget();
            }
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            match self
                .outcomes
                .lock()
                .expect("fake outcomes poisoned")
                .pop_front()
                .expect("fake outcome")
            {
                FakeOutcome::Success(segments) => Ok(segments),
                FakeOutcome::Failure(error) => Err(anyhow!(error)),
            }
        }
    }

    fn ctx() -> AuthContext {
        AuthContext {
            tenant_id: TenantId::new("local"),
            user_id: UserId::new("user"),
            roles: vec![Role::Owner],
            request_id: RequestId::generate(),
        }
    }

    async fn test_db(path: &Path) -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(true),
            )
            .await
            .expect("open db");
        MIGRATOR.run(&pool).await.expect("migrate");
        pool
    }

    async fn seed_meeting(pool: &SqlitePool, id: &str) {
        sqlx::query("INSERT INTO meetings (id, workspace_id, title, created_at, updated_at) VALUES (?, 'local', 'Meeting', '2026-09-13T00:00:00Z', '2026-09-13T00:00:00Z')")
            .bind(id)
            .execute(pool)
            .await
            .expect("seed meeting");
    }

    fn segment(start_ms: i64, end_ms: i64, speaker: &str) -> SpeakerSegment {
        SpeakerSegment {
            start_ms,
            end_ms,
            speaker_key: speaker.into(),
            speaker_confidence: Some(0.9),
            audio_source: AudioSource::Mixed,
            provisional: false,
            revision: 1,
            segment_kind: SegmentKind::Speech,
            assignment_method: AssignmentMethod::Diarization,
            overlap: false,
        }
    }

    fn fake_paths(dir: &Path) -> models::ModelPaths {
        models::ModelPaths {
            segmentation: dir.join("segmentation.onnx"),
            embedding: dir.join("embedding.onnx"),
        }
    }

    async fn wait_for_calls(backend: &FakeDiarizationBackend, expected: usize) {
        for _ in 0..100 {
            if backend.calls() >= expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("backend did not reach {expected} calls");
    }

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

    /// Result and latest job are deliberately independent: a failed rerun must
    /// not hide the last successful speaker timeline.
    #[test]
    fn the_wire_shape_the_ui_is_written_against() {
        assert_eq!(
            serde_json::to_string(&Availability {
                result: Some(DiarizationResult {
                    diarized_at: "2026-08-09T10:00:00Z".into(),
                    turns: 3
                }),
                job: DiarizationJobState::Failed {
                    error: Some("boom".into())
                }
            })
            .expect("serialize"),
            r#"{"result":{"diarized_at":"2026-08-09T10:00:00Z","turns":3},"job":{"status":"failed","error":"boom"}}"#
        );
    }

    #[test]
    fn a_folder_with_audio_resolves_to_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let audio = dir.path().join("audio.mp4");
        std::fs::write(&audio, b"not really audio, but the probe is by name").expect("write");
        assert_eq!(meeting_audio(dir.path().to_str()), Some(audio));
    }

    #[tokio::test]
    async fn same_meeting_double_request_is_scheduled_then_already_running() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = test_db(&dir.path().join("lifecycle.db")).await;
        seed_meeting(&pool, "meeting-a").await;
        let gate = Arc::new(Semaphore::new(0));
        let backend = Arc::new(
            FakeDiarizationBackend::new([FakeOutcome::Success(vec![segment(
                0,
                2_000,
                "speaker_01",
            )])])
            .gated(gate.clone()),
        );
        let registry = DiarizationJobRegistry::default();
        let key = "local:meeting-a".to_string();
        let (first_result, guard) = registry.request(key.clone());
        assert_eq!(first_result, DiarizationRequestResult::Scheduled);
        let guard = guard.expect("scheduled guard");
        let pool_for_job = pool.clone();
        let ctx_for_job = ctx();
        let paths = fake_paths(dir.path());
        let wav = dir.path().join("prepared.wav");
        let backend_for_job = backend.clone();
        let job = tokio::spawn(async move {
            let _guard = guard;
            execute_backend_job(
                &pool_for_job,
                &ctx_for_job,
                "meeting-a",
                &wav,
                &paths,
                AudioSource::Mixed,
                backend_for_job.as_ref(),
                |_| {},
            )
            .await
        });
        wait_for_calls(&backend, 1).await;
        let (second_result, second_guard) = registry.request(key);
        assert_eq!(second_result, DiarizationRequestResult::AlreadyRunning);
        assert!(second_guard.is_none());
        gate.add_permits(1);
        assert_eq!(job.await.expect("join").expect("success"), 1);
        assert_eq!(backend.calls(), 1);
    }

    #[tokio::test]
    async fn different_meetings_execute_independently() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = test_db(&dir.path().join("independent.db")).await;
        seed_meeting(&pool, "meeting-a").await;
        seed_meeting(&pool, "meeting-b").await;
        let gate = Arc::new(Semaphore::new(0));
        let backend = Arc::new(
            FakeDiarizationBackend::new([
                FakeOutcome::Success(vec![segment(0, 2_000, "speaker_01")]),
                FakeOutcome::Success(vec![segment(0, 2_000, "speaker_01")]),
            ])
            .gated(gate.clone())
            .delayed(Duration::from_millis(1)),
        );
        let registry = DiarizationJobRegistry::default();
        let mut jobs = Vec::new();
        for meeting in ["meeting-a", "meeting-b"] {
            let guard = registry
                .try_start(format!("local:{meeting}"))
                .expect("independent scheduled");
            let pool = pool.clone();
            let backend = backend.clone();
            let paths = fake_paths(dir.path());
            let wav = dir.path().join(format!("{meeting}.wav"));
            jobs.push(tokio::spawn(async move {
                let _guard = guard;
                execute_backend_job(
                    &pool,
                    &ctx(),
                    meeting,
                    &wav,
                    &paths,
                    AudioSource::Mixed,
                    backend.as_ref(),
                    |_| {},
                )
                .await
            }));
        }
        wait_for_calls(&backend, 2).await;
        gate.add_permits(2);
        for job in jobs {
            assert_eq!(job.await.expect("join").expect("success"), 1);
        }
        assert_eq!(backend.calls(), 2);
    }

    #[tokio::test]
    async fn backend_success_moves_queued_running_completed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = test_db(&dir.path().join("success.db")).await;
        seed_meeting(&pool, "meeting-a").await;
        let backend = FakeDiarizationBackend::new([FakeOutcome::Success(vec![segment(
            0,
            2_000,
            "speaker_01",
        )])]);
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let observed = statuses.clone();
        execute_backend_job(
            &pool,
            &ctx(),
            "meeting-a",
            &dir.path().join("prepared.wav"),
            &fake_paths(dir.path()),
            AudioSource::Mixed,
            &backend,
            move |status| observed.lock().expect("statuses").push(status.to_string()),
        )
        .await
        .expect("success");
        assert_eq!(
            *statuses.lock().expect("statuses"),
            ["queued", "running", "completed"]
        );
    }

    #[tokio::test]
    async fn failure_preserves_previous_success_and_retry_can_complete() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = test_db(&dir.path().join("retry.db")).await;
        seed_meeting(&pool, "meeting-a").await;
        let backend = FakeDiarizationBackend::new([
            FakeOutcome::Success(vec![segment(0, 2_000, "speaker_01")]),
            FakeOutcome::Failure("synthetic backend failure".into()),
            FakeOutcome::Success(vec![segment(0, 2_000, "speaker_02")]),
        ]);
        let wav = dir.path().join("prepared.wav");
        let paths = fake_paths(dir.path());
        execute_backend_job(
            &pool,
            &ctx(),
            "meeting-a",
            &wav,
            &paths,
            AudioSource::Mixed,
            &backend,
            |_| {},
        )
        .await
        .expect("first success");
        let before = SpeakerTurnsRepository::list_for_meeting(&pool, &ctx(), "meeting-a")
            .await
            .expect("previous result");
        assert!(execute_backend_job(
            &pool,
            &ctx(),
            "meeting-a",
            &wav,
            &paths,
            AudioSource::Mixed,
            &backend,
            |_| {}
        )
        .await
        .is_err());
        let after_failure = SpeakerTurnsRepository::list_for_meeting(&pool, &ctx(), "meeting-a")
            .await
            .expect("preserved result");
        assert_eq!(after_failure, before);
        let status: String =
            sqlx::query_scalar("SELECT diarization_status FROM meetings WHERE id = 'meeting-a'")
                .fetch_one(&pool)
                .await
                .expect("failed status");
        assert_eq!(status, "failed");

        assert_eq!(
            execute_backend_job(
                &pool,
                &ctx(),
                "meeting-a",
                &wav,
                &paths,
                AudioSource::Mixed,
                &backend,
                |_| {}
            )
            .await
            .expect("retry success"),
            1
        );
        let status: String =
            sqlx::query_scalar("SELECT diarization_status FROM meetings WHERE id = 'meeting-a'")
                .fetch_one(&pool)
                .await
                .expect("completed status");
        assert_eq!(status, "completed");
        assert_eq!(backend.calls(), 3);
    }
}
