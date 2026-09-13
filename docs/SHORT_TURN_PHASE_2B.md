# Phase 2B — Short-turn evidence hardening

Phase 2B keeps the Phase 2A model-free boundary. It changes evidence plumbing,
candidate recovery, acceptance policy, and measurement; it does not add a new
embedding, AED, denoising, or realtime model.

## Production data flow

- `transcripts.asr_confidence` is independent from `speaker_confidence`.
  Whisper/provider scores are preserved; Parakeet and other providers without a
  score write `NULL`.
- Short-candidate overlap reuses the canonical timeline implementation
  (`has_true_speaker_overlap`). A transcript spanning a sequential handoff is
  not simultaneous speech.
- Raw backend clusters remain in `speaker_turns`. User-facing turns are an inner
  join with accepted `speakers`, so weak short-only clusters cannot affect
  speaker count, talk time, picker choices, rename, or final transcript speaker.
- Speaker acceptance aggregates total duration, turn count, longest turn,
  available confidence, and overlap ratio. Missing confidence is not `1.0`; it
  requires at least two turns and four seconds of corroborated speech.

## Candidate recovery

`ShortTurnCandidateExtractor` consumes three independent sources:

1. short transcript rows;
2. 100–1200 ms diarizer turns, including turns inside a long transcript row;
3. an independent 100 ms minimum / 250 ms redemption short-candidate VAD pass.

The short VAD pass never changes main ASR VAD segmentation. Candidates merge
deterministically using time IoU, containment, and center distance. The merged
candidate retains every contributing source and never fabricates missing
confidence.

Speaker attribution follows direct diarizer evidence, accepted-speaker matching,
and consistent temporal context. Lexical evidence primarily classifies the
event as a backchannel and cannot create a speaker identity. Decisions expose
separate `kind_confidence` and `speaker_confidence`.

Manual assignments bypass every refinement pass. Automatic revision increments
only when speaker, segment kind, or speaker confidence actually changes.

## Evaluation

Metadata fixtures remain deterministic regression tests. The local real-audio
harness lives under `evaluation/short_turn_dataset` and reports overall plus
100–300, 300–500, 500–800, 800–1200, and 1200–1500 ms control buckets. It also
reports candidate recall by Transcript, DiarizerTurn, VadEvent, and union, plus
separate transcript-assignment and visible-speaker false-new-speaker rates.

Audio is intentionally excluded from source control. Model-assisted Phase 2D
should not begin until representative real-audio results show a remaining error
that evidence plumbing and threshold tuning cannot address.
