-- Optional provider-native timing used only as derived reconstruction evidence.
-- Raw transcript text remains the source of truth. NULL means exact V1 fallback.
ALTER TABLE transcripts ADD COLUMN asr_timing_json TEXT;
