//! Workspace-scoped persistence for Phase 2C short timeline annotations.

use std::collections::HashMap;

use anyhow::{bail, Result};
use chrono::Utc;
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use crate::context::AuthContext;
use crate::diarization::short_turn::ShortTurnCandidateSource;
use crate::diarization::short_turn_event::ShortTurnEvent;
use crate::diarization::types::{AssignmentMethod, AudioSource, SegmentKind};

pub struct ShortTurnEventsRepository;

macro_rules! event_query {
    ($suffix:literal) => {
        sqlx::query(concat!(
            "SELECT e.id, e.meeting_id, e.transcript_id, e.start_ms, e.end_ms, e.segment_kind, e.kind_confidence, e.speaker_key, s.display_name AS speaker_display_name, e.speaker_confidence, e.candidate_sources, e.audio_source, e.assignment_method, e.revision, e.transcript_aligned, e.user_visible FROM short_turn_events e LEFT JOIN speakers s ON s.meeting_id = e.meeting_id AND s.workspace_id = e.workspace_id AND s.speaker_key = e.speaker_key ",
            $suffix
        ))
    };
}

impl ShortTurnEventsRepository {
    pub(crate) async fn replace_automatic_for_meeting_tx(
        tx: &mut Transaction<'_, Sqlite>,
        ctx: &AuthContext,
        meeting_id: &str,
        events: &[ShortTurnEvent],
    ) -> Result<usize> {
        let existing = Self::list_for_meeting_tx(tx, ctx, meeting_id).await?;
        let previous_by_id: HashMap<_, _> = existing
            .iter()
            .map(|event| (event.id.as_str(), event))
            .collect();
        let manual_ids: std::collections::HashSet<_> = existing
            .iter()
            .filter(|event| event.assignment_method == AssignmentMethod::Manual)
            .map(|event| event.id.as_str())
            .collect();

        sqlx::query("DELETE FROM short_turn_events WHERE meeting_id = ? AND workspace_id = ? AND assignment_method != 'manual'")
            .bind(meeting_id)
            .bind(ctx.tenant_id.as_str())
            .execute(&mut **tx)
            .await?;

        let now = Utc::now().to_rfc3339();
        let mut written = 0;
        for event in events {
            if event.meeting_id != meeting_id {
                bail!("short-turn event belongs to another meeting");
            }
            if event.end_ms <= event.start_ms {
                bail!("short-turn event has invalid timing");
            }
            if manual_ids.contains(event.id.as_str()) {
                continue;
            }
            let mut event = event.clone();
            if let Some(previous) = previous_by_id.get(event.id.as_str()) {
                event.revision = if same_automatic_result(previous, &event) {
                    previous.revision
                } else {
                    previous.revision.saturating_add(1)
                };
            }
            let sources = serde_json::to_string(&event.candidate_sources)?;
            sqlx::query(
                "INSERT INTO short_turn_events (id, meeting_id, workspace_id, transcript_id, start_ms, end_ms, segment_kind, kind_confidence, speaker_key, speaker_confidence, candidate_sources, audio_source, assignment_method, revision, transcript_aligned, user_visible, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&event.id)
            .bind(meeting_id)
            .bind(ctx.tenant_id.as_str())
            .bind(&event.transcript_id)
            .bind(event.start_ms)
            .bind(event.end_ms)
            .bind(event.kind.as_str())
            .bind(event.kind_confidence)
            .bind(&event.speaker_key)
            .bind(event.speaker_confidence)
            .bind(sources)
            .bind(event.audio_source.as_str())
            .bind(event.assignment_method.as_str())
            .bind(event.revision)
            .bind(event.transcript_aligned as i64)
            .bind(event.user_visible as i64)
            .bind(&now)
            .bind(&now)
            .execute(&mut **tx)
            .await?;
            written += 1;
        }
        Ok(written)
    }

    pub async fn replace_for_meeting(
        pool: &SqlitePool,
        ctx: &AuthContext,
        meeting_id: &str,
        events: &[ShortTurnEvent],
    ) -> Result<usize> {
        ensure_owned(pool, ctx, meeting_id).await?;
        let mut tx = pool.begin().await?;
        let written =
            Self::replace_automatic_for_meeting_tx(&mut tx, ctx, meeting_id, events).await?;
        tx.commit().await?;
        Ok(written)
    }

    pub async fn list_for_meeting(
        pool: &SqlitePool,
        ctx: &AuthContext,
        meeting_id: &str,
    ) -> Result<Vec<ShortTurnEvent>> {
        let rows = event_query!(
            "WHERE e.meeting_id = ? AND e.workspace_id = ? ORDER BY e.start_ms, e.end_ms, e.id"
        )
        .bind(meeting_id)
        .bind(ctx.tenant_id.as_str())
        .fetch_all(pool)
        .await?;
        rows.into_iter().map(event_from_row).collect()
    }

    /// Only embedded/non-aligned annotations are returned to the transcript UI.
    /// Transcript-aligned events remain persisted as the unified short-event
    /// truth but the transcript row owns their primary rendering.
    pub async fn list_visible_annotations_for_meeting(
        pool: &SqlitePool,
        ctx: &AuthContext,
        meeting_id: &str,
    ) -> Result<Vec<ShortTurnEvent>> {
        let rows = event_query!(
            "WHERE e.meeting_id = ? AND e.workspace_id = ? AND e.user_visible = 1 AND e.transcript_aligned = 0 ORDER BY e.start_ms, e.end_ms, e.id"
        )
        .bind(meeting_id)
        .bind(ctx.tenant_id.as_str())
        .fetch_all(pool)
        .await?;
        rows.into_iter().map(event_from_row).collect()
    }

    pub async fn list_for_transcript(
        pool: &SqlitePool,
        ctx: &AuthContext,
        meeting_id: &str,
        transcript_id: &str,
    ) -> Result<Vec<ShortTurnEvent>> {
        let rows = event_query!(
            "WHERE e.meeting_id = ? AND e.workspace_id = ? AND e.transcript_id = ? ORDER BY e.start_ms, e.end_ms, e.id"
        )
        .bind(meeting_id)
        .bind(ctx.tenant_id.as_str())
        .bind(transcript_id)
        .fetch_all(pool)
        .await?;
        rows.into_iter().map(event_from_row).collect()
    }

    pub async fn delete_for_meeting(
        pool: &SqlitePool,
        ctx: &AuthContext,
        meeting_id: &str,
    ) -> Result<u64> {
        Ok(
            sqlx::query("DELETE FROM short_turn_events WHERE meeting_id = ? AND workspace_id = ?")
                .bind(meeting_id)
                .bind(ctx.tenant_id.as_str())
                .execute(pool)
                .await?
                .rows_affected(),
        )
    }

    pub async fn assign_speaker(
        pool: &SqlitePool,
        ctx: &AuthContext,
        meeting_id: &str,
        event_id: &str,
        speaker_key: &str,
    ) -> Result<()> {
        let valid: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM speakers WHERE meeting_id = ? AND workspace_id = ? AND speaker_key = ?",
        )
        .bind(meeting_id.to_string())
        .bind(ctx.tenant_id.as_str().to_string())
        .bind(speaker_key)
        .fetch_optional(pool)
        .await?;
        if valid.is_none() {
            bail!("speaker does not belong to this meeting");
        }
        let result = sqlx::query("UPDATE short_turn_events SET speaker_key = ?, speaker_confidence = NULL, assignment_method = 'manual', revision = revision + CASE WHEN speaker_key IS NOT ? OR assignment_method != 'manual' THEN 1 ELSE 0 END, updated_at = ? WHERE id = ? AND meeting_id = ? AND workspace_id = ?")
            .bind(speaker_key)
            .bind(speaker_key)
            .bind(Utc::now().to_rfc3339())
            .bind(event_id)
            .bind(meeting_id)
            .bind(ctx.tenant_id.as_str())
            .execute(pool)
            .await?;
        if result.rows_affected() != 1 {
            bail!("short-turn event does not belong to this meeting");
        }
        Ok(())
    }

