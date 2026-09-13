# Short-turn real-audio benchmark

Large or sensitive audio files are intentionally not committed. Put local files
beside `manifest.jsonl` (or reference absolute paths), then run:

```text
cargo run -p mityu --bin short_turn_benchmark -- --dataset evaluation/short_turn_dataset
```

Each JSONL row requires `audio_path`, `start_ms`, `end_ms`,
`duration_bucket`, `ground_truth_kind`, `ground_truth_speaker`,
`transcript_text`, and `notes`. Optional production evidence fields are
`transcript_start_ms`, `transcript_end_ms`, `asr_confidence`,
`accepted_speakers`, and `diarizer_turns` (`start_ms`, `end_ms`,
`speaker_key`, optional `confidence`). `expected_visible_speakers` enables the
visible-meeting-speaker false-new-speaker metric. Missing confidence must be omitted or
`null`; never insert a neutral placeholder.

The report includes overall and duration-bucket classification metrics plus
candidate recall for Transcript, DiarizerTurn, VadEvent, and their union.
