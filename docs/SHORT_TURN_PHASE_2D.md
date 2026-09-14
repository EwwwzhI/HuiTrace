# Phase 2D — Real-meeting benchmark and targeted model decision

Phase 2D diagnoses the first user-visible failure in the short-turn pipeline.
It does not assume that speaker embeddings, an acoustic classifier, or a new
diarization backend are required.

## Frozen baseline

`PHASE_2C1_FROZEN_BASELINE` is commit
`7b34d7b7d422deeafeb21f07449f8a3ca8c5f60a`.

The frozen default configuration is:

- `ShortTurnConfig`: min candidate 100 ms, very short 500 ms, max short turn
  1200 ms, prototype minimum 1500 ms, duration boundaries 299/499/799 ms,
  high/weak confidence 0.75/0.40, nearby gap 350 ms, lexical backchannels on.
- `ShortCandidateVadConfig`: min speech 100 ms, redemption 250 ms, max
  candidate 1200 ms.
- `SpeakerAcceptancePolicy`: max overlap ratio 0.50, confident longest turn
  1200 ms, or 4000 ms across two turns when confidence is unavailable.
- `CandidateMatchConfig`: IoU 0.30, GT coverage 0.75, center tolerance 200 ms.
- `ShortTurnMaterializationPolicy`: aligned IoU 0.75, strong internal evidence
  0.75.
- Short-candidate VAD: Silero through `ContinuousVadProcessor`, 16 kHz,
  positive/negative thresholds 0.50/0.35.
- ASR is application-configurable whisper.cpp; a production artifact must state
  the actual model and hash. The repository pins large-v3 at SHA-256
  `64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2`.
- Diarization is sherpa-onnx using pyannote segmentation-3.0
  (`220ad67c...e1079`) and 3D-Speaker CAM++ (`f682b514...fd11`).

Development-host snapshot at baseline review: Windows 11 25H2, AMD Ryzen 5
9600X, 31.1 GiB RAM. The verified commands in this phase used the CPU-only
Rust build; an NVIDIA GPU was present but not enabled. A meeting artifact, not
this host snapshot, is authoritative for each benchmark run.

## Immutable production artifact

Schema version 2 contains an artifact, meeting, and transcription-run id,
source-audio duration and optional hash, creation time, app commit,
ASR/diarization backend and model,
the complete production policy snapshot, full transcripts, raw diarizer turns,
VAD events, accepted and visible speakers, safety counters, and original
metadata. The benchmark computes the artifact file SHA-256 and reports it; a
manifest may pin that hash.

Ground-truth rows contain labels and a `meeting_id` reference only. Production
replay rejects a row with no artifact path even if `evidence_origin` says
`production_artifact`. Each artifact is loaded once and the complete meeting is
processed once before matching predictions to labels. Evidence replay remains
available for deterministic regression, but is not end-to-end.

Matching is one-to-one and uses temporal geometry only: IoU, GT/prediction
coverage, center distance, boundary error, and duration difference. Kind,
speaker, expected materialization, and annotation tags never select a pair.
Same-time/same-duration multi-speaker groups are reported as ambiguous and use
event-count plus speaker-set evaluation rather than label-assisted pairing.

## Annotation methodology

The exporter creates blind and suggestion-assisted versions of the same 5 s / 4
s stride windows. First-pass labels are made from the blind file. Suggestions
are shown only during review. All identities use source-meeting timestamps plus
a stable `ground_truth_event_id`; obvious high-IoU duplicates are rejected.
`annotation_uncertain` preserves ambiguous cases without forcing them into
precision, recall, or speaker accuracy.

## Representative-data gate

The centralized policy requires 100 scorable samples, 60 true short events, 20
short-speech, 15 backchannels, 20 negative controls, 10 true short events in
each duration bucket, three meetings, two multi-speaker meetings, eight overlap
cases, and eight handoffs. Counts cannot be filled with uncertain events.

No private real-meeting dataset is present in this repository. Therefore the
current evidence-supported architecture decision is:

`INSUFFICIENT_REPRESENTATIVE_DATA`

This decision forbids a model spike and is not evidence that the model-free
pipeline is sufficient or insufficient.

## Metrics and first-failure taxonomy

The report separates transcript, diarizer, VAD, and union candidate recall by
duration bucket from candidate overgeneration. A negative candidate refined to
Noise/Unknown and hidden is a successful downstream rejection. A negative
candidate accepted as speech/backchannel is a `kind_error`.

Scorable outputs include per-class precision/recall/F1 and confusion matrix;
speaker accuracy, unattributed and wrong-existing-speaker rates by duration and
overlap/handoff/embedded context; false-new and missed-real-speaker rates;
visible precision/recall, embedded accuracy, duplicates, false embedded events,
long-transcript corruption and manual-override violations; and confidence
reliability bins. Reliability bins make no calibration claim.

Each failing event receives one primary cause: `candidate_miss`, `kind_error`,
`speaker_attribution_error`, `speaker_acceptance_error`,
`materialization_error`, `segmentation_overlap_error`, or
`annotation_uncertain`. Candidate overgeneration is a secondary efficiency
finding and does not override the first user-visible error.

## Model-free sensitivity and Pareto frontier

Threshold sensitivity is intentionally marked not run until whole meetings can
be split into development and final-evaluation sets. Events from one meeting
must never cross that boundary. Any future frontier must jointly report
candidate recall, kind F1, speaker accuracy, false-new speakers, noise false
accepts, materialization safety, processing time, and preserve zero long-row
corruption and manual-override violations.

