# Local Short-Turn Annotation Workspace

## Purpose and privacy boundary

Phase 2D.2 is a local, single-user Ground Truth workbench for real meetings.
It is not an inference path, annotation service, or general-purpose labelling
product. Source media, drafts, artifacts, manifests and replay reports stay in
the gitignored `evaluation/short_turn_dataset/local/` tree. No media is sent to
a network service; absolute media paths and local speaker descriptions never
enter `manifest.jsonl`.

## Technology decision

- **Direct dependency:** WaveSurfer.js (MIT), using the Regions, Timeline and
  Minimap plugins for media-synchronised waveform rendering, seek, zoom and
  temporal region editing. A WaveSurfer Region is strictly an ephemeral visual
  projection of a canonical event ID.
- **Conceptual references only:** VGG VIA's compact temporal workflow, Audino's
  speaker-label interaction, Label Studio's annotation/prediction separation,
  and ELAN's separate time-aligned tiers. Their code and server architecture are
  not imported.

## Data model

`AnnotationEvent` is persisted in `annotations.draft.json`, separate from ASR
transcripts, production `ShortTurnEvent`s and overlapping `AnnotationWindow`s.
It contains stable `event_id`, absolute source `start_ms`/`end_ms`, fixed kind,
meeting-local speaker key, overlap/handoff/embedded/uncertain flags, expected
materialization, notes, and annotation status. Editing an event never changes
its ID. `annotation_session.json` has local source-media path/duration, artifact
path, speaker-map descriptions and per-window status. `annotation_project.json`
is a recoverable local index of those files and blind/review completion state.

## Modes and tiers

Blind loading reads the blind window file and returns no review evidence. It
does not load ASR, diarizer, VAD or system suggestion payloads. Review is
unlocked only after blind windows complete; its backend-only payload contains
artifact-backed transcript, diarizer and VAD tiers plus review-window system
suggestions. All are rendered as `SYSTEM SUGGESTION — NOT GROUND TRUTH` and
can only create a new editable draft event after an explicit user action.

The UI keeps waveform, Ground Truth, speaker helper, prediction, ASR, diarizer
and VAD conceptually separate. In blind mode only waveform, Ground Truth and
manual speaker helper are available.

## Annotation guide and shortcuts

Use **backchannel** for conversational acknowledgements, **short speech** for
short independent propositions, **noise** for environmental sound,
**non-speech vocalization** for human non-linguistic sound, and **ordinary
speech control** for longer negative/control speech. Mark uncertainty as a
separate flag rather than inventing a kind. Overlap means concurrent real
speech; handoff means a clear speaker transition; embedded means an event
inside a longer speech interval.

Space toggles play; arrows step 100 ms (Shift: 500 ms); J/L step one second;
B/S/N/V/C select kind; 1–9 select a speaker; O/H/E/U toggle flags; Enter marks
the window complete; Delete removes an event; Ctrl/Cmd+Z and Ctrl/Cmd+Shift+Z
undo/redo. Shortcuts are ignored when a text field has focus.

## Localization

The workspace uses HuiTrace's existing `UiLanguageProvider`, `useUiTranslation`
hook, and shared `en` / `zh-CN` catalogs. It follows the global UI Language
setting immediately, without reloading the route or reopening an annotation
project. A compact Chinese / English control in the workspace header updates
that same global preference; it does not create annotation-specific language
state.

## Normal meeting entry point

Meeting Details exposes **Evaluation Tools** beside the transcript controls when
the existing short-turn annotation feature gate is enabled. Development builds
enable the gate automatically; an evaluation build must set
`NEXT_PUBLIC_ENABLE_SHORT_TURN_ANNOTATION=true`. **Export Production Artifact**
is enabled only after speaker analysis reaches `done`, including a valid result
with zero speaker turns. The action calls the existing persisted-snapshot export
command; it does not rerun ASR, diarization, VAD, or ShortTurn inference.

Use this end-to-end workflow for a real meeting:

1. Record or import a real meeting in HuiTrace.
2. Complete transcription.
3. Run **Identify Speakers** and wait for it to complete.
4. Open **Evaluation Tools** in Meeting Details and choose **Export Production Artifact**.
5. Run `short_turn_export` with the meeting media and exported artifact to create Blind and Review windows.
6. Choose **Open Annotation Workspace** and initialize the generated project.
7. Complete Blind, then Review, run QA, save, and export the benchmark manifest.
8. Run `short_turn_dataset_check` against the local dataset root.
9. Run `short_turn_benchmark --mode production-artifact-replay` for frozen replay.

Cancelling the native export dialog makes no change. A missing completed snapshot
means the meeting must finish normal production speaker analysis before retrying.

