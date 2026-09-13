-- Phase 2B keeps ASR evidence separate from speaker-attribution evidence.
-- Providers that do not expose a calibrated confidence store NULL.
ALTER TABLE transcripts ADD COLUMN asr_confidence REAL;
