//! Tenant-scoped `speaker_turns` storage (ADR-0034).
//!
//! Every statement scopes on `workspace_id = ctx.tenant_id`, like every other
//! repository here (`docs/CONTRACTS.md` §2).
//!
//! Turns are LOCAL-DERIVED and not synced: ADR-0012 pins the synced entity set,
//! and a peer can regenerate turns from audio. So the table carries no
//! `rev`/`updated_by`/`deleted_at`, and neither does this module.

use anyhow::{bail, Result};
use chrono::Utc;
use sqlx::{Row, SqlitePool};

use crate::context::AuthContext;
use crate::diarization::timeline::reconcile_transcript;
use crate::diarization::types::{
    AssignmentMethod, AudioSource, SegmentKind, SpeakerSegment, TranscriptTiming,
};

/// One anonymous speaker turn, in the units the schema stores.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpeakerTurn {
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker_label: String,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub speaker_key: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpeakerProfile {
    pub speaker_key: String,
    pub display_name: String,
}

pub struct SpeakerTurnsRepository;

impl SpeakerTurnsRepository {
    /// Replace this meeting's turns and stamp `meetings.diarized_at`, in ONE
    /// transaction.
    ///
    /// Replace rather than append: a second diarization pass re-labels the whole
    /// recording, and leaving the previous pass behind would show one stretch of
    /// audio attributed to two different speakers at once.
    ///
    /// The stamp is written even when `turns` is empty, and that is the point:
    /// an empty turn list must mean "ran and found nothing separable", which is
    /// a different statement from "never ran" (the migration's own note). Only
    /// `diarized_at` can tell those apart.
    pub async fn replace_for_meeting(
        pool: &SqlitePool,
        ctx: &AuthContext,
        meeting_id: &str,
        turns: &[SpeakerTurn],
    ) -> Result<usize> {
        if meeting_id.trim().is_empty() {
            bail!("meeting_id cannot be empty");
        }
        for t in turns {
            if t.end_ms <= t.start_ms {
                bail!(
                    "refusing to store a turn that ends before it starts ({} -> {})",
                    t.start_ms,
                    t.end_ms
                );
            }
            if t.speaker_label.trim().is_empty() {
                bail!("refusing to store a turn with no speaker label");
            }
        }

        let now = Utc::now().to_rfc3339();
        let mut tx = pool.begin().await?;

        // Scoped to the caller's workspace: a meeting id from another tenant
        // must match nothing rather than delete anything.
        let owned: Option<String> =
            sqlx::query_scalar("SELECT id FROM meetings WHERE id = ? AND workspace_id = ?")
                .bind(meeting_id)
                .bind(ctx.tenant_id.as_str())
                .fetch_optional(&mut *tx)
                .await?;
        if owned.is_none() {
            tx.rollback().await?;
            bail!("meeting {meeting_id} is not in this workspace");
        }

        // Profiles are upserted without touching display_name. A person may
        // rename "Speaker 1" to a real-world name; re-running automatic
        // analysis must never undo that explicit edit.
        for t in turns {
            let key = if t.speaker_key.is_empty() {
                speaker_key_from_label(&t.speaker_label)
            } else {
                t.speaker_key.clone()
            };
            sqlx::query("INSERT INTO speakers (id, meeting_id, workspace_id, speaker_key, display_name, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(workspace_id, meeting_id, speaker_key) DO UPDATE SET updated_at = excluded.updated_at")
                .bind(uuid::Uuid::new_v4().to_string()).bind(meeting_id).bind(ctx.tenant_id.as_str())
                .bind(key).bind(&t.speaker_label).bind(&now).bind(&now).execute(&mut *tx).await?;
        }

        sqlx::query("DELETE FROM speaker_turns WHERE meeting_id = ? AND workspace_id = ?")
            .bind(meeting_id)
            .bind(ctx.tenant_id.as_str())
            .execute(&mut *tx)
            .await?;

        for t in turns {
            sqlx::query(
                "INSERT INTO speaker_turns \
                 (id, meeting_id, workspace_id, speaker_label, speaker_key, start_ms, end_ms, confidence, audio_source, provisional, revision, segment_kind, assignment_method, overlap, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'mixed', 0, 1, 'speech', 'diarization', 0, ?, ?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(meeting_id)
            .bind(ctx.tenant_id.as_str())
            .bind(&t.speaker_label)
            .bind(if t.speaker_key.is_empty() { speaker_key_from_label(&t.speaker_label) } else { t.speaker_key.clone() })
            .bind(t.start_ms)
            .bind(t.end_ms)
            .bind(t.confidence)
            .bind(&now)
            .bind(&now)
            .execute(&mut *tx)
            .await?;
        }

        // Reconcile only automatic assignments. `manual` is intentionally a
        // durable override and must outlive every later offline pass.
        let rows = sqlx::query("SELECT id, audio_start_time, audio_end_time, COALESCE(audio_source, 'mixed') AS audio_source FROM transcripts WHERE meeting_id = ? AND workspace_id = ? AND speaker_assignment_method != 'manual'")
            .bind(meeting_id).bind(ctx.tenant_id.as_str()).fetch_all(&mut *tx).await?;
        let timings: Vec<TranscriptTiming> = rows
            .into_iter()
            .filter_map(|row| {
                let start: Option<f64> = row.get("audio_start_time");
                let end: Option<f64> = row.get("audio_end_time");
                match (start, end) {
                    (Some(start), Some(end)) if end > start => Some(TranscriptTiming {
                        id: row.get("id"),
                        start_ms: (start * 1000.0).round() as i64,
                        end_ms: (end * 1000.0).round() as i64,
                        audio_source: audio_source_from_db(&row.get::<String, _>("audio_source")),
                    }),
                    _ => None,
                }
            })
            .collect();
        let speaker_segments: Vec<SpeakerSegment> = turns
            .iter()
            .map(|turn| SpeakerSegment {
                start_ms: turn.start_ms,
                end_ms: turn.end_ms,
                speaker_key: if turn.speaker_key.is_empty() {
                    speaker_key_from_label(&turn.speaker_label)
                } else {
                    turn.speaker_key.clone()
                },
                speaker_confidence: turn.confidence,
                audio_source: AudioSource::Mixed,
                provisional: false,
                revision: 1,
                segment_kind: SegmentKind::Speech,
                assignment_method: AssignmentMethod::Diarization,
                overlap: false,
            })
            .collect();
        for assignment in reconcile_transcript(&timings, &speaker_segments) {
            sqlx::query("UPDATE transcripts SET speaker_id = ?, speaker_confidence = ?, speaker_provisional = ?, speaker_revision = ?, segment_kind = ?, audio_source = ?, speaker_assignment_method = ?, speaker_overlap = ? WHERE id = ? AND meeting_id = ? AND workspace_id = ? AND speaker_assignment_method != 'manual'")
                .bind(assignment.speaker_key).bind(assignment.speaker_confidence).bind(assignment.speaker_provisional as i64).bind(assignment.speaker_revision).bind(assignment.segment_kind.as_str()).bind(assignment.audio_source.as_str()).bind(assignment.assignment_method.as_str()).bind(assignment.overlap as i64).bind(assignment.transcript_id).bind(meeting_id).bind(ctx.tenant_id.as_str()).execute(&mut *tx).await?;
        }

        // No `rev` bump: `meetings` is synced, and marking every diarized
        // meeting as freshly modified would make a sync peer re-pull it for a
        // field that is local-derived anyway.
        sqlx::query("UPDATE meetings SET diarized_at = ?, diarization_status = 'completed', diarization_error = NULL WHERE id = ? AND workspace_id = ?")
            .bind(&now)
            .bind(meeting_id)
            .bind(ctx.tenant_id.as_str())
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(turns.len())
    }

    /// This meeting's turns, earliest first.
    pub async fn list_for_meeting(
        pool: &SqlitePool,
        ctx: &AuthContext,
        meeting_id: &str,
    ) -> Result<Vec<SpeakerTurn>> {
        let rows = sqlx::query(
            "SELECT COALESCE(s.display_name, st.speaker_label) AS speaker_label, COALESCE(st.speaker_key, '') AS speaker_key, st.start_ms, st.end_ms, st.confidence FROM speaker_turns st \
             LEFT JOIN speakers s ON s.meeting_id = st.meeting_id AND s.workspace_id = st.workspace_id AND s.speaker_key = st.speaker_key \
             WHERE st.meeting_id = ? AND st.workspace_id = ? ORDER BY st.start_ms, st.end_ms",
        )
        .bind(meeting_id)
        .bind(ctx.tenant_id.as_str())
        .fetch_all(pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| SpeakerTurn {
                speaker_label: r.get("speaker_label"),
                start_ms: r.get("start_ms"),
                end_ms: r.get("end_ms"),
                confidence: r.get("confidence"),
                speaker_key: r.get("speaker_key"),
            })
            .collect())
    }