    pub async fn restore_automatic(
        pool: &SqlitePool,
        ctx: &AuthContext,
        meeting_id: &str,
        event_id: &str,
    ) -> Result<()> {
        let event: Option<(i64, i64)> = sqlx::query_as(
            "SELECT start_ms, end_ms FROM short_turn_events WHERE id = ? AND meeting_id = ? AND workspace_id = ?",
        )
        .bind(event_id)
        .bind(meeting_id.to_string())
        .bind(ctx.tenant_id.as_str().to_string())
        .fetch_optional(pool)
        .await?;
        let (start_ms, end_ms) = event
            .ok_or_else(|| anyhow::anyhow!("short-turn event does not belong to this meeting"))?;
        let automatic_speaker: Option<(String, Option<f64>)> = sqlx::query_as(
            "SELECT st.speaker_key, st.confidence FROM speaker_turns st INNER JOIN speakers s ON s.meeting_id = st.meeting_id AND s.workspace_id = st.workspace_id AND s.speaker_key = st.speaker_key WHERE st.meeting_id = ? AND st.workspace_id = ? AND st.start_ms < ? AND st.end_ms > ? ORDER BY (MIN(st.end_ms, ?) - MAX(st.start_ms, ?)) DESC, st.speaker_key LIMIT 1",
        )
        .bind(meeting_id)
        .bind(ctx.tenant_id.as_str())
        .bind(end_ms)
        .bind(start_ms)
        .bind(end_ms)
        .bind(start_ms)
        .fetch_optional(pool)
        .await?;
        let (speaker_key, confidence) = automatic_speaker
            .map(|(key, confidence)| (Some(key), confidence))
            .unwrap_or((None, None));
        sqlx::query("UPDATE short_turn_events SET speaker_key = ?, speaker_confidence = ?, assignment_method = 'short_turn_refinement', revision = revision + 1, updated_at = ? WHERE id = ? AND meeting_id = ? AND workspace_id = ?")
            .bind(speaker_key)
            .bind(confidence)
            .bind(Utc::now().to_rfc3339())
            .bind(event_id)
            .bind(meeting_id)
            .bind(ctx.tenant_id.as_str())
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn list_for_meeting_tx(
        tx: &mut Transaction<'_, Sqlite>,
        ctx: &AuthContext,
        meeting_id: &str,
    ) -> Result<Vec<ShortTurnEvent>> {
        let rows = event_query!(
            "WHERE e.meeting_id = ? AND e.workspace_id = ? ORDER BY e.start_ms, e.end_ms, e.id"
        )
        .bind(meeting_id)
        .bind(ctx.tenant_id.as_str())
        .fetch_all(&mut **tx)
        .await?;
        rows.into_iter().map(event_from_row).collect()
    }
}

fn event_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ShortTurnEvent> {
    let sources: String = row.get("candidate_sources");
    Ok(ShortTurnEvent {
        id: row.get("id"),
        meeting_id: row.get("meeting_id"),
        transcript_id: row.get("transcript_id"),
        start_ms: row.get("start_ms"),
        end_ms: row.get("end_ms"),
        kind: kind_from_db(&row.get::<String, _>("segment_kind")),
        kind_confidence: row.get("kind_confidence"),
        speaker_key: row.get("speaker_key"),
        speaker_display_name: row.get("speaker_display_name"),
        speaker_confidence: row.get("speaker_confidence"),
        candidate_sources: serde_json::from_str::<Vec<ShortTurnCandidateSource>>(&sources)?,
        audio_source: source_from_db(&row.get::<String, _>("audio_source")),
        revision: row.get("revision"),
        assignment_method: method_from_db(&row.get::<String, _>("assignment_method")),
        transcript_aligned: row.get::<i64, _>("transcript_aligned") != 0,
        user_visible: row.get::<i64, _>("user_visible") != 0,
    })
}

fn same_automatic_result(left: &ShortTurnEvent, right: &ShortTurnEvent) -> bool {
    left.transcript_id == right.transcript_id
        && left.start_ms == right.start_ms
        && left.end_ms == right.end_ms
        && left.kind == right.kind
        && (left.kind_confidence - right.kind_confidence).abs() < 1e-9
        && left.speaker_key == right.speaker_key
        && option_f64_eq(left.speaker_confidence, right.speaker_confidence)
        && left.candidate_sources == right.candidate_sources
        && left.audio_source == right.audio_source
        && left.transcript_aligned == right.transcript_aligned
        && left.user_visible == right.user_visible
}

fn option_f64_eq(left: Option<f64>, right: Option<f64>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => (left - right).abs() < 1e-9,
        (None, None) => true,
        _ => false,
    }
}

