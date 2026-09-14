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