    pub async fn list_speakers(
        pool: &SqlitePool,
        ctx: &AuthContext,
        meeting_id: &str,
    ) -> Result<Vec<SpeakerProfile>> {
        let rows = sqlx::query("SELECT speaker_key, display_name FROM speakers WHERE meeting_id = ? AND workspace_id = ? ORDER BY speaker_key")
            .bind(meeting_id).bind(ctx.tenant_id.as_str()).fetch_all(pool).await?;
        Ok(rows
            .into_iter()
            .map(|row| SpeakerProfile {
                speaker_key: row.get("speaker_key"),
                display_name: row.get("display_name"),
            })
            .collect())
    }

    pub async fn rename_speaker(
        pool: &SqlitePool,
        ctx: &AuthContext,
        meeting_id: &str,
        speaker_key: &str,
        display_name: &str,
    ) -> Result<()> {
        if speaker_key.trim().is_empty() || display_name.trim().is_empty() {
            bail!("speaker key and display name cannot be empty");
        }
        let result = sqlx::query("UPDATE speakers SET display_name = ?, updated_at = ? WHERE meeting_id = ? AND workspace_id = ? AND speaker_key = ?")
            .bind(display_name.trim()).bind(Utc::now().to_rfc3339()).bind(meeting_id).bind(ctx.tenant_id.as_str()).bind(speaker_key).execute(pool).await?;
        if result.rows_affected() == 0 {
            bail!("speaker {speaker_key} is not in this meeting");
        }
        Ok(())
    }

