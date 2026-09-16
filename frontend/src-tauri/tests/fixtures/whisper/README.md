# Whisper text-preservation audio gate

`jfk.wav` is the public JFK inaugural-address sample distributed by whisper.cpp
v1.7.3, downloaded from:
https://raw.githubusercontent.com/ggml-org/whisper.cpp/v1.7.3/samples/jfk.wav

SHA-256: `59dfb9a4acb36fe2a2affc14bacbee2920ff435cb13cc314a08c13f66ba7860e`.
The US federal government speech recording is public domain. PCM16, mono, 16 kHz.

Use the existing model catalog's pinned multilingual `ggml-small.bin`:
SHA-256 `1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b`.
The test discovers and verifies the model through the app's usual model loader.
It never downloads a model. From the workspace root, in PowerShell:

```powershell
$env:WHISPER_TEST_MODELS_DIR = '<directory containing ggml-small.bin>'
cargo test -p huitrace --lib e978c55_audio -- --ignored --nocapture
```

The test compares text, confidence and partial status with an independent frozen
copy of the pre-change confidence path from `ded4f32a58117b1f440eb6b8b0e41f0a65b9e568`.
It additionally checks both speech clauses and the final words, and requires
validated NativeToken metadata. Without a model the test is explicitly ignored;
unit fallback, decoder-policy and frozen-V1 gates always run.

This is a fixed audio preservation regression, **not** the original failing audio
from e978c55 (that commit contains no reproducer). A separate always-on policy
gate locks timestamp-token decoding off in both production parameter paths;
token-ID tests ensure a single trailing timestamp cannot become lexical metadata.
Do not claim the historical unsafe decoder branch is reproduced by this audio.
