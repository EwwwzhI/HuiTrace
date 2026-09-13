//! Versioned, privacy-minimised snapshots of a completed production meeting run.
//!
//! The builder only reads persisted application state. It deliberately has no
//! ASR, diarization, VAD, or short-turn inference dependency, so exporting an
//! artifact cannot create a second inference architecture.

use std::collections::HashSet;
use std::path::Path;

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

pub const ARTIFACT_SCHEMA_VERSION: u32 = 1;
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
    #[serde(default)]
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

fn valid_confidence(value: Option<f64>) -> bool {
    value.map_or(true, |v| v.is_finite() && (0.0..=1.0).contains(&v))
}

fn valid_commit_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
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
    if artifact.artifact_id.trim().is_empty()
        || artifact.meeting_id.trim().is_empty()
        || expected_meeting.is_some_and(|meeting| artifact.meeting_id != meeting)
        || artifact.created_at.trim().is_empty()
        || !valid_commit_sha(&artifact.app_commit_sha)
        || artifact.source_audio.duration_ms <= 0
        || artifact.asr.backend.trim().is_empty()
        || artifact.asr.model.trim().is_empty()
        || artifact.diarization.backend.trim().is_empty()
        || artifact.diarization.model.trim().is_empty()
        || artifact
            .production_config
            .vad_implementation
            .trim()
            .is_empty()
        || !artifact.production_config.vad_runtime_config.is_object()
    {
        bail!("artifact identity, backend, source duration, or config snapshot is incomplete");
    }
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

/// Persist exactly the VAD/config/speaker state used by a successful run.
/// The caller invokes this inside the same transaction as the derived rows.
pub async fn persist_run_snapshot_tx(
    tx: &mut Transaction<'_, Sqlite>,
    ctx: &AuthContext,
    meeting_id: &str,
    vad_events: &[VadEventCandidateInput],
    accepted_speakers: &[String],
    visible_speakers: &[String],
) -> Result<()> {
    let asr: Option<(String, String)> = sqlx::query_as(
        "SELECT provider, model FROM transcript_settings WHERE workspace_id = ? ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(ctx.tenant_id.as_str())
    .fetch_optional(&mut **tx)
    .await?;
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
    let config_json = serde_json::to_string(&ProductionConfigSnapshot::default())?;
    let vad_json = serde_json::to_string(&events)?;
    let accepted_json = serde_json::to_string(accepted_speakers)?;
    let visible_json = serde_json::to_string(visible_speakers)?;
    let result = sqlx::query(
        "INSERT INTO meeting_production_snapshots (id, meeting_id, workspace_id, created_at, app_commit_sha, asr_backend, asr_model, asr_version_or_hash, diarization_backend, diarization_model, diarization_version_or_hash, production_config_json, vad_events_json, accepted_speakers_json, visible_speakers_json, long_transcript_speaker_corruption_count, manual_override_violation_count) SELECT ?, ?, ?, ?, ?, ?, ?, NULL, 'sherpa-onnx', 'pyannote-segmentation-3.0+3D-Speaker-CAM++', 'segmentation:220ad67ca923bef2fa91f2390c786097bf305bceb5e261d4af67b38e938e1079;embedding:f682b514c05d947ee3fa91cd6ec6c5c7543479a128373fa29b1faedccd21fd11', ?, ?, ?, ?, NULL, NULL WHERE EXISTS (SELECT 1 FROM meetings WHERE id = ? AND workspace_id = ?) ON CONFLICT(workspace_id, meeting_id) DO UPDATE SET id=excluded.id, created_at=excluded.created_at, app_commit_sha=excluded.app_commit_sha, asr_backend=excluded.asr_backend, asr_model=excluded.asr_model, asr_version_or_hash=excluded.asr_version_or_hash, diarization_backend=excluded.diarization_backend, diarization_model=excluded.diarization_model, diarization_version_or_hash=excluded.diarization_version_or_hash, production_config_json=excluded.production_config_json, vad_events_json=excluded.vad_events_json, accepted_speakers_json=excluded.accepted_speakers_json, visible_speakers_json=excluded.visible_speakers_json, long_transcript_speaker_corruption_count=NULL, manual_override_violation_count=NULL WHERE workspace_id=excluded.workspace_id",
    )
    .bind(snapshot_id)
    .bind(meeting_id)
    .bind(ctx.tenant_id.as_str())
    .bind(now)
    .bind(app_commit_sha())
    .bind(asr.as_ref().map(|value| value.0.as_str()))
    .bind(asr.as_ref().map(|value| value.1.as_str()))
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
    Ok(())
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

/// Build a schema-v1 artifact from the latest persisted production run.
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
        "SELECT id, created_at, app_commit_sha, asr_backend, asr_model, asr_version_or_hash, diarization_backend, diarization_model, diarization_version_or_hash, production_config_json, vad_events_json, accepted_speakers_json, visible_speakers_json, long_transcript_speaker_corruption_count, manual_override_violation_count FROM meeting_production_snapshots WHERE meeting_id = ? AND workspace_id = ?",
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
    let transcript_rows = sqlx::query(
        "SELECT id, transcript, audio_start_time, audio_end_time, asr_confidence FROM transcripts WHERE meeting_id = ? AND workspace_id = ? AND deleted_at IS NULL ORDER BY audio_start_time, audio_end_time, id",
    )
    .bind(meeting_id)
    .bind(ctx.tenant_id.as_str())
    .fetch_all(pool)
    .await?;
    let transcripts = transcript_rows
        .into_iter()
        .map(|row| -> Result<ArtifactTranscript> {
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
    fn schema_v1_round_trip_preserves_core_fields() {
        let expected = artifact();
        let bytes = serde_json::to_vec(&expected).unwrap();
        let actual: MeetingProductionArtifact = serde_json::from_slice(&bytes).unwrap();
        validate_artifact(&actual, Some("meeting-1")).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn future_schema_is_not_silently_reinterpreted() {
        let mut value = artifact();
        value.schema_version = 2;
        assert!(validate_artifact(&value, Some("meeting-1")).is_err());
    }

    #[test]
    fn invalid_timing_confidence_and_meeting_mismatch_are_rejected() {
        let mut timing = artifact();
        timing.transcripts[0].end_ms = 3_000;
        assert!(validate_artifact(&timing, Some("meeting-1")).is_err());

        let mut confidence = artifact();
        confidence.raw_diarizer_turns[0].confidence = Some(1.1);
        assert!(validate_artifact(&confidence, Some("meeting-1")).is_err());

        assert!(validate_artifact(&artifact(), Some("meeting-2")).is_err());
    }

    #[test]
    fn private_absolute_source_path_is_not_part_of_the_default_schema() {
        let value = serde_json::to_value(artifact()).unwrap();
        assert_eq!(value["source_audio"]["path_hint"], serde_json::Value::Null);
        assert!(!value.to_string().contains("Users\\\\"));
    }
}
