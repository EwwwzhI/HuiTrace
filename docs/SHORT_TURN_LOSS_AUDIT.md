# Phase 2A short-turn loss audit

This audit covers the evidence path, not only diarization. Phase 2A changes are
offline-only unless a row explicitly says otherwise.

| Pipeline | Loss or distortion point | Phase 2A treatment |
| --- | --- | --- |
| Live capture VAD | `audio/vad.rs` keeps the live `min_speech_time` at 250 ms, so shorter feedback may never reach ASR. | Documented limitation. Live behaviour is unchanged; realtime belongs to Phase 3. |
| Live pipeline | `audio/pipeline.rs` drops VAD outputs shorter than 800 samples (about 50 ms at 16 kHz). | Documented limitation; below the 100 ms Phase 2A candidate floor. |
| Live transcription | `audio/transcription/worker.rs` accepts non-empty Whisper/provider text only at confidence >= 0.3 and treats `AudioTooShort` as an expected skip. | Documented limitation. No live-ASR policy change in Phase 2A. |
| Text post-processing | `audio/post_processor.rs` removed `uh/um/ah/oh/hm/hmm` as artifacts. | Filler deletion removed; lexical evidence is preserved. |
| Import VAD | A shared 250 ms VAD minimum discarded the 100-249 ms part of the target range. A 2,000 ms redemption can also merge a short acknowledgement into a neighbouring turn. | Import now uses the centralized 100 ms offline candidate floor. The 2,000 ms bridge remains a known segmentation limitation. |
| Import transcription | Segments below 1,600 samples (100 ms at 16 kHz) were skipped and empty ASR output was not persisted. | The skip now derives from `ShortTurnConfig.min_candidate_ms`; empty output remains available only as diarizer timeline evidence, not a lexical transcript. |
| Retranscription VAD | Same shared 250 ms minimum and 2,000 ms redemption as import. | Retranscription now uses the centralized 100 ms offline floor; the bridge remains documented. |
| Retranscription ASR | Same sub-100 ms and empty-text skips as import. | Threshold centralized; empty ASR output remains a limitation. |
| Diarization reconciliation | A dominant overlapping diarizer turn was assigned without duration-aware confidence, and a short-only cluster could appear as a new transcript speaker. | Post-diarization `ShortTurnRefiner` applies duration weighting, known-speaker gating, contextual evidence, and never creates a speaker. |
| Manual assignment | Automatic reruns could be dangerous if refinement ignored provenance. | Existing SQL `assignment_method != 'manual'` guard remains; refinement also explicitly bypasses manual assignments. |
| Transcript rendering | `VirtualizedTranscriptView` and legacy `TranscriptView` silently deleted English fillers/backchannels. | Primary transcript rendering now preserves raw ASR text verbatim; regression tests cover `uh`, `hmm`, and `哦`. |

The main remaining Phase 2A recall ceiling is upstream evidence absence: a VAD
miss, empty ASR result, or a 2-second offline VAD bridge cannot be reconstructed
perfectly by a post-diarization rule layer. The evaluation harness makes those
gaps measurable before deciding whether Phase 2B needs a model.
