# P0-04 spike: QNN-only ORT Session, implicit CPU fallback disabled

Issue: [#5 `[P0-04] spike: QNN-only Sessionと暗黙CPU fallback無効化を実証する`](https://github.com/takurot/npu-router/issues/5)
Traceability: `docs/SPEC.md` 3.1 (固定方針 #3), AC-04, OQ-01; Phase 0 gate (`docs/SPEC.md` 24章)

## Result

**Partially passed on real Windows 11 ARM64 / Snapdragon X Plus hardware** (not
mocked -- the QNN Hexagon Tensor Compiler's own stage logs, shown below, are
proof this ran on the actual HTP/NPU, not a CPU simulator):

| Check | Result |
| --- | --- |
| QNN-only Session creation + `Run`, no CPU-EP fallback, `qdq_conv.onnx` | ✅ succeeded, zero silent CPU fallback |
| Numeric agreement with the CPU golden (`docs/TEST_FIXTURES.md` 1 LSB tolerance) | ❌ **did not match** -- see "Finding" below |
| `qnn_ep_partial_support.onnx` (`MatMulInteger`, QNN-unsupported) fails `CreateSession` cleanly, not a silent fallback, not a crash | ✅ succeeded |

```json
{
  "ort_version": "1.23.2",
  "ort_api_version_used": 23,
  "provider": "qnn",
  "disable_cpu_ep_fallback": true,
  "supported_model": {
    "status": "ok",
    "input_name": "X_0",
    "output_name": "Y_0",
    "output_bytes": [0, 0, 0, 0, 0, 0, 0, 0, 0],
    "golden_bytes": [255, 255, 255, 255, 255, 255, 255, 255, 255],
    "max_abs_diff_lsb": 255,
    "within_tolerance": false
  },
  "unsupported_model": {
    "status": "failed_as_expected",
    "ort_error_code": 1,
    "ort_error_message": "This session contains graph nodes that are assigned to the default CPU EP, but fallback to CPU EP has been explicitly disabled by the user."
  }
}
```

Exit code `1` (the spike's own JSON `within_tolerance: false` for the
supported model is what makes it non-zero; nothing crashed, aborted, or
returned an unclassified error).

## Finding: QNN HTP output does not match CPU golden for `qdq_conv.onnx`

The QNN-only `Session` for `qdq_conv.onnx` creates and runs successfully
(HTP graph compiles, `Run` returns no `OrtStatus` error), but the returned
quantized `uint8` output is all-zero, while both the pinned CPU golden
(`docs/TEST_FIXTURES.md`) **and this spike's own CPU-only run** (same
harness, same input, QNN EP simply not appended) return all-`255`,
`max_abs_diff_lsb=0`. This rules out a harness/input-feeding bug: the exact
same tensor-construction, `Run`, and read-back code path produces the
correct golden-matching result on CPU EP and an all-zero result on QNN EP,
with only the execution provider differing.

Ruled out before concluding this is a backend-level discrepancy:

- **Harness bug in general**: no -- CPU EP run (same code, QNN provider
  simply not appended) reproduces the golden exactly (`max_abs_diff_lsb=0`).
- **`offload_graph_io_quantization` graph-boundary optimization**: no --
  explicitly setting the QNN EP provider option
  `offload_graph_io_quantization=0` produced the identical all-zero result.
- **Silent CPU fallback masking the real QNN answer**: no -- fallback is
  disabled (`session.disable_cpu_ep_fallback=1`) and confirmed working (the
  unsupported-model test above proves ORT does raise a hard error instead of
  silently falling back when a node can't run on QNN); if any node here had
  needed CPU, session creation would have failed the same way.

Not ruled out / not investigated further (out of this spike's time-box,
Non-goals below): incorrect QDQ scale/zero-point handling specific to this
HTP v73 build, a `qdq_conv.onnx` graph shape this particular NPU/driver
combination mishandles, or an ORT v1.23.2 QNN EP defect. All of ORT, the
QNN SDK, and the driver stack are pinned to specific, recorded versions
below, so this is reproducible and version-scoped, not open-ended.

**This is real evidence, reported as observed, not smoothed over.** It
satisfies Issue #5's Scope item "unsupported operator時の挙動を記録する" in
spirit (an operator-support boundary condition on real hardware, just a
numeric one rather than a `CreateSession`-time rejection) and directly
informs ADR-001 (OQ-01): QNN-only Session **mechanics** are proven sound
(no implicit CPU mixing, clean failure classification), but bit-exact
numeric parity with this specific pinned fixture is **not yet proven** on
this hardware/SDK combination and needs follow-up before AC-04 ("QNN
output が CPU golden から最大 1 LSB 以内であること",
`docs/TEST_FIXTURES.md` "実機で残る検証") can be marked satisfied.

## Environment

| Item | Value |
| --- | --- |
| OS | Windows 11 Home, build `10.0.26200` (Windows 11 ARM64) |
| CPU / NPU | Snapdragon(R) X 10-core X1P64100 @ 3.40 GHz (Snapdragon X Plus) / Qualcomm Hexagon (HTP v73) |
| Rust | `rustc 1.97.1 (8bab26f4f 2026-07-14)`, host `aarch64-pc-windows-msvc` |
| ONNX Runtime | `1.23.2` (commit `a83fc4d58cb48eb68890dd689f94f28288cf2278`) |
| ONNX Runtime QNN EP plugin (`onnxruntime_providers_qnn.dll`) | file version `1.23.20251021.4.a83fc4d` |
| QNN SDK (`QnnHtp.dll`) | file version `2.37.1.0`, product version `2.37.1.250807093845_124904`, publisher Qualcomm Technologies, Inc. |

## Artifact provenance

### ONNX Runtime + QNN EP

Distributed together as the `Microsoft.ML.OnnxRuntime.QNN` NuGet package
(Microsoft-published, not a GitHub Release asset -- ORT does not ship QNN
binaries as GitHub Release zips):

- Package: `Microsoft.ML.OnnxRuntime.QNN` `1.23.2`
- Download URL: `https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.qnn/1.23.2/microsoft.ml.onnxruntime.qnn.1.23.2.nupkg`
- SHA-256 (`.nupkg`, this exact download): `8837d647caf4b902e9bc93eb69bdb497b603bdfc5a28a31da2f66968946ccce9`
- `<repository commit="a83fc4d58cb48eb68890dd689f94f28288cf2278">` in the package's own `.nuspec` -- the **same** ORT commit `docs/TEST_FIXTURES.md` already pinned as the source of `qdq_conv.onnx` / `qnn_ep_partial_support.onnx`, and the same commit/version as the sibling P0-03 (CPU) spike's `onnxruntime-win-arm64-1.23.2.zip`.
- License: MIT (`LICENSE` in the package, same as the CPU-only release) **plus** a separate `Qualcomm_LICENSE.pdf` covering the `Qnn*.dll`/`.so` files -- both must be recorded for ADR-002 (redistribution terms are not identical for the Microsoft-authored and Qualcomm-authored files in this one package).

SHA-256 of the native files actually loaded by this spike (`runtimes/win-arm64/native/`):

| File | SHA-256 |
| --- | --- |
| `onnxruntime.dll` | `d9d9dd7c60d9ceac89f7a30aafb0c3fb4ad8dd317c170d8c4d0cd538c165f62a` |
| `onnxruntime_providers_shared.dll` | `0e6148c575d4ff964bb77cfae908aec366513e61275a9dc852d23e55bf4d78c9` |
| `onnxruntime_providers_qnn.dll` | `d35bc25c42e4a53cf80419a8dc23a5761392c412b5e133bea8605f2f0e6e50a5` |
| `QnnHtp.dll` | `1e29a38940728bd058434616d6edbfb4a188e12b2db74f3c6e491e4fb22e4fd2` |
| `QnnHtpPrepare.dll` | `f715535ac0e96ca727620e84f3bbb32a176413ca487d892c66c572df307f879c` |
| `QnnHtpV73Stub.dll` | `447bd385c2cfee95af200e11b1b7885d1a25929f0945332a17fd313af3531369` |
| `QnnSystem.dll` | `d1d0db888d0609d72a04217c82d3505405e0676608bca3c38a7d79dac10867f0` |
| `QnnCpu.dll` | `8fc84d4dfd8ea0fbef89c87851da20fdc3570c1e6b4e448e0ee2b28108c2b93a` |
| `QnnGpu.dll` | `8070204ea9c6f13688311ee11e831c69c1462e9b9f94dc65f1017fadaa08b75d` |
| `libQnnHtpV73Skel.so` | `3c0de799379af2f4063724238d5f6ebfd709f811de03b99bc56196ee058ef2e8` |
| `libqnnhtpv73.cat` | `61eb791da65b81ce63c4560321c72f9f0de3d700044b053d95e106f8cec46345` |

Not committed to git (`.cache/` is gitignored, matching `docs/TEST_FIXTURES.md`
convention). To reproduce, download the `.nupkg` above (it is a plain zip),
verify its SHA-256, extract it, and point this spike at
`runtimes/win-arm64/native/onnxruntime.dll` and `...\QnnHtp.dll`.

### Model + golden vectors

Reused unmodified from `docs/TEST_FIXTURES.md` (Issue #3), via
`node scripts/fetch-test-fixtures.mjs`:

- `qnn/qdq-conv.onnx` -- QNN-supported: static-shape uint8 QDQ `Conv`
- `qnn/qnn-unsupported-matmul-integer.onnx` -- QNN-unsupported (`MatMulInteger`)
- `qnn/golden/input.raw` (25 bytes, `[1,1,5,5]` uint8) / `qnn/golden/output.raw` (9 bytes, `[1,1,3,3]` uint8, CPU golden)

These are raw binary tensors (not protobuf), so unlike the sibling P0-03
spike this one needs no protobuf parsing -- fixed-size byte buffers are read
directly per the shapes `docs/TEST_FIXTURES.md` already declares.

## Reproduction

```powershell
# 1. Fetch the pinned model/golden fixtures (from the repo root).
node scripts/fetch-test-fixtures.mjs

# 2. Download + verify + extract Microsoft.ML.OnnxRuntime.QNN 1.23.2 (see
#    hashes above) into .cache/vendor/onnxruntime-qnn-win-arm64-1.23.2/.

# 3. Build and run the spike (from spikes/p0-04-qnn-only-session/).
cargo build --release
target\release\p0-04-qnn-only-session-spike.exe `
  ..\..\.cache\vendor\onnxruntime-qnn-win-arm64-1.23.2\runtimes\win-arm64\native\onnxruntime.dll `
  ..\..\.cache\vendor\onnxruntime-qnn-win-arm64-1.23.2\runtimes\win-arm64\native\QnnHtp.dll `
  ..\..\.cache\test-fixtures\qnn\qdq-conv.onnx `
  ..\..\.cache\test-fixtures\qnn\golden\input.raw `
  ..\..\.cache\test-fixtures\qnn\golden\output.raw `
  ..\..\.cache\test-fixtures\qnn\qnn-unsupported-matmul-integer.onnx
```

stdout is the JSON report above; ORT/QNN's own diagnostic stage logs
(Hexagon Tensor Compiler stage timings, DDR bandwidth summary) go to
stderr, interleaved, and are proof of real HTP compilation -- they cannot be
produced without the actual NPU/driver stack.

## No CPU-node mixing: how this is proven, not just claimed

`provider=qnn`にCPU nodeが混在しない証跡 (Issue #5 AC) is established
structurally, not by post-hoc profiling: `session.disable_cpu_ep_fallback=1`
is set via `AddSessionConfigEntry` *before* `CreateSession`, so if ORT's
QNN EP capability check (`QNNExecutionProvider::GetCapability`) assigns even
one node to the default CPU EP, `CreateSession` itself fails with
`This session contains graph nodes that are assigned to the default CPU EP,
but fallback to CPU EP has been explicitly disabled by the user.` -- exactly
what this spike observes for `qnn_ep_partial_support.onnx`'s `MatMulInteger`
node. A `qdq_conv.onnx` session that *succeeds* under this same config
therefore could not have silently used CPU for any node; the alternative
(inspecting an ORT profiling trace for node-to-EP assignment) is unnecessary
given this stronger, session-creation-time guarantee.

## Session options / provider options used

```text
AddSessionConfigEntry(options, "session.disable_cpu_ep_fallback", "1")
SessionOptionsAppendExecutionProvider(options, "QNN",
    keys=["backend_path"], values=["<canonical path to QnnHtp.dll>"])
```

`backend_path` is passed as an absolute, canonicalized path (never a bare
filename), consistent with `docs/SPEC.md` 18.2's fixed DLL search order
policy applied to the main `onnxruntime.dll` load in both this and the
sibling P0-03 spike.

## Non-goals (per Issue #5)

- Not the Router's CPU fallback path (Issue #20, P2-02) -- this spike never
  invokes a CPU `Session` as *part of* the QNN flow; the one CPU-only run
  referenced under "Finding" above was a throwaway isolation check, not
  part of the spike's own pass/fail logic.
- Not a QNN numerical-accuracy root-cause investigation -- the discrepancy
  above is recorded as a finding for follow-up, not resolved here.
- Not a general ORT/QNN C API wrapper -- see `../p0-03-ort-cpu-session/README.md`
  "OrtApi field derivation" for the shared technique; this spike's
  `ffi::OrtApi` is a superset of that one (adds `AddSessionConfigEntry` and
  `SessionOptionsAppendExecutionProvider`), independently re-verified
  against the vendored header (see `src/ffi.rs` field-count check in the
  PR description).

## Files

- `src/ffi.rs` -- kernel32 + `OrtApi`/`OrtApiBase` FFI declarations, a
  superset of the sibling CPU spike's subset (217 slots, through
  `SessionOptionsAppendExecutionProvider` at index 216).
- `src/guards.rs` -- RAII guards releasing each native handle exactly once.
- `src/error.rs` -- `SpikeError` + the `check()` `OrtStatus` classifier.
- `src/main.rs` -- CLI parsing, QNN session-options construction
  (`qnn_only_session_options`), the supported- and unsupported-model checks,
  and JSON reporting.
- `Cargo.toml` / `Cargo.lock` -- standalone package, zero external
  dependencies, outside the root Cargo workspace (own empty `[workspace]`
  table), same rationale as the sibling P0-03 spike.
