# Issue #38 investigation: QNN HTP output diverges from CPU golden

Issue: [#38 `bug: QNN HTP output diverges from CPU golden for qdq_conv.onnx (blocks AC-04)`](https://github.com/takurot/npu-router/issues/38)
Parent: #5 (P0-04), evidence gathered in `spikes/p0-04-qnn-only-session/`

## Status: root cause narrowed, not yet fixed or fully attributed

This is a **progress report**, not a resolution. Issue #38 stays open.

## What this adds beyond the original finding

`spikes/p0-04-qnn-only-session/` established (on real Windows ARM64 /
Snapdragon X Plus / Hexagon HTP v73 hardware) that `qdq_conv.onnx`'s QNN
output (`[0,0,0,0,0,0,0,0,0]`) does not match the CPU golden
(`[255,255,255,255,255,255,255,255,255]`), and ruled out a harness bug (the
identical code with QNN not appended reproduces the golden exactly).

This investigation answers the next Scope question from Issue #38: **is the
CPU golden (255) even the value the model's own quantization parameters
say is correct, or could QNN's answer (0) also be a legitimate
interpretation?** `analyze_qdq_conv.py` decodes the pinned
`qdq-conv.onnx`'s ONNX protobuf by hand (verifying its SHA-256 against
`fixtures/catalog.json` before trusting it) and computes the QDQ
Conv/DequantizeLinear/QuantizeLinear math directly from the model's own
declared `Scale_0`/`Zero_point_uint8_0`/`Zero_point_int32_0`/`W_0`/`Bias_0`
values and the pinned golden input.

### Run it

```bash
python3 spikes/issue-38-qnn-qdq-zero-point/analyze_qdq_conv.py
```

(No dependencies -- pure standard library, matching the sibling Rust
spikes' "no external crates" convention for the same auditability reason.)

### Result

```text
Decoded QDQ parameters (shared by every DequantizeLinear/QuantizeLinear node --
this fixture reuses one Scale_0/Zero_point_uint8_0 pair for input, weight, and
output, per the graph's own node list):
  Scale_0            = 256.0
  Zero_point_uint8_0 = 0
  Zero_point_int32_0 = 0
  W_0 (3x3, uint8)   = [128, 128, 128, 128, 128, 128, 128, 128, 128]
  Bias_0 (int32)     = 64

Per-output-position float conv accumulator (dequantized domain), then the
QuantizeLinear result (round-to-nearest, clamp to [0,255]):
  row 0: [255, 255, 255]
  row 1: [255, 255, 255]
  row 2: [255, 255, 255]

=> Every output position saturates to the maximum representable uint8 (255)
regardless of position, because Scale_0=256 and the weight/bias magnitudes
here make the pre-clamp accumulator many orders of magnitude larger than 255
for every 3x3 window. This is the CPU golden's answer -- it is the ONLY
value consistent with this model's own quantization parameters.
```

**Conclusion: the CPU golden (255) is unambiguously, arithmetically the
correct answer.** For every one of the 9 output positions, the minimum
possible pre-clamp accumulator (using the smallest 3x3 input window, sum of
pixel values = 54) is `54 * 256(weight scale contribution) * 256(Scale_0)
+ 64(bias) ≈ 1.77M`, roughly 6900x larger than the `255` saturation
ceiling -- there is no window, no rounding mode, and no plausible
alternate zero-point convention under which this model's own declared
parameters produce `0`. QNN HTP's `0` is not an alternate-but-valid
quantization interpretation; it is inconsistent with the graph it was
asked to run.

## Working hypothesis for *why* (not yet confirmed)

Two properties of this specific fixture are unusual for a "realistic"
quantized model and are the leading suspects:

1. **`Zero_point_uint8_0 = 0`.** This is the extreme edge of the valid
   `uint8` zero-point range `[0, 255]`. Most real quantized models use an
   interior zero-point (often near 128 for roughly-symmetric activation
   ranges); `0` degenerates a `uint8` tensor into "plain unsigned integer,
   no offset" and is the kind of boundary value most likely to expose an
   off-by-one or signed/unsigned edge case in a backend's zero-point
   handling.
2. **A very large output requantization multiplier.** The effective ratio
   `(input_scale * weight_scale) / output_scale = (256 * 256) / 256 = 256`
   is unusually large -- realistic quantized conv layers typically have
   this ratio well below `1.0` (activations and weights are usually
   similar-magnitude, so their scale product is normally much smaller than
   a single scale). Hardware requantization pipelines commonly represent
   this ratio as a fixed-point multiplier `M0` (`Q0.31`-style, i.e.
   designed for multipliers `< 1.0`) plus a right-shift; a `256x` ratio is
   far outside that design envelope and is a plausible trigger for a
   saturation-direction or shift-amount bug specific to out-of-range
   multipliers.

Both point the same direction: `qdq_conv.onnx` (an ORT-internal graph
*transformation* test fixture -- see `docs/TEST_FIXTURES.md`'s own
rationale for picking it, which is about op coverage for QNN's supported-op
table, not numeric realism) likely has more extreme quantization parameters
than QNN HTP's requantization path was validated against, rather than a
general QNN correctness defect that would show up on realistically-scaled
models too. This is a hypothesis, not a proven root cause -- no QNN-internal
source or documentation was available to confirm the fixed-point pipeline's
actual behavior.

## Recommended next step (not done here)

Test with a second, non-degenerate static-shape uint8 QDQ Conv fixture --
interior zero-points (e.g. 128) on input/weight/output, and a
requantization ratio `< 1.0` (realistic scale magnitudes) -- on the same
hardware. If it matches the CPU golden within 1 LSB, that confirms the
`qdq_conv.onnx` fixture's extreme parameters (not a general QNN defect) are
the cause, and Issue #38's resolution becomes "document this as a known
limitation with extreme quantization parameters in ADR-001, not a blocker
for realistically-quantized MVP models." If it still diverges, that
escalates this to a broader QNN EP correctness concern requiring an
upstream report to `microsoft/onnxruntime` and/or Qualcomm.

Creating and pinning such a fixture (per `docs/TEST_FIXTURES.md`'s
provenance/hash process) is out of this investigation's scope and is left
as the next actionable step on Issue #38.

## Non-goals

- Not a fix -- no QNN/ORT source change is proposed or possible from this
  repo (`onnxruntime_providers_qnn.dll`/`Qnn*.dll` are vendored binaries).
- Not a general ONNX protobuf decoder -- `analyze_qdq_conv.py` understands
  only the message subset this one pinned fixture uses (same narrow-parser
  convention as the sibling Rust spikes' `protobuf.rs`).
- Not a QNN provider-option sweep -- `spikes/p0-04-qnn-only-session/`
  already tried `offload_graph_io_quantization=0` with no change; this
  investigation is about the model's own math, not further option-guessing.