Localization applies only to interface chrome, labels, help, status messages,
and controls. Meeting IDs, event IDs, speaker keys and descriptions, ASR text,
paths, filenames, JSON values, enum values, and Ground Truth data are never
translated or rewritten.

## Theme support

The workspace uses HuiTrace's existing `next-themes` provider and semantic CSS
tokens for its page, cards, borders, text, inputs, and controls. It supports
Light and Dark directly. When the global preference is System, the workspace
renders according to `resolvedTheme`; choosing Light or Dark from the compact
header control updates the same global theme preference.

WaveSurfer updates only its visual wave, progress, cursor, and projected region
colors when the resolved theme changes. Canonical annotation events remain
parent-owned, and the controlled projection guard prevents visual region
updates from creating events. The multi-tier timeline uses paired Light/Dark
data colors for Ground Truth, system suggestions, ASR, diarizer, VAD, pending,
and playhead states. Language and theme changes do not alter Ground Truth,
schema values, speaker identities, time boundaries, window status, autosave,
manifests, or benchmark behavior.

## Save, QA and export

Edits debounce to local atomic temporary-file writes. Reopening restores draft,
speaker map and window status. QA validates timings, kinds, duplicate IDs,
speaker requirements/map membership, artifact schema v2/identity and media
duration consistency. Duplicate candidates reuse `evaluation::dataset`; equal
overlap events from different speakers remain legal.

Manifest export derives benchmark-only fields, relative artifact path and SHA-256,
duration bucket, expected visible speakers, and tags from canonical events. It
then invokes the existing dataset checker. Frozen replay remains the existing
`short_turn_benchmark --mode production-artifact-replay` command; the workspace
does not rerun ASR, VAD, diarization or short-turn inference.

## Known limits

The desktop UI accepts local source paths rather than copying media. Browser
container support is platform-dependent; WAV and MP4 are the supported baseline.
The first UI exposes existing frozen replay through the documented CLI, rather
than bundling a second benchmark binary launcher.
# Phase 2D.2a integrity model

## Project initialization and media boundary

An exported meeting is not an annotation project until **Initialize Project** succeeds. The backend validates the dataset/meeting identifiers, both Blind and Review window files, their `production_artifact_path`, the schema-v2 Production Artifact, and the source-media type. It then creates `annotation_project.json`, `annotation_session.json`, and `annotations.draft.json` as one backend-owned workflow and opens Blind mode. Source WAV/MP3/M4A/MP4/WebM media is copied beneath the application audio directory (`mityu-recordings/huitrace-annotation/<meeting_id>`), which is already inside the narrow Tauri asset-protocol scope; arbitrary filesystem access is not enabled.

The project binds the artifact ID, SHA-256, schema version, transcription run ID, and dataset-relative path. Load, Review, save, QA, and export revalidate that identity. Replacing or modifying the artifact is a hard failure.

## Exact pass state machine

Blind is complete only when every ID in `annotation_windows.blind.jsonl` is `reviewed_blind` or `reviewed_second_pass`. Review is complete only when every ID in `annotation_windows.review.jsonl` is `reviewed_second_pass`. Partial Review can therefore be reopened. The current annotation pass is separate from the QA/dataset panel, so running QA cannot turn a Blind completion into a second-pass completion. Last Blind and Review window IDs are persisted and restored; navigation seeks source media to the window start and completion advances to the next unfinished window.

## Ground-truth semantics

`expected_materialized` is tri-state: Auto (`null`/`None`) applies the benchmark default, while Yes and No are explicit overrides. New annotations always start at Auto and in a pending state; QA blocks export until each is explicitly confirmed. Meeting-local speaker keys are generated as immutable `gt_speaker_XX` identifiers, while descriptions remain editable and local-only. QA rejects empty/duplicate keys, missing event membership, and `ordinary_speech_control` intervals at or below 1200 ms.

The benchmark aligns meeting-local GT identities to production diarizer clusters using temporal overlap from independent ordinary-speech reference intervals and a maximum-weight one-to-one assignment before speaker metrics are computed. Production cluster labels are never exposed during Blind annotation.

## Persistence, visualization, and visibility

Autosave uses monotonically increasing edit/saving/saved revisions and a serialized single-flight queue. An older save completing cannot mark a newer edit Saved. Backend atomic writes use per-process unique temporary names. Any malformed root-manifest line aborts export instead of being silently dropped.

Blind exposes only media, waveform, manual GT, local speaker helpers, progress, and structural QA. Dataset quotas, representative-data gates, suggestions, ASR, diarizer, and VAD evidence remain hidden. Review renders viewport-scaled ASR text, diarizer speaker/overlap, VAD confidence, system suggestions, and GT as visually separate tiers. Export is available only after all Blind and Review windows are complete, current-revision QA passes and the same revision has been saved.

