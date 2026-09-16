# Safe Whisper lexical timing audit (2026-09-16)

## Findings before implementation

- Cargo.lock resolves whisper-rs 0.13.2 / whisper-rs-sys 0.11.1. All platform
  declarations enable raw-api, but extraction needs no unsafe/raw calls.
- WhisperState exposes segment count/text/t0/t1, token count/data/text/probability.
  WhisperContext exposes token_eot and token_to_cstr. Token data includes id,
  t0/t1 and p. Ordinary lexical IDs are below the model-specific EOT ID.
- Bundled whisper.cpp `whisper_full_with_state` invokes
  `whisper_exp_compute_token_level_timestamps` in the final nonempty-text branch
  when token_timestamps is true, independently of no_timestamps. The latter
  suppresses timestamp decoder tokens; it does not disable timing estimation.
- `timestamp_to_sample(t)` uses `t * WHISPER_SAMPLE_RATE / 100`, confirming
  centiseconds. Conversion is checked multiplication by 10, not an offset or
  interpolation. All times remain relative to the input chunk.
- The confidence Engine method returned a text/confidence/partial tuple. The
  separate prompt path read segment timing for logging only. Provider, worker,
  import and retranscription all discarded timing. Default capability incorrectly
  equated disabled decoder timestamps with absent lexical timing.
- TranscriptTiming/TimedToken already support NativeToken intervals. V3 validates,
  groups BPE, aligns with speaker turns and checks reconstructed lexical content.
  It did not require NativeToken ends or reject extreme intervals. Frozen V1
  ignores asr_timing_json through its independent frozen normalizer.

## Implementation and safety

Engine returns WhisperTranscriptionResult with independent token metadata; the
old tuple method remains as a compatibility wrapper. Segment text assembly,
confidence calculation, repetition cleanup, decoding settings and partial policy
are unchanged. Both production paths retain no_timestamps=true and
token_timestamps=true. Token extraction runs only after authoritative text exists;
API, UTF-8 and conversion failures yield no tokens, never an inference error.

Special tokens are filtered by vocabulary ID, not token spelling. UTF-8 is strict:
a BPE fragment that is not independently valid UTF-8 rejects the whole metadata
batch. No replacement characters, guessed timestamps or token-based text rewrite.
Segment whitespace in metadata follows the existing trimmed, space-joined text
path. Empty strings are excluded; meaningful separators and punctuation survive.

One provider mapper is used by the trait, direct worker, import and retranscription.
It leaves text/confidence/partial unchanged and advertises token timestamps and
confidence, not word or segment timestamps. Every token is NativeToken, never
NativeTokenEmission. Gate failure returns timing=None with the same text.

Provider and reconstruction share the existing whitespace-preserving lexical
normalization and interval validator. Punctuation, case, Unicode and word boundaries
must match; only whitespace layout is normalized. Starts/ends must be monotonic,
starts nonnegative, ends present and >= starts for NativeToken. NativeToken spans
over 10 seconds are rejected. Provider bounds allow 10 ms quantization, with zero
ordering jitter tolerance. No clipping or interpolation. V3 revalidates persisted
metadata (its existing configurable 200 ms bounds tolerance remains unchanged),
then groups, aligns and falls back if grouping/atomic spans lose lexical content.
Boundary scoring, thresholds and diarization are untouched.

Frozen-V1 regression compares the complete serialized result before/after adding
Whisper timing. Candidate tests verify native timing and invalid-interval fallback.

## Evidence and limitations

The fixed JFK audio test uses pinned multilingual small and an independent copy
of the pre-change Engine method. It checks full text equality, confidence/partial
equality, both clauses and the tail, and requires accepted native metadata. See
the fixture README for provenance, hashes and execution instructions.

The original e978c55 failing audio is unavailable; this test does not demonstrate
that JFK reproduces its unsafe single-timestamp branch. Always-on source-policy
and special-token-ID tests protect that constraint, alongside audio preservation.

Token timing is whisper.cpp's experimental heuristic, not measured alignment
accuracy. Valid intervals do not establish accurate speaker boundaries. Split-byte
Unicode, Chinese tokenizer variants and other multilingual models may conservatively
fall back. Synthetic tests cover Chinese/English mixing, punctuation, leading BPE
spaces, partial flags and zero/short bounds; only English JFK has a real-audio gate.
Actual short-audio, multilingual and partial streaming accuracy remains unmeasured.
Repetition cleanup can invalidate timing; that intentionally preserves baseline text.
Windows CPU inference was exercised; macOS capture and Windows live recording
smoke tests still require the corresponding interactive environments before merge.

## Validation results

- `cargo fmt --all -- --check`: passed.
- `cargo clippy -p huitrace --all-targets`: passed, with existing repository
  warnings; no warning locations in the new timing/provider/test implementation.
- `cargo test -p huitrace`: 773 passed, 0 failed, 5 ignored across unit, binary,
  integration and doc tests (library: 639 passed, 4 ignored).
- `cargo test -p huitrace --lib e978c55_audio -- --ignored --nocapture` with the
  pinned local small model: 1 passed; actual native timing accepted, baseline and
  tail checks passed. This explicitly executes one of the default ignored tests.
- `git diff --check`: passed.

These commands cover the changed app package, not unrelated workspace helpers.
