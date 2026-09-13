//! `speaker_turns` storage: tenant scoping, replace semantics, and the stamp
//! that distinguishes "ran and found nothing" from "never ran" (ADR-0034).

use app_lib::context::{AuthContext, RequestId, Role, TenantId, UserId};
use app_lib::database::repositories::short_turn_event::ShortTurnEventsRepository;
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

fn confident_turn(start_ms: i64, end_ms: i64, label: &str, confidence: f64) -> SpeakerTurn {
    SpeakerTurn {
        confidence: Some(confidence),
        ..turn(start_ms, end_ms, label)
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

#[tokio::test]
async fn weak_short_only_cluster_stays_raw_but_never_becomes_visible_or_final() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = db(&dir.path().join("phantom.db")).await;
    let ctx = ctx_for("local");
    seed_meeting(&pool, "m-1", "local").await;
    seed_transcript(&pool, "t-weak", "m-1", "oh", 2.0, 2.25).await;

    SpeakerTurnsRepository::replace_for_meeting(
        &pool,
        &ctx,
        "m-1",
        &[
            confident_turn(0, 2_000, "Speaker 1", 0.92),
            confident_turn(2_000, 2_250, "Speaker 3", 0.30),
        ],
    )
    .await
    .expect("store raw evidence");

    assert_eq!(
        SpeakerTurnsRepository::list_raw_turns_for_meeting(&pool, &ctx, "m-1")
            .await
            .expect("raw")
            .len(),
        2
    );
    let accepted = SpeakerTurnsRepository::list_accepted_turns_for_meeting(&pool, &ctx, "m-1")
        .await
        .expect("accepted");
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].speaker_key, "speaker_01");
    let final_speaker: Option<String> =
        sqlx::query_scalar("SELECT speaker_id FROM transcripts WHERE id = 't-weak'")
            .fetch_one(&pool)
            .await
            .expect("final assignment");
    assert_eq!(final_speaker, None);
}

#[tokio::test]
async fn missing_confidence_needs_multiple_turns_not_one_two_second_cluster() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = db(&dir.path().join("acceptance.db")).await;
    let ctx = ctx_for("local");
    seed_meeting(&pool, "m-1", "local").await;
    SpeakerTurnsRepository::replace_for_meeting(&pool, &ctx, "m-1", &[turn(0, 2_000, "Speaker 1")])
        .await
        .expect("store raw turn");
    assert!(
        SpeakerTurnsRepository::list_accepted_turns_for_meeting(&pool, &ctx, "m-1")
            .await
            .expect("accepted")
            .is_empty()
    );

    SpeakerTurnsRepository::replace_for_meeting(
        &pool,
        &ctx,
        "m-1",
        &[turn(0, 2_000, "Speaker 1"), turn(3_000, 5_000, "Speaker 1")],
    )
    .await
    .expect("store corroborated turns");
    assert_eq!(
        SpeakerTurnsRepository::list_accepted_turns_for_meeting(&pool, &ctx, "m-1")
            .await
            .expect("accepted")
            .len(),
        2
    );
}

#[tokio::test]
async fn embedded_short_turn_materializes_without_relabeling_long_transcript() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = db(&dir.path().join("embedded.db")).await;
    let ctx = ctx_for("local");
    seed_meeting(&pool, "m-1", "local").await;
    seed_transcript(&pool, "t-long", "m-1", "a long sentence", 0.0, 5.0).await;

    SpeakerTurnsRepository::replace_for_meeting(
        &pool,
        &ctx,
        "m-1",
        &[
            confident_turn(0, 5_000, "Speaker A", 0.92),
            confident_turn(2_100, 2_400, "Speaker B", 0.93),
            confident_turn(6_000, 8_000, "Speaker B", 0.93),
        ],
    )
    .await
    .expect("diarization");

    let transcript_speaker: Option<String> =
        sqlx::query_scalar("SELECT speaker_id FROM transcripts WHERE id = 't-long'")
            .fetch_one(&pool)
            .await
            .expect("transcript speaker");
    let accepted = SpeakerTurnsRepository::list_accepted_turns_for_meeting(&pool, &ctx, "m-1")
        .await
        .expect("accepted");
    let speaker_a = accepted
        .iter()
        .find(|turn| turn.speaker_label == "Speaker A")
        .expect("speaker A");
    let speaker_b = accepted
        .iter()
        .find(|turn| turn.speaker_label == "Speaker B")
        .expect("speaker B");
    assert_eq!(
        transcript_speaker.as_deref(),
        Some(speaker_a.speaker_key.as_str())
    );

    let events = ShortTurnEventsRepository::list_for_transcript(&pool, &ctx, "m-1", "t-long")
        .await
        .expect("events");
    assert_eq!(events.len(), 1);
    assert!(!events[0].transcript_aligned);
    assert_eq!(
        events[0].speaker_key.as_deref(),
        Some(speaker_b.speaker_key.as_str())
    );
    assert_eq!(events[0].kind, SegmentKind::Speech);
}