## Operational smoke workflow

For one 10–15 minute, 2–4 speaker meeting: run production processing; export Production Artifact v2; run `short_turn_export`; initialize the project without editing JSON; annotate and close/reopen midway through Blind; finish all Blind windows; repeat the close/reopen check midway through Review; run QA; export `manifest.jsonl`; run `short_turn_dataset_check`; then run `short_turn_benchmark --mode production-artifact-replay`. With only one meeting, `INSUFFICIENT_REPRESENTATIVE_DATA` is the expected gate result. The workspace never invokes ASR, diarization, VAD, or ShortTurn inference.


## Phase 2D.2a-final: Blind Completion Integrity

A completed Blind viewport must contain no pending annotation. Membership uses
strict source-time intersection: event.start_ms < window.source_end_ms and
event.end_ms > window.source_start_ms. Boundary contact alone is not overlap;
a pending event crossing two overlapping windows blocks both. Confirmation is
always an explicit annotator action; completing a window never confirms events.
The UI reports the pending count in Chinese/English. The backend validates every
completed viewport before autosave writes any draft/session file. Review completion
also rejects review_pending annotations. Editing or undoing into a pending state
reopens affected viewports; reopening Review retains the previous Blind completion.

Opening Review requires every expected Blind window ID to be complete AND zero
pending events anywhere in the draft, including historical sessions with incorrect
completed statuses. The same evidence boundary applies to a direct QA-mode load,
which returns Review evidence. Structural QA still rejects pending/review_pending.

## Phase 2D.2a-final: Review Export Integrity

Formal export requires Blind complete + Review complete + QA pass + Saved.
Expected IDs come from both window files, never from the number of session keys.
Blind accepts reviewed_blind or reviewed_second_pass; Review accepts only
reviewed_second_pass. Empty window sets and missing statuses fail closed.
The UI displays Review completed/total and disables export until every Review
window is complete, current-revision QA passes, and the same revision is saved.

The backend independently checks both passes before QA or manifest writes, and
requires the supplied draft/session to equal persisted state. Errors include
completed, total, and remaining counts. It parses the existing root manifest,
constructs the merged rows and runs the shared dataset check before replacing
either manifest. Incomplete passes, pending annotations, unsaved changes, malformed
existing JSON and dataset/artifact validation failures leave both manifests unchanged.
If root replacement fails after the meeting copy was written, the meeting copy is
restored. This is rollback for reported write failures, not a crash-atomic transaction
across two files; sudden termination or failure of rollback storage remains a limit.

## Phase 2D.2a-final: Speaker Alignment Integrity

Only independent ordinary-speech reference intervals are used for GT ↔ production
cluster alignment. A reference must retain the explicit ordinary_speech_control
label, have duration > 1200 ms, a nonempty known speaker, annotation_uncertain=false,
and no overlap tag. Prefer clear intervals of at least 2 seconds during collection;
the implemented eligibility rule is the fixed >1200 ms threshold. Handoff and
embedded flags alone do not exclude a reference.

The shared GroundTruthKind enum preserves the original annotation label rather
than reducing both short_speech and ordinary_speech_control to SegmentKind::Speech.
Production comparisons still use SegmentKind via explicit conversion. Legacy
speech remains readable and keeps its prior coverage interpretation, but never
becomes a reference based on duration. short_speech, backchannel, noise and
non_speech_vocalization never participate in the alignment matrix.

Per meeting, reference-only temporal overlaps are accumulated and a maximum-weight
one-to-one assignment is frozen before ShortTurn scoring. There is no fallback to
short events. Speakers with no valid positive-overlap assignment are unaligned and
excluded from individual attribution denominators; ambiguous speaker sets are scored
only if all their GT speakers are aligned. Dataset coverage is computed on original
GT identities before alignment. JSON output includes speaker_alignment by meeting:
mapped_speakers, total_gt_speakers, unmapped_gt_speakers, reference_intervals,
reference_duration_ms and the frozen mapping. Production cluster identities remain
confined to Review evidence/evaluation and are never added to Blind UI.

Regression coverage includes overlapping-window pending gates, corrupted historical
Review entry, 199/200 and missing-status export failures with unchanged manifests,
full export, unsaved changes, swapped clusters, adversarial short-turn evidence,
uncertain/overlap/legacy exclusions, accumulated references and unmapped denominators.
The synthetic integration test exercises short_turn_export → Initialize → Blind
Confirm/100% → Review/100% → QA → Saved → Export → Dataset Check → Frozen Replay,
using generated silence and explicitly synthetic labels, not representative data.
