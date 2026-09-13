# Short-turn real-meeting benchmark

Private audio, production artifacts, annotations, and benchmark reports belong
under the gitignored `local/` directory. Only schemas and examples are committed.

## Blind annotation

Create overlapping 5 s windows with a 4 s stride:

```text
cargo run -p huitrace --bin short_turn_export -- --audio meeting.wav --meeting-id MEETING_ID --production-artifact local/MEETING_ID.production.json --output evaluation/short_turn_dataset/local/MEETING_ID
```

The exporter writes two timelines. Annotators must complete
`annotation_windows.blind.jsonl` first, without candidate suggestions. The
second pass uses `annotation_windows.review.jsonl` to inspect omissions and
boundaries. Suggestions never overwrite first-pass labels.

Every event uses source-meeting timestamps and a stable
`ground_truth_event_id`. Adjacent windows must not create duplicate labels.
Use `annotation_uncertain=true` when speaker, acoustic kind, or overlap is not
reliably decidable; uncertain rows remain in case analysis but are excluded
from hard metrics and the representative-data count.

## Ground truth and production artifacts

`manifest.jsonl` contains only ground-truth events and controls. It references
one immutable production artifact per meeting through
`production_artifact_path`; it does not copy transcripts, diarizer turns, VAD
events, or accepted speakers into every row. An optional
`production_artifact_sha256` pins the exact file.

The artifact schema is shown in `production_artifact.example.json`. It records
identity, source-audio metadata, app commit, ASR/diarization backend and model,
the complete production config snapshot, full transcripts, raw diarizer turns,
VAD events, accepted/visible speakers, safety counters, and original metadata.

Production replay loads each meeting artifact once, runs candidate extraction,
refinement, speaker acceptance, and materialization once, and only then matches
the complete prediction set to ground truth:

```text
cargo run -p huitrace --bin short_turn_benchmark -- --dataset evaluation/short_turn_dataset/local --mode production-artifact-replay
```

Changing `evidence_origin` cannot enable production replay. A valid artifact
path and schema are required. `--mode evidence` remains an annotation-associated
regression mode and is never described as end-to-end. `--mode pipeline`
remains unsupported until the desktop service layer can be reused directly.

## Representative-data gate

The centralized coverage policy requires at least 100 scorable rows, 60 true
short events, 20 short-speech events, 15 backchannels, 20 noise/negative
controls, 10 true short events in each 100–300, 300–500, 500–800, and
800–1200 ms bucket, three meetings, two meetings with at least two speakers,
eight overlap cases, and eight speaker-handoff cases.

Below the gate the only architecture decision is
`INSUFFICIENT_REPRESENTATIVE_DATA`; model selection and threshold tuning are
not allowed.