## Model spike, deployment cost, and license

No model spike was executed. Accuracy gain, model/download/install size, RAM,
CPU/GPU cost, load/inference time, whole-meeting latency, and license impact are
therefore not applicable. If the data gate later passes, only the capability
selected by the dominant error class may be evaluated, and only on an
experimental path against both the frozen and tuned model-free baselines.

## Known limitations

- There is no committed representative real-meeting dataset or benchmark
  output, so no real-world recall or attribution number can be claimed.
- Phase 2D.1 wires production run snapshots and local artifact export into the
  desktop lifecycle; see `SHORT_TURN_PHASE_2D_DATASET.md`. Existing meetings
  processed before that migration need a normal production reprocessing run
  before they can be exported.
- Pipeline mode remains unsupported to avoid duplicating inference lifecycle.
- Materialization safety counters must be captured by production artifacts;
  annotated evidence replay explicitly reports them as not exercised.

## Phase 2D.2 local annotation workspace

`/dev/short-turn-annotation` is an unlinked development/evaluation route. It
is available in development builds, or when
`NEXT_PUBLIC_ENABLE_SHORT_TURN_ANNOTATION=true` is supplied to an evaluation
build. It stores a meeting-local `annotation_session.json` and
`annotations.draft.json` next to the already-exported windows and artifact.
Neither source media paths nor speaker-map descriptions are emitted to a
benchmark manifest.

The normal Meeting Details page now provides the minimal bridge into this
evaluation workflow. Under the same development/evaluation feature gate,
**Evaluation Tools** can export the existing persisted Production Artifact and
open `/dev/short-turn-annotation`. Export stays disabled until the shared
diarization lifecycle is `done`; a completed zero-turn result is still valid.
The bridge consumes the existing lifecycle and export command and never launches
inference or builds an artifact in the browser.

The operator sequence is: record/import a real meeting → complete transcription
→ Identify Speakers → export Production Artifact from Meeting Details → run
`short_turn_export` → open and initialize the annotation workspace → Blind →
Review → QA → save/export → `short_turn_dataset_check` → frozen Production
Artifact replay. This is the supported UI entry path for the real-meeting human
annotation smoke workflow.

The first pass reads only `annotation_windows.blind.jsonl`; the Tauri boundary
clears all suggestion fields before returning its viewport. Review is locked
until every blind viewport has `reviewed_blind` status and then reads only the
review window suggestions. Production artifacts are validated but never used to
run inference from this workspace.

Use **Run QA** before **Export Benchmark Manifest**. Export replaces the current
meeting's rows in the dataset-root `manifest.jsonl`, also writes a convenient
per-meeting copy, and invokes the existing `evaluation::dataset::check_dataset`
logic. Continue frozen replay with the existing command:

```text
cargo run -p huitrace --bin short_turn_benchmark -- --dataset evaluation/short_turn_dataset/local --mode production-artifact-replay
```


## Phase 2D.2a-final: Blind Completion Integrity

Pending events block completion in every strictly intersecting source-time viewport.
The UI checks before completion; the backend checks completed window states before
save. Any pending event anywhere blocks Review evidence, even in an old session
whose Blind window statuses are already marked complete. Review completion also
rejects review_pending; final QA confirmation checks remain in place.

## Phase 2D.2a-final: Review Export Integrity

Export requires every expected Blind ID complete, every expected Review ID complete,
QA pass and persisted current state. Review progress is visible and disables export
when incomplete. Backend checks, merged-manifest parsing and dataset validation run
before either manifest is replaced; root-write errors roll back the meeting copy.
A missing status, empty window set, 199/200 Review or an unsaved edit cannot export.
This does not provide crash atomicity across two files.

## Phase 2D.2a-final: Speaker Alignment Integrity

Only independent ordinary_speech_control intervals >1200 ms with a known speaker,
no annotation uncertainty and no overlap tag establish the frozen mapping. The shared
GroundTruthKind preserves original labels; short_speech and backchannel never enter
the matrix. No-reference speakers are explicitly unmapped, excluded from attribution
denominators and listed in per-meeting speaker_alignment coverage. Dataset coverage
uses original GT identities. No prediction or ShortTurn row can repair the mapping.

## Partial Speaker Alignment

Speaker attribution and speaker acceptance intentionally have different scorability.
Attribution continues to score only events whose GT speaker has a frozen production
mapping; unmapped events remain outside its denominator. Speaker acceptance requires
the complete meeting-level expected speaker set to map into the production namespace.
If any expected GT speaker is unmapped, acceptance is unscorable rather than correct
or erroneous, and it cannot create `speaker_acceptance_error` or select
`FIX_SPEAKER_ACCEPTANCE`. A meeting with no expected speaker GT is also unscorable.
False-new and missed-real rates use only scorable meetings as their denominator.

For formal collection, each real speaker should have 1–2 clear, non-overlapping,
non-uncertain `ordinary_speech_control` references, preferably at least 2 seconds.
This is collection guidance only; hard reference eligibility remains strictly
greater than 1200 ms.

See [workspace integrity details](SHORT_TURN_ANNOTATION_WORKSPACE.md) and the
[implementation and validation report](SHORT_TURN_PHASE_2D_2A_FINAL_REPORT.md).
The synthetic CLI workflow now covers export through Frozen Replay. Formal collection
readiness still requires the documented real-meeting human annotation smoke workflow.