#[tokio::test]
async fn two_embedded_candidates_in_one_transcript_are_both_preserved() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = db(&dir.path().join("multiple.db")).await;
    let ctx = ctx_for("local");
    seed_meeting(&pool, "m-1", "local").await;
    seed_transcript(&pool, "t-long", "m-1", "one long ASR row", 0.0, 5.0).await;

    SpeakerTurnsRepository::replace_for_meeting(
        &pool,
        &ctx,
        "m-1",
        &[
            confident_turn(0, 5_000, "Speaker A", 0.95),
            confident_turn(1_500, 1_800, "Speaker B", 0.95),
            confident_turn(5_500, 7_500, "Speaker B", 0.95),
            confident_turn(3_000, 3_300, "Speaker C", 0.95),
            confident_turn(8_000, 10_000, "Speaker C", 0.95),
        ],
    )
    .await
    .expect("diarization");

    let events = ShortTurnEventsRepository::list_for_transcript(&pool, &ctx, "m-1", "t-long")
        .await
        .expect("events");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].start_ms, 1_500);
    assert_eq!(events[1].start_ms, 3_000);
}

#[tokio::test]
async fn transcript_aligned_event_is_not_returned_as_duplicate_annotation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = db(&dir.path().join("aligned.db")).await;
    let ctx = ctx_for("local");
    seed_meeting(&pool, "m-1", "local").await;
    seed_transcript(&pool, "t-short", "m-1", "hmm", 0.1, 0.4).await;

    SpeakerTurnsRepository::replace_for_meeting(
        &pool,
        &ctx,
        "m-1",
        &[confident_turn(0, 2_000, "Speaker A", 0.95)],
    )
    .await
    .expect("diarization");

    let all = ShortTurnEventsRepository::list_for_transcript(&pool, &ctx, "m-1", "t-short")
        .await
        .expect("all events");
    assert_eq!(all.len(), 1);
    assert!(all[0].transcript_aligned);
    assert!(
        ShortTurnEventsRepository::list_visible_annotations_for_meeting(&pool, &ctx, "m-1")
            .await
            .expect("annotations")
            .is_empty()
    );
}

#[tokio::test]
async fn short_turn_event_reads_and_deletes_are_workspace_scoped() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = db(&dir.path().join("event-tenant.db")).await;
    let local = ctx_for("local");
    let foreign = ctx_for("foreign");
    seed_meeting(&pool, "m-1", "local").await;
    seed_transcript(&pool, "t-short", "m-1", "hmm", 0.1, 0.4).await;
    SpeakerTurnsRepository::replace_for_meeting(
        &pool,
        &local,
        "m-1",
        &[confident_turn(0, 2_000, "Speaker A", 0.95)],
    )
    .await
    .expect("diarization");
    assert_eq!(
        ShortTurnEventsRepository::list_for_meeting(&pool, &local, "m-1")
            .await
            .expect("local")
            .len(),
        1
    );
    assert!(
        ShortTurnEventsRepository::list_for_meeting(&pool, &foreign, "m-1")
            .await
            .expect("foreign")
            .is_empty()
    );
    assert_eq!(
        ShortTurnEventsRepository::delete_for_meeting(&pool, &foreign, "m-1")
            .await
            .expect("foreign delete"),
        0
    );
    assert_eq!(
        ShortTurnEventsRepository::list_for_meeting(&pool, &local, "m-1")
            .await
            .expect("still local")
            .len(),
        1
    );
}

#[tokio::test]
async fn manual_event_assignment_survives_rerun_and_transcript_manual_is_independent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = db(&dir.path().join("manual-event.db")).await;
    let ctx = ctx_for("local");
    seed_meeting(&pool, "m-1", "local").await;
    seed_transcript(&pool, "t-long", "m-1", "long row", 0.0, 5.0).await;
    sqlx::query("UPDATE transcripts SET speaker_id = 'manual-main', speaker_assignment_method = 'manual' WHERE id = 't-long'")
        .execute(&pool)
        .await
        .expect("manual transcript");
    let turns = [
        confident_turn(0, 5_000, "Speaker A", 0.95),
        confident_turn(2_100, 2_400, "Speaker B", 0.95),
        confident_turn(6_000, 8_000, "Speaker B", 0.95),
    ];
    SpeakerTurnsRepository::replace_for_meeting(&pool, &ctx, "m-1", &turns)
        .await
        .expect("first pass");
    let events = ShortTurnEventsRepository::list_for_transcript(&pool, &ctx, "m-1", "t-long")
        .await
        .expect("events");
    assert_eq!(
        events.len(),
        1,
        "manual transcript must not suppress embedded event"
    );
    let speaker_a = SpeakerTurnsRepository::list_accepted_turns_for_meeting(&pool, &ctx, "m-1")
        .await
        .expect("turns")
        .into_iter()
        .find(|turn| turn.speaker_label == "Speaker A")
        .expect("speaker A");
    ShortTurnEventsRepository::assign_speaker(
        &pool,
        &ctx,
        "m-1",
        &events[0].id,
        &speaker_a.speaker_key,
    )
    .await
    .expect("manual event");

    SpeakerTurnsRepository::replace_for_meeting(&pool, &ctx, "m-1", &turns)
        .await
        .expect("rerun");
    let rerun = ShortTurnEventsRepository::list_for_transcript(&pool, &ctx, "m-1", "t-long")
        .await
        .expect("rerun event");
    assert_eq!(rerun.len(), 1);
    assert_eq!(rerun[0].assignment_method, AssignmentMethod::Manual);
    assert_eq!(
        rerun[0].speaker_key.as_deref(),
        Some(speaker_a.speaker_key.as_str())
    );
    let transcript_method: String =
        sqlx::query_scalar("SELECT speaker_assignment_method FROM transcripts WHERE id = 't-long'")
            .fetch_one(&pool)
            .await
            .expect("transcript method");
    assert_eq!(transcript_method, "manual");
}