async fn ensure_owned(pool: &SqlitePool, ctx: &AuthContext, meeting_id: &str) -> Result<()> {
    let owned: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM meetings WHERE id = ? AND workspace_id = ?")
            .bind(meeting_id)
            .bind(ctx.tenant_id.as_str())
            .fetch_optional(pool)
            .await?;
    if owned.is_none() {
        bail!("meeting is not in this workspace");
    }
    Ok(())
}

fn kind_from_db(value: &str) -> SegmentKind {
    match value {
        "speech" => SegmentKind::Speech,
        "backchannel" => SegmentKind::Backchannel,
        "noise" => SegmentKind::Noise,
        "non_speech_vocalization" => SegmentKind::NonSpeechVocalization,
        _ => SegmentKind::Unknown,
    }
}

fn source_from_db(value: &str) -> AudioSource {
    match value {
        "microphone" => AudioSource::Microphone,
        "system" => AudioSource::System,
        "imported" => AudioSource::Imported,
        _ => AudioSource::Mixed,
    }
}

fn method_from_db(value: &str) -> AssignmentMethod {
    match value {
        "manual" => AssignmentMethod::Manual,
        "short_turn_refinement" => AssignmentMethod::ShortTurnRefinement,
        _ => AssignmentMethod::Diarization,
    }
}
