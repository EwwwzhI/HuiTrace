-- Phase 2D.1a: bind persisted transcript rows and production snapshots to the
-- ASR run that actually produced them. Workspace settings are mutable and are
-- therefore not valid artifact provenance.
CREATE TABLE meeting_transcription_runs (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    completed_at TEXT NOT NULL,
    backend TEXT NOT NULL,
    model TEXT NOT NULL,
    model_version_or_hash TEXT,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE,
    CHECK (length(trim(backend)) > 0),
    CHECK (length(trim(model)) > 0)
);

CREATE INDEX idx_meeting_transcription_runs_meeting
    ON meeting_transcription_runs(workspace_id, meeting_id, completed_at);

ALTER TABLE transcripts ADD COLUMN transcription_run_id TEXT
    REFERENCES meeting_transcription_runs(id);

CREATE INDEX idx_transcripts_transcription_run
    ON transcripts(workspace_id, meeting_id, transcription_run_id);

ALTER TABLE meeting_production_snapshots ADD COLUMN transcription_run_id TEXT
    REFERENCES meeting_transcription_runs(id);
