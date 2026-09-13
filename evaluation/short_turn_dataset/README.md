# Short-turn real-audio benchmark

Private audio and local artifacts stay under the gitignored `local/` directory.
Create candidate-independent, overlapping review windows with:

```text
cargo run -p huitrace --bin short_turn_export -- --audio meeting.wav --meeting-id MEETING_ID --production-artifact app-run.json --output evaluation/short_turn_dataset/local
```

`annotation_windows.jsonl` covers the complete meeting timeline. Candidate
suggestions are hints only. Annotators review every window and create a separate
`manifest.jsonl` with one `ground_truth_event` row per true short event, noise
trigger, ordinary non-short control, overlap, or handoff. A missed event is
added even when `candidate_suggestions` is empty.

Rows require a unique `id`, `record_type=ground_truth_event`,
`recall_eligible=true`, non-empty `meeting_id`, valid timing, a duration bucket
derived from timing, and `ground_truth_kind` (`short_speech`, `backchannel`,
`noise`, `non_speech_vocalization`, or `ordinary_speech_control`). `speech` is
a read alias; new short-event data uses `short_speech`. Non-short controls use
`duration_bucket=non_short_control`. Unknown confidence is `null`/omitted,
never a constant.

Each row carries real meeting evidence: transcript timing/text/confidence where
applicable, complete raw diarizer turns, VAD events, accepted speakers,
expected visible speakers, and overlap/handoff tags. Suggestions, production
evidence, ground truth, and replay predictions remain distinct.

```text
cargo run -p huitrace --bin short_turn_benchmark -- --dataset evaluation/short_turn_dataset --mode evidence
```

Use `--mode production-artifact-replay` to require
`evidence_origin=production_artifact`. Neither replay mode runs ASR or
diarization. `--mode pipeline` explicitly fails because directly wiring the
desktop lifecycle into this CLI would duplicate model/inference orchestration.

The gate requires 100 annotated event/control rows, three meetings, two
speaker/meeting scenarios, all four buckets, short speech, backchannel, noise,
overlap, and handoff. Below it the decision is
`INSUFFICIENT_DATA_FOR_PHASE_2D`.
