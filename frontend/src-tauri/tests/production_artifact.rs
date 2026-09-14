use app_lib::context::{AuthContext, RequestId, Role, TenantId, UserId};
use app_lib::database::repositories::speaker_turn::{SpeakerTurn, SpeakerTurnsRepository};
use app_lib::diarization::short_turn::VadEventCandidateInput;
use app_lib::diarization::types::AudioSource;
use app_lib::evaluation::production_artifact::{
    build_from_persisted_meeting, validate_artifact, ProductionConfigSnapshot,
    ARTIFACT_SCHEMA_VERSION,
};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

fn context(workspace: &str) -> AuthContext {
    AuthContext {
        tenant_id: TenantId::new(workspace),
        user_id: UserId::new("tester"),
        roles: vec![Role::Owner],
        request_id: RequestId::generate(),
    }
}

async fn database() -> (tempfile::TempDir, SqlitePool) {
    let directory = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(directory.path().join("artifact.db"))
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    (directory, pool)
}

async fn seed_production_state(pool: &SqlitePool) {
    sqlx::query("INSERT INTO meetings (id, workspace_id, title, created_at, updated_at) VALUES ('meeting-1', 'local', 'Test', '2026-09-14T00:00:00Z', '2026-09-14T00:00:00Z')")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO transcript_settings (id, workspace_id, provider, model, created_at, updated_at) VALUES ('settings-1', 'local', 'whisper', 'large-v3', '2026-09-14T00:00:00Z', '2026-09-14T00:00:00Z')")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO meeting_transcription_runs (id, meeting_id, workspace_id, created_at, completed_at, backend, model, model_version_or_hash) VALUES ('transcription-run-1', 'meeting-1', 'local', '2026-09-14T00:00:00Z', '2026-09-14T00:00:01Z', 'whisper', 'large-v3', NULL)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO transcripts (id, meeting_id, workspace_id, transcript, timestamp, audio_start_time, audio_end_time, duration, asr_confidence, transcription_run_id) VALUES ('transcript-1', 'meeting-1', 'local', 'hello', '2026-09-14T00:00:00Z', 0.1, 0.8, 0.7, 0.82, 'transcription-run-1')")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE transcript_settings SET model = 'medium', updated_at = '2026-09-14T00:00:02Z' WHERE id = 'settings-1'")
        .execute(pool)
        .await
        .unwrap();
    let mut runtime_config = ProductionConfigSnapshot::default();
    runtime_config.candidate_match.min_iou = 0.42;
    SpeakerTurnsRepository::replace_for_meeting_with_evidence_and_config(
        pool,
        &context("local"),
        "meeting-1",
        &[
            SpeakerTurn {
                start_ms: 0,
                end_ms: 1_000,
                speaker_label: "Speaker 1".into(),
                confidence: Some(0.9),
                speaker_key: "speaker_01".into(),
            },
            SpeakerTurn {
                start_ms: 500,
                end_ms: 1_500,
                speaker_label: "Speaker 2".into(),
                confidence: Some(0.8),
                speaker_key: "speaker_02".into(),
            },
        ],
        AudioSource::Mixed,
        &[VadEventCandidateInput {
            start_ms: 120,
            end_ms: 420,
            confidence: None,
            audio_source: AudioSource::Mixed,
        }],
        &runtime_config,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn builder_uses_persisted_production_state_without_second_inference() {
    let (_directory, pool) = database().await;
    seed_production_state(&pool).await;
    let before: (i64, i64, i64, String) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM transcripts), (SELECT COUNT(*) FROM speaker_turns), (SELECT COUNT(*) FROM meeting_production_snapshots), (SELECT id FROM meeting_production_snapshots WHERE meeting_id = 'meeting-1')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let artifact = build_from_persisted_meeting(&pool, &context("local"), "meeting-1")
        .await
        .unwrap();
    let second = build_from_persisted_meeting(&pool, &context("local"), "meeting-1")
        .await
        .unwrap();
    let after: (i64, i64, i64, String) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM transcripts), (SELECT COUNT(*) FROM speaker_turns), (SELECT COUNT(*) FROM meeting_production_snapshots), (SELECT id FROM meeting_production_snapshots WHERE meeting_id = 'meeting-1')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(before, after);
    assert_eq!(artifact, second);
    assert_eq!(artifact.schema_version, ARTIFACT_SCHEMA_VERSION);
    assert_eq!(artifact.transcripts.len(), 1);
    assert_eq!(artifact.raw_diarizer_turns.len(), 2);
    assert!(artifact.raw_diarizer_turns.iter().all(|turn| turn.overlap));
    assert_eq!(artifact.vad_events.len(), 1);
    assert_eq!(artifact.asr.backend, "whisper");
    assert_eq!(artifact.asr.model, "large-v3");
    assert_eq!(artifact.transcription_run_id, "transcription-run-1");
    assert_eq!(artifact.production_config.candidate_match.min_iou, 0.42);
    assert!(artifact.source_audio.path_hint.is_none());
    assert!(artifact
        .safety_observations
        .manual_override_violation_count
        .is_none());
    validate_artifact(&artifact, Some("meeting-1")).unwrap();
}

#[tokio::test]
async fn builder_is_workspace_scoped() {
    let (_directory, pool) = database().await;
    seed_production_state(&pool).await;
    assert!(
        build_from_persisted_meeting(&pool, &context("foreign"), "meeting-1")
            .await
            .is_err()
    );
}