    /// When a diarization pass last completed, or `None` if none ever has.
    ///
    /// `None` is NOT "no speakers found" — see `replace_for_meeting`.
    pub async fn diarized_at(
        pool: &SqlitePool,
        ctx: &AuthContext,
        meeting_id: &str,
    ) -> Result<Option<String>> {
        let value: Option<Option<String>> = sqlx::query_scalar(
            "SELECT diarized_at FROM meetings WHERE id = ? AND workspace_id = ?",
        )
        .bind(meeting_id)
        .bind(ctx.tenant_id.as_str())
        .fetch_optional(pool)
        .await?;
        Ok(value.flatten())
    }
}

fn speaker_key_from_label(label: &str) -> String {
    if let Some(number) = label
        .split_whitespace()
        .last()
        .and_then(|value| value.parse::<usize>().ok())
    {
        return format!("speaker_{number:02}");
    }
    format!("speaker_{:08x}", crc32(label.as_bytes()))
}

fn crc32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0u32, |hash, byte| {
        hash.wrapping_mul(16777619) ^ u32::from(*byte)
    })
}

fn audio_source_from_db(value: &str) -> AudioSource {
    match value {
        "microphone" => AudioSource::Microphone,
        "system" => AudioSource::System,
        "imported" => AudioSource::Imported,
        _ => AudioSource::Mixed,
    }
}
