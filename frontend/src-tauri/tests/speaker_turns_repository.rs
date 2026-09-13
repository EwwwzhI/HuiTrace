//! `speaker_turns` storage: tenant scoping, replace semantics, and the stamp
//! that distinguishes "ran and found nothing" from "never ran" (ADR-0034).

use app_lib::context::{AuthContext, RequestId, Role, TenantId, UserId};
use app_lib::database::repositories::speaker_turn::{SpeakerTurn, SpeakerTurnsRepository};
use app_lib::diarization::short_turn::{
    refine_timeline_assignment, MeetingSpeakerPrototypeStore, ShortTurnRefiner,
};
use app_lib::diarization::timeline::reconcile_transcript;
use app_lib::diarization::types::{
    AssignmentMethod, AudioSource, SegmentKind, SpeakerSegment, TranscriptTiming,
};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

fn ctx_for(workspace: &str) -> AuthContext {
    AuthContext {
        tenant_id: TenantId::new(workspace),
        user_id: UserId::new("user"),
        roles: vec![Role::Owner],
        request_id: RequestId::generate(),
    }
}

async fn db(path: &std::path::Path) -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("open temp db");
    MIGRATOR.run(&pool).await.expect("migrations apply");
    pool
}

async fn seed_meeting(pool: &SqlitePool, id: &str, workspace: &str) {
    sqlx::query(
        "INSERT INTO meetings (id, workspace_id, title, created_at, updated_at) \
         VALUES (?, ?, 'Meeting', '2026-08-09T10:00:00Z', '2026-08-09T10:00:00Z')",
    )
    .bind(id)
    .bind(workspace)
    .execute(pool)
    .await
    .expect("seed meeting");
}

async fn seed_transcript(
    pool: &SqlitePool,
    id: &str,
    meeting_id: &str,
    text: &str,
    start: f64,
    end: f64,
) {
    sqlx::query("INSERT INTO transcripts (id, meeting_id, workspace_id, transcript, timestamp, audio_start_time, audio_end_time, duration) VALUES (?, ?, 'local', ?, '2026-09-13T00:00:00Z', ?, ?, ?)")
        .bind(id)
        .bind(meeting_id)
        .bind(text)
        .bind(start)
        .bind(end)
        .bind(end - start)
        .execute(pool)
        .await
        .expect("seed transcript");
}

fn turn(start_ms: i64, end_ms: i64, label: &str) -> SpeakerTurn {
    SpeakerTurn {
        start_ms,
        end_ms,
        speaker_label: label.to_string(),
        confidence: None,
        speaker_key: String::new(),
    }
}

#[tokio::test]
async fn turns_round_trip_in_time_order_and_stamp_the_meeting() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = db(&dir.path().join("t.db")).await;
    let ctx = ctx_for("local");
    seed_meeting(&pool, "m-1", "local").await;

    assert_eq!(
        SpeakerTurnsRepository::diarized_at(&pool, &ctx, "m-1")
            .await
            .expect("read stamp"),
        None,
        "a meeting that has never been diarized carries no stamp"
    );

    let written = SpeakerTurnsRepository::replace_for_meeting(
        &pool,
        &ctx,
        "m-1",
        &[
            turn(9_000, 20_500, "Speaker 2"),
            turn(0, 9_000, "Speaker 1"),
        ],
    )
    .await
    .expect("write turns");
    assert_eq!(written, 2);

    let turns = SpeakerTurnsRepository::list_for_meeting(&pool, &ctx, "m-1")
        .await
        .expect("read turns");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].start_ms, 0, "returned earliest first");
    assert_eq!(turns[0].speaker_label, "Speaker 1");
    assert!(turns.iter().all(|t| t.confidence.is_none()));

    assert!(SpeakerTurnsRepository::diarized_at(&pool, &ctx, "m-1")
        .await
        .expect("read stamp")
        .is_some());
}

/// The distinction the migration exists to preserve: a pass that separated
/// nothing is an ANSWER, and must not look like a pass that never happened.
#[tokio::test]
async fn a_pass_that_found_nothing_is_still_recorded_as_having_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = db(&dir.path().join("t.db")).await;
    let ctx = ctx_for("local");
    seed_meeting(&pool, "m-1", "local").await;

    let written = SpeakerTurnsRepository::replace_for_meeting(&pool, &ctx, "m-1", &[])
        .await
        .expect("write empty result");
    assert_eq!(written, 0);
    assert!(
        SpeakerTurnsRepository::diarized_at(&pool, &ctx, "m-1")
            .await
            .expect("read stamp")
            .is_some(),
        "an empty result must still stamp diarized_at, or the UI cannot tell it \
         apart from never having run"
    );
}

