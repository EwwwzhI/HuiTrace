# Phase 2C.1 — Validation closure and evidence integrity

Phase 2C.1 adds no model. It makes materialized short events durable and makes
the Phase 2D gate depend on recall-valid, production-like evidence.

## Event identity

The initial deterministic id hashes meeting id and 50 ms-quantized boundaries.
Transcript ids and candidate provenance are deliberately excluded: ASR
retranscription, transcript split/merge, and source availability do not define
a physical acoustic event.

Hashes alone cannot safely tolerate arbitrary jitter without colliding nearby
events. Before replacement, inferred and previous events therefore undergo a
deterministic one-to-one reconciliation. Compatible speech/backchannel/unknown
events match only with at least 50% shorter-event overlap and at most 200 ms
center distance. Edges prefer IoU, then center distance, then a shared
transcript relation and stable indexes. This preserves manual annotations across
±20/40/60 ms jitter while preventing two inferred events from claiming one old
identity. Unmatched manual events remain; a later matching inference updates
them instead of creating a duplicate.

## Automatic and manual speaker assignment

`automatic_speaker_key` and `automatic_speaker_confidence` persist the latest
`ShortTurnRefiner` result separately from the effective `speaker_key` and
`speaker_confidence`. A rerun updates automatic evidence and all derived event
metadata. When a manual override exists, only the effective speaker stays
manual. Restore removes that override and exposes the stored automatic result;
it does not infer from maximum speaker-turn overlap and does not change kind,
timing, provenance, visibility, or transcript relation. Repeated restore is a
no-op, including revision.

The migration copies existing non-manual values into the automatic columns.
Legacy manual rows remain `NULL` because their prior automatic result cannot be
recovered honestly.

## Candidate evidence coherence

Merged candidates retain direct diarizer observations as source-level tuples.
After final boundaries are known, the extractor recomputes the direct/dominant
speaker, its confidence and coverage as one tuple, plus previous/next speaker,
gaps, overlap, and true multi-speaker overlap. Confidence from one source can no
longer be paired with coverage from another.

## Candidate-independent ground truth

`short_turn_export` emits fixed 5 s windows at a 4 s stride over the complete
meeting. Production VAD candidates appear only as labeled annotation
suggestions. Annotators review every window and write a separate manifest row
for every true short event, noise/false trigger, ordinary non-short control,
overlap, and handoff. They explicitly add missed events with no proposal.
Consequently, suppressing a production candidate leaves the ground-truth row in
the population and lowers measured recall.

Private audio and artifacts stay under `evaluation/short_turn_dataset/local`,
which is gitignored. Unknown confidence is absent/`null`, never synthesized.

## Evidence and metric definitions

Each benchmark row is a `ground_truth_event`, is explicitly `recall_eligible`,
has a stable sample id and meeting id, and associates the label with transcript
timing/text/ASR confidence where applicable, the meeting's complete raw
diarizer turns, short-candidate VAD events, accepted/visible speakers, timing,
kind, and overlap/handoff tags.

- Candidate recall: fraction of true short speech/backchannel labels matched by
  transcript, diarizer-turn, VAD, or union proposals using configured IoU or
  ground-truth coverage plus center tolerance.
- Kind metrics: expected versus replayed classification, including noise
  controls.
- Speaker attribution: correct expected speaker among labeled speaker events.
- Speaker acceptance: transcript and visible-speaker false-new-speaker rates.
- Materialization: visible precision/recall, embedded false-event rate, and
  embedded speaker accuracy.
- Segmentation/overlap: failures on overlap or handoff-tagged examples.

The CLI rejects invalid timing, missing/duplicate ids, missing meeting ids,
candidate proposals presented as labels, non-recall-capable rows, and declared
duration buckets that disagree with start/end. New manifests use
`short_speech`; legacy `speech` is accepted only as an input alias.
Raw diarizer and VAD evidence must also be identical across rows from the same
meeting, preventing a clip-local subset from being presented as meeting evidence.

## Phase 2D gate and error taxonomy

The minimum remains 100 annotated event/control rows, three meetings, two
speaker/meeting scenarios, all four 100–1200 ms buckets, short speech,
backchannel, noise, overlap, and speaker handoff. Below it the decision is
`INSUFFICIENT_DATA_FOR_PHASE_2D`.

Reports separately count candidate/extraction, event-kind, speaker-attribution,
false-new-speaker/acceptance, embedded/materialization, and overlap/segmentation
failures, with up to five representative sample ids each.
`NO_NEW_MODEL_SIGNAL_FROM_CURRENT_ERRORS` is possible only when every safety
category is clear and metric thresholds pass.

## Execution modes and limitations

Evidence Mode is reported as `evidence_replay`. Production Artifact Replay
additionally requires evidence captured by a real application run. Both reuse
production rules but do not execute ASR or diarization, so neither is called
end-to-end. Pipeline Mode remains explicitly unsupported: the desktop model
lifecycle is coupled to application state, and duplicating it in a CLI would
create a second inference architecture.

Remaining limitations: reconciliation is temporal and meeting-local, not an
acoustic identity match; a physical event moving more than the conservative
threshold may retain an unmatched manual record for review. No representative
private real-audio manifest is committed, so this repository does not itself
satisfy the Phase 2D data floor.
