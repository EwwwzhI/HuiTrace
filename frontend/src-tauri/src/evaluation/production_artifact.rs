//! Versioned, privacy-minimised snapshots of a completed production meeting run.
//!
//! The builder only reads persisted application state. It deliberately has no
//! ASR, diarization, VAD, or short-turn inference dependency, so exporting an
//! artifact cannot create a second inference architecture.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use crate::context::AuthContext;
use crate::diarization::short_turn::{
    CandidateMatchConfig, ShortCandidateVadConfig, ShortTurnConfig, SpeakerAcceptancePolicy,
    VadEventCandidateInput,
};
use crate::diarization::short_turn_event::ShortTurnMaterializationPolicy;

pub const ARTIFACT_SCHEMA_VERSION: u32 = 2;
pub const PHASE_2C1_FROZEN_BASELINE_COMMIT: &str = "7b34d7b7d422deeafeb21f07449f8a3ca8c5f60a";

pub fn app_commit_sha() -> &'static str {
    option_env!("HUITRACE_GIT_COMMIT_SHA").unwrap_or("unknown-source-tree")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactTranscript {
    pub id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub asr_confidence: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactDiarizerTurn {
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker_key: String,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub overlap: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactVadEvent {
    pub start_ms: i64,
    pub end_ms: i64,
    #[serde(default)]
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactBackend {
    pub backend: String,
    pub model: String,
    #[serde(default)]
    pub version_or_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceAudioMetadata {
    #[serde(default)]
    pub path_hint: Option<String>,
    pub duration_ms: i64,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductionConfigSnapshot {
    pub short_turn: ShortTurnConfig,
    pub short_candidate_vad: ShortCandidateVadConfig,
    pub speaker_acceptance: SpeakerAcceptancePolicy,
    pub candidate_match: CandidateMatchConfig,
    pub materialization: ShortTurnMaterializationPolicy,
    pub vad_implementation: String,
    pub vad_runtime_config: serde_json::Value,
}

impl Default for ProductionConfigSnapshot {
    fn default() -> Self {
        Self {
            short_turn: ShortTurnConfig::default(),
            short_candidate_vad: ShortCandidateVadConfig::default(),
            speaker_acceptance: SpeakerAcceptancePolicy::default(),
            candidate_match: CandidateMatchConfig::default(),
            materialization: ShortTurnMaterializationPolicy::default(),
            vad_implementation: "silero_rs::ContinuousVadProcessor".into(),
            vad_runtime_config: serde_json::json!({
                "sample_rate_hz": 16000,
                "positive_speech_threshold": 0.50,
                "negative_speech_threshold": 0.35
            }),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProductionSafetyObservations {
    /// `None` means the production run did not instrument this invariant.
    #[serde(default)]
    pub long_transcript_speaker_corruption_count: Option<usize>,
    /// `None` means the production run did not instrument this invariant.
    #[serde(default)]
    pub manual_override_violation_count: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeetingProductionArtifact {
    pub schema_version: u32,
    pub artifact_id: String,
    pub transcription_run_id: String,
    pub meeting_id: String,
    pub source_audio: SourceAudioMetadata,
    pub created_at: String,
    pub app_commit_sha: String,
    pub asr: ArtifactBackend,
    pub diarization: ArtifactBackend,
    pub production_config: ProductionConfigSnapshot,
    pub transcripts: Vec<ArtifactTranscript>,
    pub raw_diarizer_turns: Vec<ArtifactDiarizerTurn>,
    pub vad_events: Vec<ArtifactVadEvent>,
    pub accepted_speakers: Vec<String>,
    pub visible_speakers: Vec<String>,
    #[serde(default)]
    pub safety_observations: ProductionSafetyObservations,
    #[serde(default)]
    pub production_metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionArtifactInspection {
    pub meeting_id: String,
    pub artifact_id: String,
    pub schema_version: u32,
    pub transcription_run_id: String,
    pub duration_ms: i64,
    pub asr_backend: String,
    pub asr_model: String,
    pub diarization_backend: String,
    pub diarization_model: String,
    pub transcript_count: usize,
    pub diarizer_turn_count: usize,
    pub vad_event_count: usize,
}

fn valid_confidence(value: Option<f64>) -> bool {
    value.map_or(true, |v| v.is_finite() && (0.0..=1.0).contains(&v))
}

fn valid_commit_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_version_or_hash(value: Option<&str>) -> bool {
    let Some(value) = value else {
        return true;
    };
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    let hash = value.strip_prefix("sha256:").unwrap_or(value);
    hash.len() != 64 || hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn validate_production_config(config: &ProductionConfigSnapshot) -> Result<()> {
    let short = &config.short_turn;
    let confidence = |value: f64| value.is_finite() && (0.0..=1.0).contains(&value);
    if short.min_candidate_ms == 0
        || short.very_short_ms == 0
        || short.max_short_turn_ms == 0
        || short.prototype_min_duration_ms == 0
        || short.duration_weight_20_until_ms == 0
        || short.duration_weight_35_until_ms == 0
        || short.duration_weight_55_until_ms == 0
        || short.nearby_gap_ms == 0
        || short.min_candidate_ms > short.very_short_ms
        || short.very_short_ms > short.max_short_turn_ms
        || short.duration_weight_20_until_ms > short.duration_weight_35_until_ms
        || short.duration_weight_35_until_ms > short.duration_weight_55_until_ms
        || !confidence(short.high_confidence_threshold)
        || !confidence(short.weak_confidence_threshold)
        || short.weak_confidence_threshold > short.high_confidence_threshold
    {
        bail!("invalid short-turn config ranges or thresholds");
    }
    let candidate_vad = &config.short_candidate_vad;
    if candidate_vad.min_speech_ms == 0
        || candidate_vad.redemption_ms == 0
        || candidate_vad.max_candidate_ms == 0
        || candidate_vad.min_speech_ms > candidate_vad.max_candidate_ms
    {
        bail!("invalid short-candidate VAD config");
    }
    let acceptance = &config.speaker_acceptance;
    if acceptance.config != config.short_turn
        || !confidence(acceptance.max_overlap_ratio)
        || acceptance.confident_min_longest_turn_ms == 0
        || acceptance.missing_confidence_min_total_ms == 0
        || acceptance.missing_confidence_min_turns == 0
    {
        bail!("invalid or inconsistent speaker-acceptance config");
    }
    let matching = &config.candidate_match;
    if !confidence(matching.min_iou)
        || !confidence(matching.min_ground_truth_coverage)
        || matching.center_tolerance_ms < 0
    {
        bail!("invalid candidate-matching config");
    }
    if !confidence(config.materialization.aligned_min_iou)
        || !confidence(config.materialization.strong_internal_evidence)
        || config.vad_implementation.trim().is_empty()
        || !config.vad_runtime_config.is_object()
    {
        bail!("invalid materialization or VAD runtime config");
    }
    Ok(())
}

pub fn validate_artifact(
    artifact: &MeetingProductionArtifact,
    expected_meeting: Option<&str>,
) -> Result<()> {
    if artifact.schema_version != ARTIFACT_SCHEMA_VERSION {
        bail!(
            "artifact schema {} is unsupported; expected {}",
            artifact.schema_version,
            ARTIFACT_SCHEMA_VERSION
        );
    }
    if artifact.artifact_id.trim().is_empty() {
        bail!("artifact id is missing");
    }
    if artifact.transcription_run_id.trim().is_empty() {
        bail!("artifact transcription run id is missing");
    }
    if artifact.meeting_id.trim().is_empty() {
        bail!("artifact meeting id is missing");
    }
    if let Some(expected) = expected_meeting {
        if artifact.meeting_id != expected {
            bail!(
                "production artifact belongs to meeting '{}', but the requested Meeting ID is '{}'",
                artifact.meeting_id,
                expected
            );
        }
    }
    if artifact.created_at.trim().is_empty() {
        bail!("artifact creation timestamp is missing");
    }
    if !valid_commit_sha(&artifact.app_commit_sha) {
        bail!("artifact app commit SHA is missing or invalid");
    }
    if artifact.source_audio.duration_ms <= 0 {
        bail!("artifact source audio duration must be greater than zero");
    }
    if artifact.asr.backend.trim().is_empty() || artifact.asr.model.trim().is_empty() {
        bail!("artifact ASR backend or model is missing");
    }
    if !valid_version_or_hash(artifact.asr.version_or_hash.as_deref()) {
        bail!("artifact ASR version or hash is invalid");
    }
    if artifact.diarization.backend.trim().is_empty()
        || artifact.diarization.model.trim().is_empty()
    {
        bail!("artifact diarization backend or model is missing");
    }
    if !valid_version_or_hash(artifact.diarization.version_or_hash.as_deref()) {
        bail!("artifact diarization version or hash is invalid");
    }
    if artifact
        .production_config
        .vad_implementation
        .trim()
        .is_empty()
        || !artifact.production_config.vad_runtime_config.is_object()
    {
        bail!("artifact VAD implementation or runtime config snapshot is missing");
    }
    validate_production_config(&artifact.production_config)?;
    let duration = artifact.source_audio.duration_ms;
    let mut transcript_ids = HashSet::new();
    if artifact.transcripts.iter().any(|item| {
        item.id.trim().is_empty()
            || !transcript_ids.insert(item.id.as_str())
            || item.start_ms < 0
            || item.end_ms <= item.start_ms
            || item.end_ms > duration
            || !valid_confidence(item.asr_confidence)
    }) || artifact.raw_diarizer_turns.iter().any(|item| {
        item.speaker_key.trim().is_empty()
            || item.start_ms < 0
            || item.end_ms <= item.start_ms
            || item.end_ms > duration
            || !valid_confidence(item.confidence)
    }) || artifact.vad_events.iter().any(|item| {
        item.start_ms < 0
            || item.end_ms <= item.start_ms
            || item.end_ms > duration
            || !valid_confidence(item.confidence)
    }) {
        bail!("artifact contains duplicate transcript ids or invalid full-meeting evidence");
    }
    let accepted = artifact
        .accepted_speakers
        .iter()
        .map(|key| key.trim())
        .collect::<HashSet<_>>();
    let visible = artifact
        .visible_speakers
        .iter()
        .map(|key| key.trim())
        .collect::<HashSet<_>>();
    let speaker_universe = artifact
        .raw_diarizer_turns
        .iter()
        .map(|turn| turn.speaker_key.as_str())
        .chain(accepted.iter().copied())
        .collect::<HashSet<_>>();
    if accepted.len() != artifact.accepted_speakers.len()
        || visible.len() != artifact.visible_speakers.len()
        || accepted.contains("")
        || visible.contains("")
        || visible.iter().any(|key| !speaker_universe.contains(key))
    {
        bail!("artifact speaker sets contain empty, duplicate, or unknown keys");
    }
    Ok(())
}

pub fn read_and_validate_artifact(
    path: &Path,
    expected_meeting: Option<&str>,
) -> Result<MeetingProductionArtifact> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let artifact: MeetingProductionArtifact =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
    validate_artifact(&artifact, expected_meeting)?;
    Ok(artifact)
}

#[tauri::command]
pub fn api_inspect_short_turn_production_artifact(
    path: PathBuf,
) -> Result<ProductionArtifactInspection, String> {
    let artifact = read_and_validate_artifact(&path, None)
        .map_err(|error| format!("inspect Production Artifact: {error:#}"))?;
    Ok(ProductionArtifactInspection {
        meeting_id: artifact.meeting_id,
        artifact_id: artifact.artifact_id,
        schema_version: artifact.schema_version,
        transcription_run_id: artifact.transcription_run_id,
        duration_ms: artifact.source_audio.duration_ms,
        asr_backend: artifact.asr.backend,
        asr_model: artifact.asr.model,
        diarization_backend: artifact.diarization.backend,
        diarization_model: artifact.diarization.model,
        transcript_count: artifact.transcripts.len(),
        diarizer_turn_count: artifact.raw_diarizer_turns.len(),
        vad_event_count: artifact.vad_events.len(),
    })
}

/// Persist exactly the VAD/config/speaker state used by a successful run.
/// The caller invokes this inside the same transaction as the derived rows.
pub async fn persist_run_snapshot_tx(
    tx: &mut Transaction<'_, Sqlite>,
    ctx: &AuthContext,
    meeting_id: &str,
    vad_events: &[VadEventCandidateInput],
    accepted_speakers: &[String],
    visible_speakers: &[String],
    production_config: &ProductionConfigSnapshot,
) -> Result<bool> {
    validate_production_config(production_config)?;
    let transcription_runs: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT DISTINCT r.id, r.backend, r.model, r.model_version_or_hash \
         FROM transcripts t INNER JOIN meeting_transcription_runs r ON r.id = t.transcription_run_id \
         WHERE t.meeting_id = ? AND t.workspace_id = ? AND t.deleted_at IS NULL",
    )
    .bind(meeting_id)
    .bind(ctx.tenant_id.as_str())
    .fetch_all(&mut **tx)
    .await?;
    if transcription_runs.is_empty() {
        // Legacy/manual transcript rows have no trustworthy execution record.
        // Keep diarization usable, but remove any stale snapshot rather than
        // exporting mutable workspace settings as fabricated provenance.
        sqlx::query(
            "DELETE FROM meeting_production_snapshots WHERE meeting_id = ? AND workspace_id = ?",
        )
        .bind(meeting_id)
        .bind(ctx.tenant_id.as_str())
        .execute(&mut **tx)
        .await?;
        return Ok(false);
    }
    let [(transcription_run_id, asr_backend, asr_model, asr_version_or_hash)] =
        transcription_runs.as_slice()
    else {
        bail!("meeting transcripts reference multiple transcription runs");
    };
    let events = vad_events
        .iter()
        .map(|event| ArtifactVadEvent {
            start_ms: event.start_ms,
            end_ms: event.end_ms,
            confidence: event.confidence,
        })
        .collect::<Vec<_>>();
    let now = Utc::now().to_rfc3339();
    let snapshot_id = uuid::Uuid::new_v4().to_string();
    let config_json = serde_json::to_string(production_config)?;
    let vad_json = serde_json::to_string(&events)?;
    let accepted_json = serde_json::to_string(accepted_speakers)?;
    let visible_json = serde_json::to_string(visible_speakers)?;
    let result = sqlx::query(
        "INSERT INTO meeting_production_snapshots (id, meeting_id, workspace_id, created_at, app_commit_sha, transcription_run_id, asr_backend, asr_model, asr_version_or_hash, diarization_backend, diarization_model, diarization_version_or_hash, production_config_json, vad_events_json, accepted_speakers_json, visible_speakers_json, long_transcript_speaker_corruption_count, manual_override_violation_count) SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, 'sherpa-onnx', 'pyannote-segmentation-3.0+3D-Speaker-CAM++', 'segmentation:220ad67ca923bef2fa91f2390c786097bf305bceb5e261d4af67b38e938e1079;embedding:f682b514c05d947ee3fa91cd6ec6c5c7543479a128373fa29b1faedccd21fd11', ?, ?, ?, ?, NULL, NULL WHERE EXISTS (SELECT 1 FROM meetings WHERE id = ? AND workspace_id = ?) ON CONFLICT(workspace_id, meeting_id) DO UPDATE SET id=excluded.id, created_at=excluded.created_at, app_commit_sha=excluded.app_commit_sha, transcription_run_id=excluded.transcription_run_id, asr_backend=excluded.asr_backend, asr_model=excluded.asr_model, asr_version_or_hash=excluded.asr_version_or_hash, diarization_backend=excluded.diarization_backend, diarization_model=excluded.diarization_model, diarization_version_or_hash=excluded.diarization_version_or_hash, production_config_json=excluded.production_config_json, vad_events_json=excluded.vad_events_json, accepted_speakers_json=excluded.accepted_speakers_json, visible_speakers_json=excluded.visible_speakers_json, long_transcript_speaker_corruption_count=NULL, manual_override_violation_count=NULL WHERE workspace_id=excluded.workspace_id",
    )
    .bind(snapshot_id)
    .bind(meeting_id)
    .bind(ctx.tenant_id.as_str())
    .bind(now)
    .bind(app_commit_sha())
    .bind(transcription_run_id)
    .bind(asr_backend)
    .bind(asr_model)
    .bind(asr_version_or_hash)
    .bind(config_json)
    .bind(vad_json)
    .bind(accepted_json)
    .bind(visible_json)
    .bind(meeting_id)
    .bind(ctx.tenant_id.as_str())
    .execute(&mut **tx)
    .await?;
    if result.rows_affected() != 1 {
        bail!("meeting {meeting_id} is not in this workspace");
    }
    Ok(true)
}

fn metadata_duration_ms(folder: Option<&str>) -> Option<i64> {
    let folder = Path::new(folder?);
    let bytes = std::fs::read(folder.join("metadata.json")).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get("duration_seconds")
        .and_then(serde_json::Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| (value * 1000.0).round() as i64)
}

/// Build a schema-v2 artifact from the latest persisted production run.
pub async fn build_from_persisted_meeting(
    pool: &SqlitePool,
    ctx: &AuthContext,
    meeting_id: &str,
) -> Result<MeetingProductionArtifact> {
    let meeting = sqlx::query(
        "SELECT folder_path FROM meetings WHERE id = ? AND workspace_id = ? AND deleted_at IS NULL",
    )
    .bind(meeting_id)
    .bind(ctx.tenant_id.as_str())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow::anyhow!("meeting {meeting_id} is not in this workspace"))?;
    let folder: Option<String> = meeting.get("folder_path");
    let snapshot = sqlx::query(
        "SELECT id, created_at, app_commit_sha, transcription_run_id, asr_backend, asr_model, asr_version_or_hash, diarization_backend, diarization_model, diarization_version_or_hash, production_config_json, vad_events_json, accepted_speakers_json, visible_speakers_json, long_transcript_speaker_corruption_count, manual_override_violation_count FROM meeting_production_snapshots WHERE meeting_id = ? AND workspace_id = ?",
    )
    .bind(meeting_id)
    .bind(ctx.tenant_id.as_str())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow::anyhow!("meeting has no completed production short-turn snapshot"))?;
    let asr_backend: Option<String> = snapshot.get("asr_backend");
    let asr_model: Option<String> = snapshot.get("asr_model");
    let (asr_backend, asr_model) = match (asr_backend, asr_model) {
        (Some(backend), Some(model)) if !backend.trim().is_empty() && !model.trim().is_empty() => {
            (backend, model)
        }
        _ => bail!("production run did not persist ASR backend/model provenance"),
    };
    let transcription_run_id: String = snapshot.get("transcription_run_id");
    if transcription_run_id.trim().is_empty() {
        bail!("production snapshot is missing transcription run identity");
    }
    let transcript_rows = sqlx::query(
        "SELECT id, transcript, audio_start_time, audio_end_time, asr_confidence, transcription_run_id FROM transcripts WHERE meeting_id = ? AND workspace_id = ? AND deleted_at IS NULL ORDER BY audio_start_time, audio_end_time, id",
    )
    .bind(meeting_id)
    .bind(ctx.tenant_id.as_str())
    .fetch_all(pool)
    .await?;
    let transcripts = transcript_rows
        .into_iter()
        .map(|row| -> Result<ArtifactTranscript> {
            if row
                .get::<Option<String>, _>("transcription_run_id")
                .as_deref()
                != Some(transcription_run_id.as_str())
            {
                bail!("transcript rows and production snapshot reference different ASR runs");
            }
            let start: Option<f64> = row.get("audio_start_time");
            let end: Option<f64> = row.get("audio_end_time");
            match (start, end) {
                (Some(start), Some(end)) if start.is_finite() && end.is_finite() => {
                    Ok(ArtifactTranscript {
                        id: row.get("id"),
                        start_ms: (start * 1000.0).round() as i64,
                        end_ms: (end * 1000.0).round() as i64,
                        text: row.get("transcript"),
                        asr_confidence: row.get("asr_confidence"),
                    })
                }
                _ => bail!("persisted transcript row is missing finite source timing"),
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let turn_rows = sqlx::query(
        "SELECT start_ms, end_ms, speaker_key, confidence, overlap FROM speaker_turns WHERE meeting_id = ? AND workspace_id = ? ORDER BY start_ms, end_ms, id",
    )
    .bind(meeting_id)
    .bind(ctx.tenant_id.as_str())
    .fetch_all(pool)
    .await?;
    let mut raw_diarizer_turns = turn_rows
        .into_iter()
        .map(|row| ArtifactDiarizerTurn {
            start_ms: row.get("start_ms"),
            end_ms: row.get("end_ms"),
            speaker_key: row.get("speaker_key"),
            confidence: row.get("confidence"),
            overlap: row.get::<i64, _>("overlap") != 0,
        })
        .collect::<Vec<_>>();
    // sherpa emits overlapping raw turns rather than a separate overlap bit.
    // Preserve that evidence in the portable schema by deriving the flag from
    // the complete persisted timeline, without invoking any model.
    let overlap_flags = raw_diarizer_turns
        .iter()
        .enumerate()
        .map(|(index, turn)| {
            raw_diarizer_turns
                .iter()
                .enumerate()
                .any(|(other_index, other)| {
                    other_index != index
                        && other.speaker_key != turn.speaker_key
                        && other.start_ms < turn.end_ms
                        && other.end_ms > turn.start_ms
                })
        })
        .collect::<Vec<_>>();
    for (turn, overlap) in raw_diarizer_turns.iter_mut().zip(overlap_flags) {
        turn.overlap = overlap;
    }
    let production_config: ProductionConfigSnapshot =
        serde_json::from_str(&snapshot.get::<String, _>("production_config_json"))?;
    let vad_events: Vec<ArtifactVadEvent> =
        serde_json::from_str(&snapshot.get::<String, _>("vad_events_json"))?;
    let accepted_speakers: Vec<String> =
        serde_json::from_str(&snapshot.get::<String, _>("accepted_speakers_json"))?;
    let visible_speakers: Vec<String> =
        serde_json::from_str(&snapshot.get::<String, _>("visible_speakers_json"))?;
    let evidence_end = transcripts
        .iter()
        .map(|item| item.end_ms)
        .chain(raw_diarizer_turns.iter().map(|item| item.end_ms))
        .chain(vad_events.iter().map(|item| item.end_ms))
        .max()
        .unwrap_or(0);
    let observed_duration = metadata_duration_ms(folder.as_deref());
    let duration_ms = observed_duration.unwrap_or(evidence_end).max(evidence_end);
    let artifact = MeetingProductionArtifact {
        schema_version: ARTIFACT_SCHEMA_VERSION,
        artifact_id: snapshot.get("id"),
        transcription_run_id,
        meeting_id: meeting_id.to_string(),
        source_audio: SourceAudioMetadata {
            path_hint: None,
            duration_ms,
            sha256: None,
        },
        created_at: snapshot.get("created_at"),
        app_commit_sha: snapshot.get("app_commit_sha"),
        asr: ArtifactBackend {
            backend: asr_backend,
            model: asr_model,
            version_or_hash: snapshot.get("asr_version_or_hash"),
        },
        diarization: ArtifactBackend {
            backend: snapshot.get("diarization_backend"),
            model: snapshot.get("diarization_model"),
            version_or_hash: snapshot.get("diarization_version_or_hash"),
        },
        production_config,
        transcripts,
        raw_diarizer_turns,
        vad_events,
        accepted_speakers,
        visible_speakers,
        safety_observations: ProductionSafetyObservations {
            long_transcript_speaker_corruption_count: snapshot
                .get::<Option<i64>, _>("long_transcript_speaker_corruption_count")
                .map(|value| value.max(0) as usize),
            manual_override_violation_count: snapshot
                .get::<Option<i64>, _>("manual_override_violation_count")
                .map(|value| value.max(0) as usize),
        },
        production_metadata: serde_json::json!({
            "snapshot_source": "persisted_application_state",
            "source_duration_observation": if observed_duration.is_some() { "meeting_metadata" } else { "evidence_extent" },
            "safety_unobserved_is_null": true
        }),
    };
    validate_artifact(&artifact, Some(meeting_id))?;
    Ok(artifact)
}

pub fn write_artifact(path: &Path, artifact: &MeetingProductionArtifact) -> Result<()> {
    validate_artifact(artifact, Some(&artifact.meeting_id))?;
    let bytes = serde_json::to_vec_pretty(artifact)?;
    std::fs::write(path, bytes).with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact() -> MeetingProductionArtifact {
        MeetingProductionArtifact {
            schema_version: ARTIFACT_SCHEMA_VERSION,
            artifact_id: "artifact-1".into(),
            transcription_run_id: "transcription-run-1".into(),
            meeting_id: "meeting-1".into(),
            source_audio: SourceAudioMetadata {
                path_hint: None,
                duration_ms: 2_000,
                sha256: None,
            },
            created_at: "2026-09-14T00:00:00Z".into(),
            app_commit_sha: PHASE_2C1_FROZEN_BASELINE_COMMIT.into(),
            asr: ArtifactBackend {
                backend: "whisper.cpp".into(),
                model: "large-v3".into(),
                version_or_hash: None,
            },
            diarization: ArtifactBackend {
                backend: "sherpa-onnx".into(),
                model: "pyannote+campplus".into(),
                version_or_hash: None,
            },
            production_config: ProductionConfigSnapshot::default(),
            transcripts: vec![ArtifactTranscript {
                id: "transcript-1".into(),
                start_ms: 100,
                end_ms: 800,
                text: "嗯".into(),
                asr_confidence: Some(0.8),
            }],
            raw_diarizer_turns: vec![ArtifactDiarizerTurn {
                start_ms: 0,
                end_ms: 1_000,
                speaker_key: "speaker_01".into(),
                confidence: None,
                overlap: false,
            }],
            vad_events: vec![ArtifactVadEvent {
                start_ms: 100,
                end_ms: 400,
                confidence: None,
            }],
            accepted_speakers: vec!["speaker_01".into()],
            visible_speakers: vec!["speaker_01".into()],
            safety_observations: ProductionSafetyObservations::default(),
            production_metadata: serde_json::json!({"snapshot_source":"persisted_application_state"}),
        }
    }

    #[test]
    fn schema_v2_round_trip_preserves_core_fields() {
        let expected = artifact();
        let bytes = serde_json::to_vec(&expected).unwrap();
        let actual: MeetingProductionArtifact = serde_json::from_slice(&bytes).unwrap();
        validate_artifact(&actual, Some("meeting-1")).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn future_schema_is_not_silently_reinterpreted() {
        let mut value = artifact();
        value.schema_version = 3;
        assert!(validate_artifact(&value, Some("meeting-1")).is_err());
    }

    #[test]
    fn invalid_timing_and_confidence_are_rejected() {
        let mut timing = artifact();
        timing.transcripts[0].end_ms = 3_000;
        assert!(validate_artifact(&timing, Some("meeting-1")).is_err());

        let mut confidence = artifact();
        confidence.raw_diarizer_turns[0].confidence = Some(1.1);
        assert!(validate_artifact(&confidence, Some("meeting-1")).is_err());
    }

    #[test]
    fn meeting_mismatch_reports_both_ids() {
        let error = validate_artifact(&artifact(), Some("meeting-2"))
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "production artifact belongs to meeting 'meeting-1', but the requested Meeting ID is 'meeting-2'"
        );
    }

    #[test]
    fn private_absolute_source_path_is_not_part_of_the_default_schema() {
        let value = serde_json::to_value(artifact()).unwrap();
        assert_eq!(value["source_audio"]["path_hint"], serde_json::Value::Null);
        assert!(!value.to_string().contains("Users\\\\"));
    }

    #[test]
    fn complete_config_is_required_and_ranges_are_validated() {
        let mut json = serde_json::to_value(ProductionConfigSnapshot::default()).unwrap();
        json.as_object_mut().unwrap().remove("vad_runtime_config");
        assert!(serde_json::from_value::<ProductionConfigSnapshot>(json).is_err());

        let mut invalid = ProductionConfigSnapshot::default();
        invalid.speaker_acceptance.max_overlap_ratio = 1.1;
        assert!(validate_production_config(&invalid).is_err());

        let mut inconsistent = ProductionConfigSnapshot::default();
        inconsistent.speaker_acceptance.config.max_short_turn_ms += 1;
        assert!(validate_production_config(&inconsistent).is_err());
    }

    #[test]
    fn duplicate_or_unknown_speaker_keys_are_rejected() {
        let mut duplicate = artifact();
        duplicate.accepted_speakers.push("speaker_01".into());
        assert!(validate_artifact(&duplicate, None).is_err());

        let mut unknown = artifact();
        unknown.visible_speakers = vec!["speaker_02".into()];
        assert!(validate_artifact(&unknown, None).is_err());
    }

    #[test]
    fn inspection_returns_only_validated_summary_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact.json");
        write_artifact(&path, &artifact()).unwrap();

        let summary = api_inspect_short_turn_production_artifact(path).unwrap();
        assert_eq!(summary.meeting_id, "meeting-1");
        assert_eq!(summary.artifact_id, "artifact-1");
        assert_eq!(summary.duration_ms, 2_000);
        assert_eq!(summary.transcript_count, 1);
        assert_eq!(summary.diarizer_turn_count, 1);
        assert_eq!(summary.vad_event_count, 1);
    }

    #[test]
    fn inspection_rejects_malformed_and_future_schema_files() {
        let directory = tempfile::tempdir().unwrap();
        let malformed = directory.path().join("malformed.json");
        std::fs::write(&malformed, b"{not-json").unwrap();
        assert!(api_inspect_short_turn_production_artifact(malformed)
            .unwrap_err()
            .contains("parse"));

        let future = directory.path().join("future.json");
        let mut value = artifact();
        value.schema_version += 1;
        std::fs::write(&future, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(api_inspect_short_turn_production_artifact(future)
            .unwrap_err()
            .contains("is unsupported"));
    }

    #[test]
    fn inspection_rejects_incomplete_identity_backend_duration_and_config() {
        let directory = tempfile::tempdir().unwrap();
        let invalid_artifacts = [
            {
                let mut value = artifact();
                value.artifact_id.clear();
                value
            },
            {
                let mut value = artifact();
                value.source_audio.duration_ms = 0;
                value
            },
            {
                let mut value = artifact();
                value.asr.backend.clear();
                value
            },
            {
                let mut value = artifact();
                value.production_config.vad_implementation.clear();
                value
            },
        ];
        for (index, value) in invalid_artifacts.into_iter().enumerate() {
            let path = directory.path().join(format!("invalid-{index}.json"));
            std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(api_inspect_short_turn_production_artifact(path).is_err());
        }
    }
}
