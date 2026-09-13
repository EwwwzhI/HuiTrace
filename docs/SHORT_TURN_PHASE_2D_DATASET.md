# Phase 2D.1 — Representative dataset activation

Phase 2D.1 turns completed HuiTrace meetings into a private, repeatable
evaluation dataset. It does not tune the frozen short-turn algorithm, train a
model, or add a speaker model. Until the gate below passes, the only supported
architecture conclusion is `INSUFFICIENT_REPRESENTATIVE_DATA`.

## Frozen baseline and replay semantics

`PHASE_2C1_FROZEN_BASELINE` remains the immutable first baseline. Its defaults
for candidate extraction, lexical backchannels, duration weighting, speaker
confidence, speaker acceptance, matching, and materialization are unchanged.

- **Frozen Production Replay** reads the artifact's production configuration
  and recorded accepted/visible speaker state. It answers: “How did HuiTrace
  behave in that production run?”
- **Counterfactual Replay** reads the same immutable raw transcript,
  diarization, and VAD evidence, but applies a complete experiment config. It
  recomputes accepted speakers, the prototype set, short-turn decisions, and
  materialization. It never overwrites the artifact.

Counterfactual replay exists to make later model-free experiments sound. It is
not permission to tune thresholds before representative data, a completed
baseline, and a meeting-level development/final split exist.

## Production lifecycle and artifact export

After a successful production diarization/short-turn transaction, HuiTrace
persists a tenant-scoped run snapshot in the same transaction as the raw
speaker evidence and short-turn results. The snapshot includes the exact VAD
events used, production configuration, accepted speakers, visible speakers,
backend/model provenance, app commit, and observation status for safety
counters. Export reads that snapshot plus persisted transcript and raw
diarizer rows. It does not invoke ASR, diarization, VAD, short-turn refinement,
or materialization again.

The registered developer command is:

```text
api_export_short_turn_production_artifact({ meetingId: "MEETING_ID" })
```

The renderer supplies only the meeting id. HuiTrace resolves it in the current
workspace and opens a native save dialog named
`MEETING_ID.production.json`. Cancel returns no path. Export remains local and
does not upload anything. Prefer
`evaluation/short_turn_dataset/local/MEETING_ID/` as the selected directory.

Only meetings processed after the Phase 2D.1 migration have the required run
snapshot. Re-run the normal production diarization command for an older
meeting before exporting; export itself will still perform no inference.

Validate a file with the same schema validator used by benchmark loading:

```text
cargo run -p huitrace --bin short_turn_artifact -- validate evaluation/short_turn_dataset/local/MEETING_ID/MEETING_ID.production.json
```

Artifact schema version is `1`. Unsupported future versions are rejected;
they are never silently reinterpreted. Validation covers identities and
commit provenance, backend/model/config completeness, unique transcript ids,
nonempty diarizer speaker keys, duration and timing bounds, meeting identity,
and confidence values in `[0, 1]` or `null`.

## Privacy rules

Artifacts contain meeting text, speaker timing, and meeting structure and must
be treated as sensitive data.

1. Keep artifacts, annotations, clips, source audio, and reports under the
   gitignored `evaluation/short_turn_dataset/local/` tree.
2. Do not commit or upload them to GitHub or any cloud service.
3. The export command only writes a user-selected local file; no upload is
   implemented.
4. Artifact JSON never embeds source audio and defaults the source path hint to
   `null`.
5. Do not add usernames, Windows account names, or absolute local paths to an
   artifact. Source hashes remain optional.
6. A safety observation that was not measured is `null`, never a fabricated
   zero.

## Dataset directory

Use one directory per source meeting:

```text
evaluation/short_turn_dataset/local/
  meeting_001/
    source.wav
    meeting_001.production.json
    annotation_windows.blind.jsonl
    annotation_windows.review.jsonl
  meeting_002/
    ...
  manifest.jsonl
```

The root `manifest.jsonl` contains ground-truth events and controls and points
to meeting artifacts by relative path. Repository-tracked files are limited to
schemas, documentation, synthetic examples, and tests.

## Collection target

Prefer 5–8 natural meetings (at least 6), 150–250 scorable samples, at least
100 true short events, mostly 2–4 speakers, and at least three multi-speaker
meetings. Do not fill quotas with repeated scripted “嗯/对”. Useful natural
coverage includes quiet meetings, ordinary office noise, feedback-heavy
conversation, interruption/overlap/crosstalk, and genuine Chinese/English
mixing when it occurs.

## Blind annotation workflow

After exporting the production artifact, create 5 s windows with a 4 s stride:

