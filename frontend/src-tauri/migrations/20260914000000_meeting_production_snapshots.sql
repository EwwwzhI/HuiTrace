-- Phase 2D.1: preserve the evidence/configuration that was actually used by
-- the latest completed post-processing run. The JSON payloads contain derived
-- local evidence only; no audio bytes or filesystem paths are stored here.

CREATE TABLE IF NOT EXISTS meeting_production_snapshots (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL DEFAULT 'local',
    created_at TEXT NOT NULL,
    app_commit_sha TEXT NOT NULL,
    asr_backend TEXT,
    asr_model TEXT,
    asr_version_or_hash TEXT,
    diarization_backend TEXT NOT NULL,
    diarization_model TEXT NOT NULL,
    diarization_version_or_hash TEXT,
    production_config_json TEXT NOT NULL,
    vad_events_json TEXT NOT NULL,
    accepted_speakers_json TEXT NOT NULL,
    visible_speakers_json TEXT NOT NULL,
    long_transcript_speaker_corruption_count INTEGER,
    manual_override_violation_count INTEGER,
    UNIQUE (workspace_id, meeting_id)
);

CREATE INDEX IF NOT EXISTS idx_meeting_production_snapshots_workspace_meeting
    ON meeting_production_snapshots(workspace_id, meeting_id);
