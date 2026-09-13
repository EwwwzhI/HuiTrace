-- Offline Speaker Foundation. `speaker_turns` remains the authoritative
-- diarizer timeline introduced by ADR-0034; this migration gives it stable
-- meeting-local keys and makes reconciled transcript assignments queryable.
-- All changes are additive so existing recordings remain readable.

CREATE TABLE IF NOT EXISTS speakers (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL DEFAULT 'local',
    -- A stable internal key such as speaker_01. It is never a person identity.
    speaker_key TEXT NOT NULL,
    -- User-editable label, initially "Speaker 1". Renaming never changes key.
    display_name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (workspace_id, meeting_id, speaker_key)
);
CREATE INDEX IF NOT EXISTS idx_speakers_workspace_meeting
    ON speakers(workspace_id, meeting_id, speaker_key);

-- Existing speaker_turns are the speaker-segment table. The default values keep
-- prior manual passes valid while allowing new offline backends to report their
-- provenance and future realtime passes to be marked provisional.
ALTER TABLE speaker_turns ADD COLUMN speaker_key TEXT;
ALTER TABLE speaker_turns ADD COLUMN audio_source TEXT NOT NULL DEFAULT 'mixed';
ALTER TABLE speaker_turns ADD COLUMN provisional INTEGER NOT NULL DEFAULT 0;
ALTER TABLE speaker_turns ADD COLUMN revision INTEGER NOT NULL DEFAULT 1;
ALTER TABLE speaker_turns ADD COLUMN segment_kind TEXT NOT NULL DEFAULT 'speech';
ALTER TABLE speaker_turns ADD COLUMN assignment_method TEXT NOT NULL DEFAULT 'diarization';
ALTER TABLE speaker_turns ADD COLUMN overlap INTEGER NOT NULL DEFAULT 0;

-- A transcript row is the actual persisted ASR segment in this application.
-- Nullable speaker_id preserves pre-diarization and legacy records. Automatic
-- reconciliation must never replace assignment_method='manual'.
ALTER TABLE transcripts ADD COLUMN speaker_id TEXT;
ALTER TABLE transcripts ADD COLUMN speaker_confidence REAL;
ALTER TABLE transcripts ADD COLUMN speaker_provisional INTEGER NOT NULL DEFAULT 0;
ALTER TABLE transcripts ADD COLUMN speaker_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE transcripts ADD COLUMN segment_kind TEXT;
ALTER TABLE transcripts ADD COLUMN audio_source TEXT;
ALTER TABLE transcripts ADD COLUMN speaker_assignment_method TEXT NOT NULL DEFAULT 'diarization';
ALTER TABLE transcripts ADD COLUMN speaker_overlap INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS idx_transcripts_workspace_meeting_speaker
    ON transcripts(workspace_id, meeting_id, speaker_id);

-- A durable status makes diarization an enhancement path: a missing model,
-- missing audio, or crashed sidecar is recorded without failing ASR/meeting
-- persistence. Older meetings naturally remain 'not_requested'.
ALTER TABLE meetings ADD COLUMN diarization_status TEXT NOT NULL DEFAULT 'not_requested';
ALTER TABLE meetings ADD COLUMN diarization_error TEXT;
