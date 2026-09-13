-- Phase 2C.1: keep automatic inference separate from the effective/manual value.
ALTER TABLE short_turn_events ADD COLUMN automatic_speaker_key TEXT;
ALTER TABLE short_turn_events ADD COLUMN automatic_speaker_confidence REAL;

-- Existing automatic rows already contain the latest inferred value.  A legacy
-- manual row has no trustworthy recoverable automatic value, so leave it NULL
-- instead of reconstructing one from a different algorithm.
UPDATE short_turn_events
SET automatic_speaker_key = speaker_key,
    automatic_speaker_confidence = speaker_confidence
WHERE assignment_method != 'manual';