/// A second pass re-labels the whole recording. Appending would leave one
/// stretch of audio attributed to two speakers at once.
#[tokio::test]
async fn a_second_pass_replaces_rather_than_appends() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = db(&dir.path().join("t.db")).await;
    let ctx = ctx_for("local");
    seed_meeting(&pool, "m-1", "local").await;

    SpeakerTurnsRepository::replace_for_meeting(&pool, &ctx, "m-1", &[turn(0, 9_000, "Speaker 1")])
        .await
        .expect("first pass");
    SpeakerTurnsRepository::replace_for_meeting(
        &pool,
        &ctx,
        "m-1",
        &[turn(0, 4_000, "Speaker 1"), turn(4_000, 9_000, "Speaker 2")],
    )
    .await
    .expect("second pass");

    let turns = SpeakerTurnsRepository::list_for_meeting(&pool, &ctx, "m-1")
        .await
        .expect("read turns");
    assert_eq!(turns.len(), 2, "the first pass must not survive");
    assert_eq!(turns[0].end_ms, 4_000);
}

/// Overlapping turns are legitimate — people talk over each other — so storage
/// must not quietly reject or merge them.
#[tokio::test]
async fn overlapping_turns_are_stored_as_given() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = db(&dir.path().join("t.db")).await;
    let ctx = ctx_for("local");
    seed_meeting(&pool, "m-1", "local").await;

    SpeakerTurnsRepository::replace_for_meeting(
        &pool,
        &ctx,
        "m-1",
        &[
            turn(0, 10_000, "Speaker 1"),
            turn(8_000, 15_000, "Speaker 2"),
        ],
    )
    .await
    .expect("write overlapping turns");

    let turns = SpeakerTurnsRepository::list_for_meeting(&pool, &ctx, "m-1")
        .await
        .expect("read turns");
    assert_eq!(turns.len(), 2);
    assert!(turns[1].start_ms < turns[0].end_ms);
}

/// Tenant scoping, in both directions: a foreign workspace can neither write to
/// nor read this meeting's turns.
#[tokio::test]
async fn a_foreign_workspace_can_neither_write_nor_read() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = db(&dir.path().join("t.db")).await;
    let local = ctx_for("local");
    let foreign = ctx_for("other-ws");
    seed_meeting(&pool, "m-1", "local").await;

    SpeakerTurnsRepository::replace_for_meeting(
        &pool,
        &local,
        "m-1",
        &[turn(0, 9_000, "Speaker 1")],
    )
    .await
    .expect("local write");

    // Writing must fail rather than silently deleting the local turns first.
    assert!(
        SpeakerTurnsRepository::replace_for_meeting(
            &pool,
            &foreign,
            "m-1",
            &[turn(0, 1_000, "Speaker 9")]
        )
        .await
        .is_err(),
        "a foreign workspace must not write to this meeting"
    );
    assert_eq!(
        SpeakerTurnsRepository::list_for_meeting(&pool, &local, "m-1")
            .await
            .expect("local read")
            .len(),
        1,
        "the refused foreign write must not have deleted anything"
    );
    assert!(
        SpeakerTurnsRepository::list_for_meeting(&pool, &foreign, "m-1")
            .await
            .expect("foreign read")
            .is_empty(),
        "a foreign workspace must see no turns"
    );
    assert!(SpeakerTurnsRepository::diarized_at(&pool, &foreign, "m-1")
        .await
        .expect("foreign stamp read")
        .is_none());
}

/// A nonsense turn would be stored, rendered as fact, and cited as evidence.
#[tokio::test]
async fn impossible_turns_are_refused_before_anything_is_written() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = db(&dir.path().join("t.db")).await;
    let ctx = ctx_for("local");
    seed_meeting(&pool, "m-1", "local").await;
    SpeakerTurnsRepository::replace_for_meeting(&pool, &ctx, "m-1", &[turn(0, 9_000, "Speaker 1")])
        .await
        .expect("seed a good pass");

    assert!(SpeakerTurnsRepository::replace_for_meeting(
        &pool,
        &ctx,
        "m-1",
        &[turn(5_000, 1_000, "Speaker 1")]
    )
    .await
    .is_err());
    assert!(SpeakerTurnsRepository::replace_for_meeting(
        &pool,
        &ctx,
        "m-1",
        &[turn(0, 1_000, "   ")]
    )
    .await
    .is_err());

    // Validation happens before the DELETE, so a rejected write leaves the
    // previous good pass intact rather than wiping it.
    assert_eq!(
        SpeakerTurnsRepository::list_for_meeting(&pool, &ctx, "m-1")
            .await
            .expect("read turns")
            .len(),
        1
    );
}

