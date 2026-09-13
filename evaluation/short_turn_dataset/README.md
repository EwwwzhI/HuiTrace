# Short-turn real-audio benchmark

Large or sensitive audio files are intentionally not committed. Put local files
beside `manifest.jsonl` (or reference absolute paths), then run:

```text
cargo run -p huitrace --bin short_turn_benchmark -- --dataset evaluation/short_turn_dataset --mode evidence
```

Each JSONL row requires `meeting_id`, `start_ms`, `end_ms`,
`duration_bucket`, `ground_truth_kind`, `ground_truth_speaker`,
`transcript_text`, and `notes`. Optional production evidence fields are
`transcript_start_ms`, `transcript_end_ms`, `asr_confidence`,
`ground_truth_accepted_speakers`, `vad_events`, and `diarizer_turns` (`start_ms`, `end_ms`,
`speaker_key`, optional `confidence`). The legacy `accepted_speakers` key is
accepted as an alias. `expected_visible_speakers` enables the
visible-meeting-speaker false-new-speaker metric. Missing confidence must be omitted or
`null`; never insert a neutral placeholder.

The report includes overall and duration-bucket classification metrics plus
candidate recall for Transcript, DiarizerTurn, VadEvent, and their union.
Candidate matching requires meaningful IoU or coverage; any overlap is not a
hit. The report also includes materialization precision/recall, false embedded
event rate, and embedded speaker accuracy.

Pipeline Mode is an explicit interface but currently refuses to run because the
CLI is not wired to the desktop model lifecycle. This prevents Evidence Mode
from being mislabeled end-to-end.

To create local clips for annotation:

```text
cargo run -p huitrace --bin short_turn_export -- --audio meeting.wav --output evaluation/short_turn_dataset/local
```

The local directory and WAV files are gitignored. Do not make a model decision
until the dataset has at least 100 events from at least three meetings with
duration, kind, noise, overlap, and handoff coverage. Smaller datasets report
`INSUFFICIENT_DATA`.
Use `tags` for required scenario coverage, including `overlap` and
`speaker_handoff`. Each Evidence Mode row should carry the meeting's complete
raw diarizer-turn evidence so the shared acceptance policy sees production-like
speaker history rather than only the labelled clip.
