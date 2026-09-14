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
