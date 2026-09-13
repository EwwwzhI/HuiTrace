# Phase 2C — Short-turn materialization and benchmark gate

Phase 2C remains model-free. It closes the gap between finding a short event
and representing it without corrupting ASR segmentation.

## Timeline semantics

`Transcript` is the ASR text segment. `ShortTurnEvent` is a meeting-local,
derived semantic/speaker interval. A transcript-aligned short event projects
its decision onto the transcript row and is not rendered twice. An event inside
a long transcript leaves the transcript's dominant speaker unchanged and is
rendered as a seekable nested timeline annotation. No text is split or guessed
without word timestamps.

Automatic events are replaced in the same transaction as successful speaker
turns. A failed backend never reaches that transaction, so the last successful
events remain. Manual event speaker corrections have stable event ids and are
not overwritten by reruns. Transcript-level manual assignment and event-level
manual assignment are independent. Talk time remains based only on accepted
`speaker_turns`; events are annotations and are never added again.

Portable meeting artifacts keep the domains separate:

- `transcripts.json`
- `short_turn_events.json`

## Materialization policy

- `Speech` and `Backchannel`: stored and user-visible.
- `Unknown`: stored internally only with strong direct speech evidence; hidden.
- `Noise`: not materialized.
- `NonSpeechVocalization`: not proactively produced without acoustic evidence.

Candidate sources are persisted as deterministic JSON arrays. Multiple events
may reference one transcript. Non-overlapping adjacent events never merge.

## Benchmark modes and decision gate

Evidence Mode consumes annotated transcript, diarizer, and VAD evidence from
`manifest.jsonl`. It reuses the exact production `SpeakerAcceptancePolicy` and
a strict candidate matcher (IoU or high ground-truth coverage plus center
tolerance). Pipeline Mode is intentionally reported as unsupported until the
CLI can use the application's actual model lifecycle; Evidence Mode is never
presented as end-to-end.

Do not make a Phase 2D model decision below this recommended floor:

- at least 100 short events;
- at least 3 meetings;
- at least 2 speaker/meeting scenarios;
- coverage across all four short duration buckets;
- backchannels, normal short speech, noise/false triggers, overlap, and handoff.

Below the floor the result is `INSUFFICIENT_DATA`. Once sufficient:

- low candidate recall → improve VAD/extraction;
- good recall but poor kind metrics → consider AED/acoustic classification;
- good recall/kind but poor speaker accuracy → consider meeting-local speaker embeddings;
- segmentation/overlap errors → consider segmentation-focused models.

No model is selected merely because an aggregate metric is low.

## Local annotation helper

Run `short_turn_export` against meeting audio to export candidate WAV clips with
400 ms context and a `manifest.template.jsonl`. Use a gitignored destination,
for example `evaluation/short_turn_dataset/local`. Ground truth fields are left
`null` for human annotation.
