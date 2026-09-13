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
use std::sync::OnceLock;

use crate::context::AuthContext;
use crate::database::repositories::short_turn_event::ShortTurnEventsRepository;
use crate::diarization::short_turn::{
    apply_revision_semantics, refine_assignment_with_candidate, MeetingSpeakerPrototypeStore,
    ShortTurnCandidateExtractor, ShortTurnCandidateSource, ShortTurnRefiner,
    SpeakerAcceptancePolicy, SpeakerAcceptanceTurn, TranscriptCandidateInput,
    VadEventCandidateInput,
};
use crate::diarization::short_turn_event::ShortTurnMaterializationPolicy;
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

static SPEAKER_TURN_WRITE_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

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
        Self::replace_for_meeting_with_source(pool, ctx, meeting_id, turns, AudioSource::Mixed)
            .await
    }

    pub async fn replace_for_meeting_with_source(
        pool: &SqlitePool,
        ctx: &AuthContext,
        meeting_id: &str,
        turns: &[SpeakerTurn],
        source: AudioSource,
    ) -> Result<usize> {
        Self::replace_for_meeting_with_evidence(pool, ctx, meeting_id, turns, source, &[]).await
    }

    pub async fn replace_for_meeting_with_evidence(
        pool: &SqlitePool,
        ctx: &AuthContext,
        meeting_id: &str,
        turns: &[SpeakerTurn],
        source: AudioSource,
        vad_events: &[VadEventCandidateInput],
    ) -> Result<usize> {
        // SQLite permits concurrent analysis but only one writer. Keep the
        // backend jobs independent while serializing their short commit phase.
        let _write_guard = SPEAKER_TURN_WRITE_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
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

        let old_turns = Self::list_raw_for_meeting_tx(&mut tx, ctx, meeting_id).await?;
        let turns = remap_turn_keys(&old_turns, turns);

        // A short-only backend cluster is evidence, not permission to create a
        // meeting speaker. Existing keys remain known across reruns; a new key
        // becomes eligible only after a long, confident, non-overlap speech
        // turn can seed the Phase 2A meeting-local prototype abstraction.
        let acceptance_policy = SpeakerAcceptancePolicy::default();
        let mut known_speaker_keys: std::collections::HashSet<String> = sqlx::query_scalar(
            "SELECT speaker_key FROM speakers WHERE meeting_id = ? AND workspace_id = ?",
        )
        .bind(meeting_id)
        .bind(ctx.tenant_id.as_str())
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
        let acceptance_turns = turns
            .iter()
            .map(|turn| SpeakerAcceptanceTurn {
                start_ms: turn.start_ms,
                end_ms: turn.end_ms,
                speaker_key: turn.speaker_key.clone(),
                confidence: turn.confidence,
            })
            .collect::<Vec<_>>();
        known_speaker_keys.extend(acceptance_policy.accepted_speaker_keys(&acceptance_turns));
        let manual_speakers: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT speaker_id FROM transcripts WHERE meeting_id = ? AND workspace_id = ? AND speaker_assignment_method = 'manual' AND speaker_id IS NOT NULL",
        )
        .bind(meeting_id)
        .bind(ctx.tenant_id.as_str())
        .fetch_all(&mut *tx)
        .await?;
        known_speaker_keys.extend(manual_speakers);
        let manual_event_speakers: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT speaker_key FROM short_turn_events WHERE meeting_id = ? AND workspace_id = ? AND assignment_method = 'manual' AND speaker_key IS NOT NULL",
        )
        .bind(meeting_id)
        .bind(ctx.tenant_id.as_str())
        .fetch_all(&mut *tx)
        .await?;
        known_speaker_keys.extend(manual_event_speakers);

        // Profiles are upserted without touching display_name. A person may
        // rename "Speaker 1" to a real-world name; re-running automatic
        // analysis must never undo that explicit edit.
        for t in &turns {
            let key = if t.speaker_key.is_empty() {
                speaker_key_from_label(&t.speaker_label)
            } else {
                t.speaker_key.clone()
            };
            if !known_speaker_keys.contains(&key) {
                continue;
            }
            sqlx::query("INSERT INTO speakers (id, meeting_id, workspace_id, speaker_key, display_name, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(workspace_id, meeting_id, speaker_key) DO UPDATE SET updated_at = excluded.updated_at")
                .bind(uuid::Uuid::new_v4().to_string()).bind(meeting_id).bind(ctx.tenant_id.as_str())
                .bind(key).bind(&t.speaker_label).bind(&now).bind(&now).execute(&mut *tx).await?;
        }

        sqlx::query("DELETE FROM speaker_turns WHERE meeting_id = ? AND workspace_id = ?")
            .bind(meeting_id)
            .bind(ctx.tenant_id.as_str())
            .execute(&mut *tx)
            .await?;

        for t in &turns {
            sqlx::query(
                "INSERT INTO speaker_turns \
                 (id, meeting_id, workspace_id, speaker_label, speaker_key, start_ms, end_ms, confidence, audio_source, provisional, revision, segment_kind, assignment_method, overlap, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, 1, 'speech', 'diarization', 0, ?, ?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(meeting_id)
            .bind(ctx.tenant_id.as_str())
            .bind(&t.speaker_label)
            .bind(if t.speaker_key.is_empty() { speaker_key_from_label(&t.speaker_label) } else { t.speaker_key.clone() })
            .bind(t.start_ms)
            .bind(t.end_ms)
            .bind(t.confidence)
            .bind(source.as_str())
            .bind(&now)
            .bind(&now)
            .execute(&mut *tx)
            .await?;
        }

        // Reconcile only automatic assignments. `manual` is intentionally a
        // durable override and must outlive every later offline pass.
        let rows = sqlx::query("SELECT id, transcript, audio_start_time, audio_end_time, asr_confidence, speaker_id, speaker_confidence, speaker_provisional, speaker_revision, COALESCE(segment_kind, 'unknown') AS segment_kind, COALESCE(audio_source, 'mixed') AS audio_source, speaker_assignment_method, speaker_overlap FROM transcripts WHERE meeting_id = ? AND workspace_id = ?")
            .bind(meeting_id).bind(ctx.tenant_id.as_str()).fetch_all(&mut *tx).await?;
        let transcript_inputs: Vec<(
            TranscriptCandidateInput,
            crate::diarization::types::TranscriptSpeakerAssignment,
        )> = rows
            .into_iter()
            .filter_map(|row| {
                let start: Option<f64> = row.get("audio_start_time");
                let end: Option<f64> = row.get("audio_end_time");
                match (start, end) {
                    (Some(start), Some(end)) if end > start => {
                        let id: String = row.get("id");
                        let audio_source =
                            audio_source_from_db(&row.get::<String, _>("audio_source"));
                        Some((
                            TranscriptCandidateInput {
                                timing: TranscriptTiming {
                                    id: id.clone(),
                                    start_ms: (start * 1000.0).round() as i64,
                                    end_ms: (end * 1000.0).round() as i64,
                                    audio_source: audio_source.clone(),
                                },
                                text: row.get("transcript"),
                                asr_confidence: row.get("asr_confidence"),
                            },
                            crate::diarization::types::TranscriptSpeakerAssignment {
                                transcript_id: id,
                                speaker_key: row.get("speaker_id"),
                                speaker_confidence: row.get("speaker_confidence"),
                                speaker_provisional: row.get::<i64, _>("speaker_provisional") != 0,
                                speaker_revision: row.get("speaker_revision"),
                                segment_kind: segment_kind_from_db(
                                    &row.get::<String, _>("segment_kind"),
                                ),
                                audio_source,
                                assignment_method: assignment_method_from_db(
                                    &row.get::<String, _>("speaker_assignment_method"),
                                ),
                                overlap: row.get::<i64, _>("speaker_overlap") != 0,
                            },
                        ))
                    }
                    _ => None,
                }
            })
            .collect();
        let timings: Vec<TranscriptTiming> = transcript_inputs
            .iter()
            .map(|(input, _)| input.timing.clone())
            .collect();
        let previous_assignments: std::collections::HashMap<
            &str,
            &crate::diarization::types::TranscriptSpeakerAssignment,
        > = transcript_inputs
            .iter()
            .map(|(input, assignment)| (input.timing.id.as_str(), assignment))
            .collect();
        let raw_speaker_segments: Vec<SpeakerSegment> = turns
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
                audio_source: source.clone(),
                provisional: false,
                revision: 1,
                segment_kind: SegmentKind::Speech,
                assignment_method: AssignmentMethod::Diarization,
                overlap: false,
            })
            .collect();
        let speaker_segments: Vec<SpeakerSegment> = raw_speaker_segments
            .iter()
            .filter(|segment| known_speaker_keys.contains(&segment.speaker_key))
            .cloned()
            .collect();
        let prototypes = MeetingSpeakerPrototypeStore::new(known_speaker_keys.iter().cloned());
        let refiner = ShortTurnRefiner::default();
        let extracted = ShortTurnCandidateExtractor::default().extract(
            &transcript_inputs
                .iter()
                .map(|(input, _)| input.clone())
                .collect::<Vec<_>>(),
            &raw_speaker_segments,
            vad_events,
        );
        let mut candidates_by_transcript: std::collections::HashMap<
            &str,
            Vec<&crate::diarization::short_turn::ShortTurnCandidate>,
        > = std::collections::HashMap::new();
        for (id, candidate) in extracted.iter().flat_map(|candidate| {
            candidate
                .transcript_ids
                .iter()
                .map(move |id| (id.as_str(), candidate))
        }) {
            candidates_by_transcript
                .entry(id)
                .or_default()
                .push(candidate);
        }
        for assignment in reconcile_transcript(&timings, &speaker_segments) {
            let proposed = candidates_by_transcript
                .get(assignment.transcript_id.as_str())
                .and_then(|candidates| {
                    candidates.iter().copied().find(|candidate| {
                        candidate
                            .candidate_sources
                            .contains(&ShortTurnCandidateSource::Transcript)
                            && timings.iter().any(|timing| {
                                timing.id == assignment.transcript_id
                                    && timing.end_ms.saturating_sub(timing.start_ms) <= 1_200
                            })
                    })
                })
                .map(|candidate| {
                    refine_assignment_with_candidate(
                        &refiner,
                        &prototypes,
                        candidate,
                        assignment.clone(),
                    )
                })
                .unwrap_or(assignment);
            let assignment = previous_assignments
                .get(proposed.transcript_id.as_str())
                .map(|previous| apply_revision_semantics(previous, proposed.clone()))
                .unwrap_or(proposed);
            sqlx::query("UPDATE transcripts SET speaker_id = ?, speaker_confidence = ?, speaker_provisional = ?, speaker_revision = ?, segment_kind = ?, audio_source = ?, speaker_assignment_method = ?, speaker_overlap = ? WHERE id = ? AND meeting_id = ? AND workspace_id = ? AND speaker_assignment_method != 'manual'")
                .bind(assignment.speaker_key).bind(assignment.speaker_confidence).bind(assignment.speaker_provisional as i64).bind(assignment.speaker_revision).bind(assignment.segment_kind.as_str()).bind(assignment.audio_source.as_str()).bind(assignment.assignment_method.as_str()).bind(assignment.overlap as i64).bind(assignment.transcript_id).bind(meeting_id).bind(ctx.tenant_id.as_str()).execute(&mut *tx).await?;
        }

        let materializer = ShortTurnMaterializationPolicy::default();
        let events = extracted
            .iter()
            .filter_map(|candidate| {
                let decision = refiner.refine(candidate, &prototypes);
                materializer
                    .materialize(meeting_id, candidate, &decision, &timings)
                    .event
            })
            .collect::<Vec<_>>();
        ShortTurnEventsRepository::replace_automatic_for_meeting_tx(
            &mut tx, ctx, meeting_id, &events,
        )
        .await?;

        // A profile that is neither active nor manually referenced must not
        // survive a smaller rerun and appear in the picker as a phantom.
        sqlx::query("DELETE FROM speakers WHERE meeting_id = ? AND workspace_id = ? AND speaker_key NOT IN (SELECT speaker_key FROM speaker_turns WHERE meeting_id = ? AND workspace_id = ?) AND speaker_key NOT IN (SELECT speaker_id FROM transcripts WHERE meeting_id = ? AND workspace_id = ? AND speaker_assignment_method = 'manual' AND speaker_id IS NOT NULL) AND speaker_key NOT IN (SELECT speaker_key FROM short_turn_events WHERE meeting_id = ? AND workspace_id = ? AND assignment_method = 'manual' AND speaker_key IS NOT NULL)")
            .bind(meeting_id).bind(ctx.tenant_id.as_str()).bind(meeting_id).bind(ctx.tenant_id.as_str()).bind(meeting_id).bind(ctx.tenant_id.as_str()).bind(meeting_id).bind(ctx.tenant_id.as_str()).execute(&mut *tx).await?;

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

    /// Raw evidence turns, retained for repository compatibility and internal
    /// analysis. User-facing callers must use `list_accepted_turns_for_meeting`.
    pub async fn list_for_meeting(
        pool: &SqlitePool,
        ctx: &AuthContext,
        meeting_id: &str,
    ) -> Result<Vec<SpeakerTurn>> {
        Self::list_raw_turns_for_meeting(pool, ctx, meeting_id).await
    }

    pub async fn list_accepted_turns_for_meeting(
        pool: &SqlitePool,
        ctx: &AuthContext,
        meeting_id: &str,
    ) -> Result<Vec<SpeakerTurn>> {
        let rows = sqlx::query(
            "SELECT s.display_name AS speaker_label, st.speaker_key, st.start_ms, st.end_ms, st.confidence FROM speaker_turns st \
             INNER JOIN speakers s ON s.meeting_id = st.meeting_id AND s.workspace_id = st.workspace_id AND s.speaker_key = st.speaker_key \
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

    pub async fn list_raw_turns_for_meeting(
        pool: &SqlitePool,
        ctx: &AuthContext,
        meeting_id: &str,
    ) -> Result<Vec<SpeakerTurn>> {
        let rows = sqlx::query("SELECT speaker_label, COALESCE(speaker_key, '') AS speaker_key, start_ms, end_ms, confidence FROM speaker_turns WHERE meeting_id = ? AND workspace_id = ? ORDER BY start_ms, end_ms")
            .bind(meeting_id).bind(ctx.tenant_id.as_str()).fetch_all(pool).await?;
        Ok(rows
            .into_iter()
            .map(|r| SpeakerTurn {
                speaker_label: r.get("speaker_label"),
                speaker_key: r.get("speaker_key"),
                start_ms: r.get("start_ms"),
                end_ms: r.get("end_ms"),
                confidence: r.get("confidence"),
            })
            .collect())
    }

    async fn list_raw_for_meeting_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        ctx: &AuthContext,
        meeting_id: &str,
    ) -> Result<Vec<SpeakerTurn>> {
        let rows = sqlx::query("SELECT speaker_label, COALESCE(speaker_key, '') AS speaker_key, start_ms, end_ms, confidence FROM speaker_turns WHERE meeting_id = ? AND workspace_id = ?")
            .bind(meeting_id).bind(ctx.tenant_id.as_str()).fetch_all(&mut **tx).await?;
        Ok(rows
            .into_iter()
            .map(|r| SpeakerTurn {
                speaker_label: r.get("speaker_label"),
                speaker_key: r.get("speaker_key"),
                start_ms: r.get("start_ms"),
                end_ms: r.get("end_ms"),
                confidence: r.get("confidence"),
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

/// Preserve meeting-local identity across backend cluster renumbering. Each new
/// cluster greedily claims the old key with which it has the most timeline
/// overlap; ties are deterministic and one old key can only be claimed once.
fn remap_turn_keys(old: &[SpeakerTurn], new: &[SpeakerTurn]) -> Vec<SpeakerTurn> {
    let mut old_keys: Vec<String> = old
        .iter()
        .map(|t| t.speaker_key.clone())
        .filter(|k| !k.is_empty())
        .collect();
    old_keys.sort();
    old_keys.dedup();
    let mut labels: Vec<String> = new.iter().map(|t| t.speaker_label.clone()).collect();
    labels.sort();
    labels.dedup();
    let mut claims: Vec<(i64, String, String)> = Vec::new();
    for label in &labels {
        for key in &old_keys {
            let score: i64 = new
                .iter()
                .filter(|n| &n.speaker_label == label)
                .flat_map(|n| {
                    old.iter()
                        .filter(move |o| &o.speaker_key == key)
                        .map(move |o| (n.end_ms.min(o.end_ms) - n.start_ms.max(o.start_ms)).max(0))
                })
                .sum();
            claims.push((score, label.clone(), key.clone()));
        }
    }
    claims.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    let mut assigned = std::collections::HashMap::new();
    let mut claimed_old = std::collections::HashSet::new();
    for (score, label, key) in claims {
        if score > 0 && !assigned.contains_key(&label) && claimed_old.insert(key.clone()) {
            assigned.insert(label, key);
        }
    }
    // Allocate once per backend cluster label, then reuse that stable key for
    // every turn in the cluster. Reserving all old keys prevents a genuinely
    // new cluster from accidentally inheriting an inactive identity merely
    // because its generated number is available in this pass.
    let mut used: std::collections::HashSet<String> = old_keys.into_iter().collect();
    used.extend(assigned.values().cloned());
    let mut next = 1usize;
    for label in &labels {
        if assigned.contains_key(label) {
            continue;
        }
        while used.contains(&format!("speaker_{next:02}")) {
            next += 1;
        }
        let key = format!("speaker_{next:02}");
        used.insert(key.clone());
        assigned.insert(label.clone(), key);
    }
    new.iter()
        .cloned()
        .map(|mut turn| {
            turn.speaker_key = assigned
                .get(&turn.speaker_label)
                .cloned()
                .expect("every deduplicated label received a key");
            turn
        })
        .collect()
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

fn segment_kind_from_db(value: &str) -> SegmentKind {
    match value {
        "speech" => SegmentKind::Speech,
        "backchannel" => SegmentKind::Backchannel,
        "noise" => SegmentKind::Noise,
        "non_speech_vocalization" => SegmentKind::NonSpeechVocalization,
        _ => SegmentKind::Unknown,
    }
}

fn assignment_method_from_db(value: &str) -> AssignmentMethod {
    match value {
        "manual" => AssignmentMethod::Manual,
        "short_turn_refinement" => AssignmentMethod::ShortTurnRefinement,
        _ => AssignmentMethod::Diarization,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn turn(start_ms: i64, end_ms: i64, label: &str, key: &str) -> SpeakerTurn {
        SpeakerTurn {
            start_ms,
            end_ms,
            speaker_label: label.into(),
            confidence: None,
            speaker_key: key.into(),
        }
    }

    #[test]
    fn remaps_swapped_backend_cluster_numbers_to_existing_voice_keys() {
        let old = vec![
            turn(0, 1_000, "Alice", "speaker_01"),
            turn(1_000, 2_000, "Bob", "speaker_02"),
        ];
        let new = vec![
            turn(0, 1_000, "Speaker 2", "speaker_02"),
            turn(1_000, 2_000, "Speaker 1", "speaker_01"),
        ];
        let remapped = remap_turn_keys(&old, &new);
        assert_eq!(remapped[0].speaker_key, "speaker_01");
        assert_eq!(remapped[1].speaker_key, "speaker_02");
    }
}
