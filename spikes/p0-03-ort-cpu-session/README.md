# P0-03 spike: ORT C API CPU Session on Windows ARM64

Issue: [#4 `[P0-03] spike: Windows ARM64でORT C API CPU Sessionを実証する`](https://github.com/takurot/npu-router/issues/4)
Traceability: `docs/SPEC.md` OQ-01, OQ-03; Phase 0 gate (`docs/SPEC.md` 24章)

## Result

**Passed on real Windows 11 ARM64 hardware.** `mnist-12` golden inference
matched the pinned CPU golden output within tolerance, and a deliberate
failure (nonexistent model file) returned a classified `OrtStatus` error
instead of aborting the process.

```json
{
  "ort_version": "1.23.2",
  "ort_api_version_used": 23,
  "provider": "cpu",
  "input_name": "Input3",
  "output_name": "Plus214_Output_0",
  "input_dims": [1, 1, 28, 28],
  "latency_ms": 0.1973,
  "max_abs_diff": 0.000030517578,
  "within_tolerance": true,
  "negative_check_ort_error_code": 3,
  "negative_check_ort_error_message": "Load model from <path>\\__p0-03-spike-nonexistent-model__.onnx failed:Load model <path>\\__p0-03-spike-nonexistent-model__.onnx failed. File doesn't exist"
}
```

(`max_abs_diff = 3.05e-5` is well within the fixture's `atol=1e-4, rtol=1e-4`
per `docs/TEST_FIXTURES.md`. Full paths above are redacted to `<path>` for
this report; the raw log contains the local absolute path, which is expected
for a developer-run diagnostic tool, not a shipped API response.)

Exit code was `0` (success). Two further negative-path runs (missing CLI
args; nonexistent DLL path) also exited cleanly with classified error
messages and no crash — see "Negative-path evidence" below.

## Environment

| Item | Value |
| --- | --- |
| OS | Windows 11 Home, build `10.0.26200` (Windows 11 ARM64) |
| CPU | Snapdragon(R) X 10-core X1P64100 @ 3.40 GHz (Snapdragon X Plus) |
| Rust | `rustc 1.97.1 (8bab26f4f 2026-07-14)`, host `aarch64-pc-windows-msvc` |
| MSVC linker | VS2022 BuildTools, MSVC toolset `14.44.35207`, `Hostarm64\arm64` |
| ONNX Runtime | `1.23.2`, `win-arm64` official release |

## Artifact provenance

### ONNX Runtime

- Release: [`v1.23.2`](https://github.com/microsoft/onnxruntime/releases/tag/v1.23.2), asset `onnxruntime-win-arm64-1.23.2.zip`
- Download URL: `https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-win-arm64-1.23.2.zip`
- SHA-256 (zip, matches GitHub's published asset digest): `1cfe88b6435df3b5fb0e9f6bd7d6f5df1e887b6174de7f6e2a47bab956f3f168`
- SHA-256 (`lib/onnxruntime.dll`, extracted): `99a11bb077f723a81d100fd98099f332e4178bbb7025ac10fd40699a0b82c2f0`
- License: MIT (`LICENSE` in the release archive), third-party notices in `ThirdPartyNotices.txt`
- `ORT_API_VERSION` in the vendored `onnxruntime_c_api.h`: `23`
- This version was chosen to match the ORT tag (`a83fc4d58cb48eb68890dd689f94f28288cf2278`, resolves to `v1.23.2`) that `docs/TEST_FIXTURES.md` already pinned the `qdq_conv.onnx` / `qnn_ep_partial_support.onnx` fixtures from, so the CPU spike and the Phase 0 QNN fixtures share one ORT version.

Not committed to git (`.cache/` is gitignored, matching the existing test
fixture convention in `docs/TEST_FIXTURES.md`). To reproduce, download and
verify the SHA-256 above, then extract to
`.cache/vendor/onnxruntime-win-arm64-1.23.2/`.

### Model + golden vectors

Reused unmodified from Issue #3 / `docs/TEST_FIXTURES.md`:

```bash
node scripts/fetch-test-fixtures.mjs
```

populates `.cache/test-fixtures/classification/mnist-12.tar.gz`, verified
against `fixtures/catalog.json`. Extract it and use:

- `mnist-12/mnist-12.onnx` (SHA-256 `5c688690f8bacf667d4c2074af5ad0646ca328d7ab03eccf944a65b320171bdd`)
- `mnist-12/test_data_set_0/input_0.pb` (SHA-256 `d44b08082c3ded89e081f699a9d604239818c805ee8b5d03cd80f338e641c720`)
- `mnist-12/test_data_set_0/output_0.pb` (SHA-256 `153a5b1d96f9a544fc398f8c1837b994bbf5f26d3d12a7eff2ec63f7fb2317e1`)

## Reproduction

On a Windows ARM64 machine with the Rust MSVC ARM64 toolchain
(`aarch64-pc-windows-msvc`) and VS Build Tools (ARM64 `cl.exe`/`link.exe`)
installed:

```powershell
# 1. Fetch the pinned model/golden fixtures (from the repo root).
node scripts/fetch-test-fixtures.mjs

# 2. Download + verify + extract ONNX Runtime v1.23.2 win-arm64 (see hashes
#    above) into .cache/vendor/onnxruntime-win-arm64-1.23.2/.

# 3. Extract .cache/test-fixtures/classification/mnist-12.tar.gz.

# 4. Build and run the spike (from spikes/p0-03-ort-cpu-session/).
cargo build --release
target\release\p0-03-ort-cpu-session-spike.exe `
  ..\..\.cache\vendor\onnxruntime-win-arm64-1.23.2\lib\onnxruntime.dll `
  ..\..\.cache\test-fixtures\classification\mnist-12\mnist-12.onnx `
  ..\..\.cache\test-fixtures\classification\mnist-12\test_data_set_0\input_0.pb `
  ..\..\.cache\test-fixtures\classification\mnist-12\test_data_set_0\output_0.pb
```

Exit code `0` and `"within_tolerance": true` mean the spike passed.

## Negative-path evidence (AC: no process abort on failure)

Three distinct failure modes were exercised; all returned classified errors
on stderr with a non-zero exit code, no panic, no abort:

1. **Wrong CLI arg count** → `exit 2`, `usage error: expected 4 arguments: ...`
2. **Nonexistent DLL path** → `exit 2`, `io error on <path>: <ファイルが見つかりません> (os error 2)` (canonicalization fails before any native call)
3. **Nonexistent model path, mid-run** (built into every successful run as a
   deliberate check reusing the already-open `OrtEnv`/`OrtSessionOptions`) →
   classified `OrtStatus`, `code=3` (`ORT_NO_SUCHFILE`), captured in the
   `negative_check_*` fields of the JSON report above.

## DLL load: fixed, restricted search order

`onnxruntime.dll` is loaded via `LoadLibraryExW` with
`LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32` and a
canonicalized (`std::fs::canonicalize`) absolute path — never a bare
filename. This means: dependent-DLL resolution only consults the directory
containing `onnxruntime.dll` itself and `%SystemRoot%\System32`; the process
CWD, the default DLL search order, and `PATH` are never consulted. This
matches `docs/SPEC.md` 18.2 ("DLL探索順序を固定し、CWDから任意DLLをloadしない").

`onnxruntime.dll` in this ORT release does not require
`onnxruntime_providers_shared.dll` to be loaded up front for the CPU EP;
`onnxruntime_providers_shared.dll` sits next to it in `lib/` for EPs that do
need it (not exercised by this CPU-only spike).

## Allocator / lifetime / `OrtStatus` notes (Scope item)

- **Ownership model**: every native handle this spike creates
  (`OrtEnv`, `OrtSessionOptions`, `OrtSession`, `OrtValue` ×2,
  `OrtMemoryInfo`, `OrtTensorTypeAndShapeInfo`, the loaded module) is wrapped
  in a small RAII guard whose `Drop` calls the matching
  `Release*`/`FreeLibrary` function exactly once. Rust's normal drop order
  (reverse of declaration) releases them in the correct dependency order
  without needing to hand-sequence cleanup.
- **`CreateTensorWithDataAsOrtValue`** does not copy the input buffer — the
  resulting `OrtValue` is a *view*. The spike keeps the backing `Vec<f32>`
  alive in the same stack frame until after `Run` returns, and never mutates
  or reallocates it in between.
- **`SessionGetInputName`/`SessionGetOutputName`** return allocator-owned
  strings that the caller must free via `OrtApi::AllocatorFree` using the
  *same* allocator (`GetAllocatorWithDefaultOptions`); the spike does this
  immediately after copying the name into an owned Rust `String`.
- **`OrtStatus`**: every API call returns `*mut OrtStatus`; a null pointer
  means success. Any non-null status is converted to a classified
  `SpikeError::Ort { code, message }` and released via `ReleaseStatus`
  before propagating — the process never inspects a status twice or forgets
  to release one, verified by routing every fallible call through one
  `check()` helper.
- **Thread model**: this spike is entirely single-threaded (matches the
  MVP's per-Session serialized-access assumption for Phase 1); Issue #6
  (P0-05) covers concurrent `Run`/threading behavior separately.

## `OrtApi` field derivation (Non-goals: not a general ORT binding)

The ORT C API exposes its ~382-function vtable as one `struct OrtApi` whose
fields are appended-only across versions (never reordered/removed), reached
via `OrtApiBase::GetApi(ORT_API_VERSION)`. This spike only calls ~20 of those
functions, so `src/main.rs` declares `OrtApi` as a `#[repr(C)]` struct whose
field order exactly matches the vendored `onnxruntime_c_api.h`, with the ~20
called functions given their real C signatures and every other field
declared as untyped `*const c_void` padding (individually or in same-sized
arrays) — a valid, layout-compatible *prefix* of the real struct for every
field this code actually touches. The field order was verified by
mechanically extracting all member names from the header in declaration
order (regex over both its `TYPE(ORT_API_CALL* Name)(...)` and
`ORT_API2_STATUS(Name, ...)` declaration styles) and cross-checking the
result against this file's struct, not transcribed by hand. This technique
is intentionally narrow to this spike's ~20 calls; the product `npu-ort`
crate (Issue #13) should use either a maintained ORT binding crate or its
own from-scratch review of the full header, not a copy of this file.

## Known issue observed (informational, not a spike failure)

ONNX Runtime's bundled `cpuinfo` library does not recognize the CPU name
string reported by this exact Snapdragon X SoC and prints a startup warning
to stderr:

```text
Error in cpuinfo: Unknown chip model name 'Snapdragon(R) X 10-core X1P64100 @ 3.40 GHz'.
Please add new Windows on Arm SoC/chip support to arm/windows/init.c!
onnxruntime cpuid_info warning: Unknown CPU vendor. cpuinfo_vendor value: 0
```

Inference still succeeded correctly (see result above), so this did not
block the spike. Worth carrying into ADR-001's support matrix as a known,
non-blocking warning for this exact CPU model with ORT v1.23.2, and into
`doctor` diagnostics scope (Issue #21) as a benign stderr line to not
misclassify as an error.

## Non-goals (per Issue #4)

This is a throwaway technical spike, not the product `npu-ort` wrapper:

- No abstraction over multiple models/sessions, no Session Manager, no
  concurrency/queueing (Issues #15, #16).
- No general-purpose protobuf parser — `parse_tensor_proto` only understands
  the exact three `TensorProto` fields (`dims`, `data_type`, `raw_data`)
  these two fixture files use, skipping unknown fields by wire type.
- No production error taxonomy (`NPU0xx`, `docs/SPEC.md` 17章) — that is
  `npu-core`'s job (Issue #10); this spike's `SpikeError` only classifies
  enough to satisfy the AC ("failures are classified, not aborts").
- No QNN Execution Provider (Issue #5) or concurrency/timeout/cancel
  behavior (Issue #6) — CPU-only, single call, single thread.

## Files

- `src/ffi.rs` — kernel32 + `OrtApi`/`OrtApiBase` FFI declarations (the
  layout-compatible struct described above).
- `src/guards.rs` — RAII guards releasing each native handle exactly once.
- `src/error.rs` — `SpikeError` + the `check()` `OrtStatus` classifier.
- `src/protobuf.rs` — the fixture-specific `TensorProto` reader, with unit
  tests against the real, pinned `output_0.pb` bytes plus a few malformed
  inputs (truncated varint, overrunning length, unknown-field skipping).
- `src/main.rs` — CLI parsing and the `run()` procedure that ties the above
  together into one Env→Session→Run→compare flow.
- `Cargo.toml` / `Cargo.lock` — standalone package, zero external
  dependencies (only kernel32 FFI and the vendored ORT C API headers/DLL).
  `unsafe_code` is not forbidden here; this crate is deliberately outside
  the workspace via its own empty `[workspace]` table in `Cargo.toml`, so
  the root workspace's `unsafe_code = "forbid"` lint does not apply to it.

Run `cargo test` (from this directory) to run the `protobuf` unit tests;
`cargo build --release` + the reproduction command above for the full,
hardware-dependent spike.
