-- Phase 2C: materialize short semantic events independently from ASR rows.
-- A transcript is text segmentation; a short_turn_event is a derived timeline
-- annotation. Keeping them separate prevents an embedded 300 ms response from
-- changing the speaker of an enclosing multi-second transcript.

CREATE TABLE IF NOT EXISTS short_turn_events (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL DEFAULT 'local',
    transcript_id TEXT REFERENCES transcripts(id) ON DELETE SET NULL,
    start_ms INTEGER NOT NULL,
    end_ms INTEGER NOT NULL,
    segment_kind TEXT NOT NULL,
    kind_confidence REAL NOT NULL,
    speaker_key TEXT,
    speaker_confidence REAL,
    candidate_sources TEXT NOT NULL,
    audio_source TEXT NOT NULL DEFAULT 'mixed',
    assignment_method TEXT NOT NULL DEFAULT 'short_turn_refinement',
    revision INTEGER NOT NULL DEFAULT 1,
    transcript_aligned INTEGER NOT NULL DEFAULT 0,
    user_visible INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (end_ms > start_ms)
);

CREATE INDEX IF NOT EXISTS idx_short_turn_events_workspace_meeting
    ON short_turn_events(workspace_id, meeting_id, start_ms, end_ms);
CREATE INDEX IF NOT EXISTS idx_short_turn_events_workspace_transcript
    ON short_turn_events(workspace_id, transcript_id, start_ms);