```text
cargo run -p huitrace --bin short_turn_export -- \
  --audio evaluation/short_turn_dataset/local/meeting_001/source.wav \
  --meeting-id meeting_001 \
  --production-artifact evaluation/short_turn_dataset/local/meeting_001/meeting_001.production.json \
  --output evaluation/short_turn_dataset/local/meeting_001
```

The audio is decoded only to produce local clips. Candidate suggestions are
derived from the validated artifact's frozen raw evidence and configuration;
the tool does not run a new VAD/ASR/diarization pass.

1. Annotate `annotation_windows.blind.jsonl` without viewing production
   candidate suggestions.
2. Review with `annotation_windows.review.jsonl`. This pass may find omissions,
   adjust boundaries, or flag ambiguity. It must not delete truth because the
   system missed it or manufacture truth because the system suggested it.
3. Consolidate unique source-meeting timestamps into the dataset-root
   `manifest.jsonl`. Overlapping windows must not duplicate one physical event.
4. Run dataset QA after every meeting and manually review every reported
   possible duplicate pair.

Allowed labels are `short_speech`, `backchannel`, `noise`,
`non_speech_vocalization`, and `ordinary_speech_control`.
`annotation_uncertain=true` is an independent flag, not a replacement label.

Speaker ids are meeting-local (`speaker_01`, `speaker_02`, ...). The same person
must retain one id within a meeting; ids need not match across meetings. Phase
2D.1 does not create a cross-meeting voice identity database.

Mark events uncertain when speech, speaker, or boundaries cannot be decided
reliably—for example complete overlap, clipping, distortion, or an inaudible
very short event. Keep uncertain rows for error review, but they are excluded
from hard accuracy, representative quotas, and the speaker denominator.

## Dataset QA and representative gate

Run:

```text
cargo run -p huitrace --bin short_turn_dataset_check -- --dataset evaluation/short_turn_dataset/local
```

The JSON report shows achieved counts, required policy, missing deltas, and
possible duplicates. Duplicate detection requires the same meeting and
compatible kind, then uses high IoU or high IoU plus very close centers. It
reports event id pairs for human review and never deletes them automatically.

The minimum gate is:

- 100 scorable samples; 60 true short events
- 20 short-speech; 15 backchannels; 20 negative controls
- 10 true short events in each 100–300, 300–500, 500–800, and 800–1200 ms bucket
- 3 meetings; 2 multi-speaker meetings
- 8 overlap and 8 speaker-handoff cases
- no unresolved duplicate warnings

These are minimums, not the desired collection target. A dataset containing
only three meetings is suitable for baseline diagnosis, not strong threshold
conclusions.

## Benchmark commands

Once the gate first passes, run only the frozen production baseline:

```text
cargo run -p huitrace --bin short_turn_benchmark -- \
  --dataset evaluation/short_turn_dataset/local \
  --mode production-artifact-replay
```

The report includes source-specific candidate recall and duration breakdown,
candidate overgeneration, kind precision/recall/F1 and confusion, speaker and
acceptance metrics, materialization safety, and a first-failure error taxonomy.
Before changing anything, replay 5–10 sample ids from each major error class to
verify annotation, timestamps, speakers, matching, and root-cause assignment.

Counterfactual mode is reserved for later model-free sensitivity work and
requires an explicit complete configuration file:

```text
cargo run -p huitrace --bin short_turn_benchmark -- \
  --dataset evaluation/short_turn_dataset/local \
  --mode counterfactual-replay \
  --experiment-config path/to/complete-production-config.json
```

Do not place any event from the same meeting in both development and final
evaluation. Prefer at least six meetings before a 3/3 or 4/2 meeting-level
split. Final-evaluation meetings remain untouched during tuning.

## Current collection progress

No private real-meeting dataset is available in this repository, so no real
accuracy is reported and the checked-in progress is:

```text
Meetings:              0 / 3
Scorable samples:      0 / 100
True short events:     0 / 60
Short speech:          0 / 20
Backchannel:           0 / 15
Negative controls:     0 / 20
100–300 ms:            0 / 10
300–500 ms:            0 / 10
500–800 ms:            0 / 10
800–1200 ms:           0 / 10
Multi-speaker meetings:0 / 2
Overlap:               0 / 8
Speaker handoff:       0 / 8
Uncertain:             0 (excluded)
```

Representative data gate: `INSUFFICIENT_REPRESENTATIVE_DATA`.

Frozen real baseline: not run. Threshold tuning: not run. Model spike: not
run. Those states are intentional and must remain so until their prerequisites
are supported by real data.
