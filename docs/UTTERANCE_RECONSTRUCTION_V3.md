# Utterance Reconstruction V3: semantic-aware deterministic baseline

## Current-state audit

The supported local recording path is:

```text
audio/stream.rs + system_audio_stream.rs
  -> audio/pipeline.rs (48 kHz mix)
  -> audio/vad.rs::ContinuousVadProcessor
  -> audio/transcription/worker.rs
  -> WhisperProvider or ParakeetProvider
  -> TranscriptUpdate (including optional provider-native timing)
  -> audio/recording_commands.rs transcript-update listener
  -> recording_saver::TranscriptSegment
  -> database/repositories/transcript.rs (raw row + asr_timing_json)
```

Speaker separation remains a post-hoc, failure-isolated pass. Accepted
`SpeakerTurn` rows and optional `ShortTurnEvent` rows are read alongside raw
transcripts by `api_get_reconstructed_utterances`; reconstruction does not
rewrite the transcript table.

The derived read path is:

```text
api_get_reconstructed_utterances
  -> normalize_timeline (raw-row fallback attribution)
  -> timing::enhance_timeline (validated provider timing only)
  -> alignment::align_words (lexical timing x accepted speaker turns)
  -> AtomicSpan
  -> semantic::evaluate + boundary::decide_boundary
  -> assembler::reconstruct_with_details
  -> ReconstructionResult
```

The meeting-details UI genuinely consumes this result. `page.tsx` loads raw
rows with `usePaginatedTranscripts` for evidence, pagination, summaries, and
fallback. `page-content.tsx` passes them to `MeetingDetails/TranscriptPanel`.
That component invokes `api_get_reconstructed_utterances`, maps utterances to
display segments, and passes those segments to `VirtualizedTranscriptView` by
default. Users can switch to Raw, and any reconstruction error fails open to
raw rows. The live recording panel continues to show streaming raw ASR because
post-hoc diarization is not yet available during capture.

## Timing policy

- Parakeet exposes native token-emission timestamps. They are validated before
  lexical grouping and can split one ASR chunk across an A-to-B handoff.
- Whisper currently uses `no_timestamps=true` to avoid a known chunk-skipping
  regression. Although the backend parameter set also enables token timestamp
  processing, the resulting timestamps are not considered safe provider data;
  `WhisperProvider` therefore advertises no timing capability and returns
  `timing: None`.
- Missing, malformed, non-monotonic, or out-of-bounds provider timing falls
  back to the exact V1 span. No lexical timestamps are inferred or fabricated.

## Root causes addressed

The former boundary implementation treated four uncertainty conditions as
observed linguistic boundaries: unreliable timing, mixed attribution, overlap,
and every speaker-key change. Each immediately returned a hard split with
`i32::MAX`, which fragmented ordinary speech after timing/provider fallback or
a low-confidence diarization fluctuation. Same-speaker terminal punctuation
also lacked a sentence-completeness feature, so real topic boundaries and ASR
fragment boundaries could not be distinguished beyond gap and punctuation.

V3 keeps hard constraints for long silence, trustworthy maximum duration,
maximum text length, and high-confidence speaker changes. Unreliable timing,
mixed attribution, overlap, and low-confidence speaker conflicts are now
explicit evidence rather than automatic boundaries. Attribution remains
Unknown when incompatible spans merge; uncertainty is never converted into a
false single-speaker claim.

## Semantic baseline and explainability

`semantic.rs` supplies a deterministic baseline with two optional values:

- `left_completeness`: strong terminal punctuation is strong completion
  evidence; weak punctuation and short punctuation-free fragments are weak
  evidence. Text length is only weak evidence.
- `cross_boundary_continuity`: continuation prefixes and weak punctuation are
  strong merge evidence; terminal punctuation is weak continuity evidence.

This is not presented as an NLP model. It uses no transcript rewriting and has
a versioned interface that a future local model can replace. Every boundary
now includes timing reliability, speaker-change confidence/reliability,
semantic values, unavailable prosody state, per-family score components, final
score, decision, and reasons. Debug builds log these values.

## Final pipeline

```text
Audio -> VAD fragments -> ASR raw text (source of truth)
                         + optional validated lexical timing
                         + accepted diarization timeline
                                      |
                                      v
                         fine-grained speaker alignment
                                      |
                                      v
                                  AtomicSpan
                                      |
                  acoustic + attribution + punctuation
                  + semantic baseline + hard constraints
                                      |
                                      v
                    semantic utterance reconstruction
                                      |
                                      v
                derived readable transcript / raw fallback
```

## Benchmark policy

The Ground Truth schema supports explicit mainline buckets for
`same_speaker_continuity`, `same_speaker_boundary`, `asr_fragmentation`, and
`vad_fragmentation`, in addition to speaker handoff and existing diagnostic
buckets. Reports expose Boundary Precision/Recall/F1, False Split Rate, False
Merge Rate, selective Speaker Attribution Accuracy, 500 ms-collar Speaker
Boundary Accuracy, and signed Utterance Count Error.

Unit fixtures cover the mainline behaviors, but they are synthetic plumbing
checks and must not be reported as product-quality gains. A trustworthy
Before/After/Delta requires completed, reviewed, QA-passed real-meeting Ground
Truth. No such private GT corpus is committed to this repository.

## Remaining limitations

- The semantic baseline is shallow and deterministic; it does not parse syntax
  or perform neural semantic inference.
- Whisper lexical timing remains unavailable until the chunk-skipping issue can
  be resolved and validated without text loss.
- Short backchannels, overlapping speech separation, advanced prosody, and
  difficult diarization ambiguity remain intentionally out of scope.
- Merging a semantically continuous low-confidence speaker conflict yields
  Unknown attribution; later evidence may improve that result, but V3 will not
  guess a speaker.
