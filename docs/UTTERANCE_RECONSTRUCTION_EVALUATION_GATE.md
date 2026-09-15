# Utterance Reconstruction Ground Truth & Evaluation Gate

## Scope

Phase 3 measures the existing V1 and V2 reconstruction pipelines. It does not
add ASR, diarization, forced alignment, punctuation, language-model rewriting,
or speech-separation inference. Raw transcript text remains the source of truth;
reconstruction remains derived-on-read.

## Frozen architecture

```text
Production Meeting
       ↓
UtteranceReconstructionArtifact v1
       ↓
Blind Annotation
       ↓
UtteranceGroundTruth v1
       ↓
Review (SYSTEM SUGGESTION — NOT GROUND TRUTH)
       ↓
QA
       ↓
Frozen Benchmark
       ↓
V1 vs V2 + Error Taxonomy
       ↓
Phase 4 Decision
```

The reconstruction artifact embeds the exact persisted transcripts (including
provider-native timing), accepted speaker turns, ShortTurn events, V1 output,
V2 output, algorithm/config provenance, diagnostics, and diagnostic coverage
metrics. It binds the source Production Artifact and source media by SHA-256.
An internal integrity hash covers deterministic JSON with the hash field blank;
modified artifacts fail closed.

Export never reruns ASR, VAD, diarization, or ShortTurn inference. Parameter
sweeps replay only deterministic utterance reconstruction over frozen evidence.

## Ground Truth schema

- `GtSpeakerInterval`: ordinary or overlapping speaker activity using stable
  meeting-local `gt_speaker_XX` identities.
- `GtUtterance`: grouping interval, GT speaker, overlap, uncertain boundary
  flags, backchannel relationship, and scenario buckets.
- `GtUtteranceBoundary`: explicit utterance, speaker-handoff, or short-speech
  boundary. Uncertain boundaries are excluded from strict scoring.
- `dataset_split`: `calibration` or `evaluation`, assigned at meeting level.

Annotators never edit ASR text. Blind mode receives no raw ASR, diarizer, V1,
V2, timing, alignment, or boundary suggestions. Review is locked until Blind is
complete and labels all system data as non-Ground-Truth evidence.

The development workbench is available at:

```text
/dev/utterance-reconstruction-annotation
```

It reuses the existing WaveSurfer timeline, region editing, seek/zoom,
meeting-local speaker allocation, autosave, reopen, undo/redo, overlap and
uncertainty controls. Artifact and GT paths stay local.

## Metrics and matching

Speaker scoring excludes overlap and uncertain GT intervals.

```text
Coverage = assigned eligible GT utterances / eligible GT utterances
Assigned-only Accuracy = correct assigned / assigned
Selective Accuracy = correct assigned / eligible GT utterances
```

GT speakers are mapped one-to-one to production clusters using temporal overlap
from non-overlap, non-uncertain GT speaker intervals. Unknown, Ambiguous and
Mixed are abstentions, never silently converted to Single.

Boundary matching is deterministic nearest-neighbour, one-to-one matching.
Reports include separate ±250 ms and ±500 ms collars:

```text
Precision = matched predictions / predictions
Recall = matched references / references
F1 = harmonic mean
Over-segmentation = unmatched predictions / predictions
Under-segmentation = unmatched references / references
```

Matched timing errors report MAE, median, P90 and P95. Handoff boundaries are
scored separately. Parakeet NativeTokenEmission transitions also report median,
P90 and P95 error to GT handoff time. They are not described as word boundaries.

Special metrics cover backchannel main-utterance preservation, short-speech
boundary recall, overlap Mixed/abstention recall, and false Single-Speaker rate
on overlap. Lexical preservation is a hard 100% gate; any deletion, insertion,
rewrite, or reordering fails the benchmark.

The parameter sweep contains 144 frozen-evidence combinations:

- overlap ratio: 0.50, 0.60, 0.70, 0.80
- margin: 0.10, 0.20, 0.30
- alignment tolerance: 25, 50, 75, 100 ms
- true-overlap minimum: 50, 100, 150 ms

Production defaults are not changed by the sweep. Calibration meetings may be
used to select parameters; Evaluation meetings may only measure the frozen
choice.

## Running the benchmark

The desktop backend can export an artifact and run the benchmark. A CLI is also
available:

```powershell
cargo run -p huitrace --bin utterance_reconstruction_benchmark -- \
  --artifact C:\dataset\meeting.reconstruction.json \
  --ground-truth C:\dataset\meeting.utterance-gt.json \
  --output C:\dataset\reports\meeting
```

It writes:

- `utterance_reconstruction_report.json`
- `utterance_reconstruction_report.md`

## Smoke workflow

1. Select 3–5 real 10–20 minute meetings with 2–4 speakers.
2. Include handoff, backchannel, short speech, overlap, noise, Chinese, English,
   mixed language, and timing-unavailable examples across the set.
3. Finish normal production ASR, diarization and ShortTurn processing.
4. Export Production Artifact v2, then export Reconstruction Artifact v1 bound
   to that artifact and source-media hash.
5. Initialize one meeting-level GT file as Calibration or Evaluation.
6. Complete Blind speaker intervals, utterances and boundaries; close/reopen to
   verify autosave.
7. Unlock Review, inspect system evidence only for annotation mistakes, and do
   not automatically apply suggestions.
8. Complete Review and QA, then run the benchmark.
9. Verify V1 and V2 share the same artifact identity and lexical preservation is
   100% before interpreting any metric.
10. Treat this as workflow/metric smoke only, not a formal quality conclusion.

## Phase 4 decision rule

- Emission timing dominates errors → evaluate forced alignment.
- Attribution is strong but boundary metrics are weak → evaluate semantic or
  punctuation evidence as soft boundary evidence.
- True overlap dominates false Single claims → evaluate selective overlap
  handling or speech separation.
- Backchannel/short-speech metrics dominate → refine ShortTurn behavior.
- Existing rules are already strong → calibrate thresholds only.

No Phase 4 choice is made without Ground Truth results.