#[tokio::test]
async fn manual_assignment_survives_rerun() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = db(&dir.path().join("manual.db")).await;
    let ctx = ctx_for("local");
    seed_meeting(&pool, "m-1", "local").await;
    seed_transcript(&pool, "t-1", "m-1", "hmm", 0.0, 0.3).await;
    sqlx::query("UPDATE transcripts SET speaker_id = 'speaker_manual', speaker_assignment_method = 'manual' WHERE id = 't-1'")
        .execute(&pool)
        .await
        .expect("manual assignment");

    SpeakerTurnsRepository::replace_for_meeting(&pool, &ctx, "m-1", &[turn(0, 2_000, "Speaker 1")])
        .await
        .expect("rerun");

    let row: (Option<String>, String) = sqlx::query_as(
        "SELECT speaker_id, speaker_assignment_method FROM transcripts WHERE id = 't-1'",
    )
    .fetch_one(&pool)
    .await
    .expect("read assignment");
    assert_eq!(row, (Some("speaker_manual".into()), "manual".into()));
}

#[tokio::test]
async fn swapped_backend_cluster_numbering_preserves_speaker_identity() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = db(&dir.path().join("swap.db")).await;
    let ctx = ctx_for("local");
    seed_meeting(&pool, "m-1", "local").await;
    SpeakerTurnsRepository::replace_for_meeting(
        &pool,
        &ctx,
        "m-1",
        &[turn(0, 2_000, "Speaker 1"), turn(2_000, 4_000, "Speaker 2")],
    )
    .await
    .expect("first pass");
    SpeakerTurnsRepository::replace_for_meeting(
        &pool,
        &ctx,
        "m-1",
        &[turn(0, 2_000, "Speaker 2"), turn(2_000, 4_000, "Speaker 1")],
    )
    .await
    .expect("swapped pass");

    let turns = SpeakerTurnsRepository::list_for_meeting(&pool, &ctx, "m-1")
        .await
        .expect("read turns");
    assert_eq!(turns[0].speaker_key, "speaker_01");
    assert_eq!(turns[1].speaker_key, "speaker_02");
}

#[tokio::test]
async fn restore_automatic_uses_the_current_timeline_immediately() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = db(&dir.path().join("restore.db")).await;
    let ctx = ctx_for("local");
    seed_meeting(&pool, "m-1", "local").await;
    SpeakerTurnsRepository::replace_for_meeting(&pool, &ctx, "m-1", &[turn(0, 2_000, "Speaker 2")])
        .await
        .expect("current timeline");
    let stored = SpeakerTurnsRepository::list_for_meeting(&pool, &ctx, "m-1")
        .await
        .expect("turns");
    let speakers: Vec<SpeakerSegment> = stored
        .iter()
        .map(|turn| SpeakerSegment {
            start_ms: turn.start_ms,
            end_ms: turn.end_ms,
            speaker_key: turn.speaker_key.clone(),
            speaker_confidence: Some(0.9),
            audio_source: AudioSource::Mixed,
            provisional: false,
            revision: 1,
            segment_kind: SegmentKind::Speech,
            assignment_method: AssignmentMethod::Diarization,
            overlap: false,
        })
        .collect();
    let timing = TranscriptTiming {
        id: "t-1".into(),
        start_ms: 100,
        end_ms: 400,
        audio_source: AudioSource::Mixed,
    };
    let assignment = reconcile_transcript(std::slice::from_ref(&timing), &speakers)
        .pop()
        .expect("assignment");
    let restored = refine_timeline_assignment(
        &ShortTurnRefiner::default(),
        &MeetingSpeakerPrototypeStore::new(stored.iter().map(|turn| turn.speaker_key.clone())),
        &timing,
        "hmm",
        Some(0.9),
        assignment,
        &speakers,
    );
    assert_eq!(
        restored.speaker_key.as_deref(),
        Some(stored[0].speaker_key.as_str())
    );
    assert_eq!(stored[0].speaker_label, "Speaker 2");
    assert_eq!(
        restored.assignment_method,
        AssignmentMethod::ShortTurnRefinement
    );
}
