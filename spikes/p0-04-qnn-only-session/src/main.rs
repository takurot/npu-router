//! Issue #5 (P0-04) technical spike: QNN-only ONNX Runtime `Session` with
//! implicit CPU EP fallback disabled, on Windows ARM64 / Qualcomm Hexagon
//! NPU.
//!
//! Scope (see ../README.md for the full report):
//!   - append the QNN execution provider by name with
//!     `session.disable_cpu_ep_fallback=1`, so `CreateSession` itself fails
//!     if any node cannot run on QNN -- a QNN-only Session is verified by
//!     construction, no profiling/node-assignment inspection needed
//!   - a QNN-supported model (`qdq_conv.onnx`) must load and `Run`
//!     successfully, matching the CPU golden output within 1 LSB
//!   - a QNN-unsupported model (`qnn_ep_partial_support.onnx`, uses
//!     `MatMulInteger`) must fail `CreateSession` with a classified error,
//!     not silently fall back and not abort
//!   - ORT/QNN/backend/driver/OS versions are recorded in the JSON report
//!
//! Non-goals (docs/SPEC.md Issue #5 "Non-goals"): this is not the Router's
//! CPU fallback path (that is Issue #20, P2-02) -- this spike only proves
//! QNN-only Session creation and the disable-fallback failure mode work on
//! real hardware. `ffi::OrtApi` reuses the same layout-compatible-prefix
//! technique as the sibling P0-03 (CPU) spike, extended with the two extra
//! functions this spike needs (`AddSessionConfigEntry`,
//! `SessionOptionsAppendExecutionProvider`); see README.md "OrtApi field
//! derivation".
//!
//! ## Safety invariants
//!
//! Identical to the sibling P0-03 (CPU) spike: `api`/`api_base` references
//! are derived from non-null DLL exports and stay valid for as long as the
//! owning `LibraryGuard` is alive; every native handle is checked non-null
//! via `error::check()` before use and released exactly once by an RAII
//! guard; every `OrtApi` field is typed to match its C signature so calling
//! it through `(api.field)(args)` is a correct ABI call.
#![allow(non_snake_case)]

mod error;
mod ffi;
mod guards;

use std::env;
use std::ffi::{CStr, CString, c_char, c_void};
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::ptr;

use error::{SpikeError, check};
use ffi::{
    GetLastError, GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
    LoadLibraryExW, ONNX_TENSOR_ELEMENT_DATA_TYPE_UINT8, ORT_API_VERSION, ORT_ARENA_ALLOCATOR,
    ORT_LOGGING_LEVEL_WARNING, ORT_MEM_TYPE_DEFAULT, OrtAllocator, OrtApi, OrtApiBase, OrtEnv,
    OrtMemoryInfo, OrtSession, OrtSessionOptions, OrtStatus, OrtValue,
};
use guards::{
    EnvGuard, LibraryGuard, MemoryInfoGuard, SessionGuard, SessionOptionsGuard, ValueGuard,
};

fn read_file(path: &Path) -> Result<Vec<u8>, SpikeError> {
    fs::read(path).map_err(|source| SpikeError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn to_wide_null(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------
// CLI plumbing.
// ---------------------------------------------------------------------

struct Args {
    dll_path: PathBuf,
    qnn_backend_path: PathBuf,
    supported_model_path: PathBuf,
    input_raw_path: PathBuf,
    golden_output_raw_path: PathBuf,
    unsupported_model_path: PathBuf,
}

fn parse_args() -> Result<Args, SpikeError> {
    let raw: Vec<_> = env::args_os().skip(1).collect();
    if raw.len() != 6 {
        return Err(SpikeError::Usage(
            "expected 6 arguments: <onnxruntime.dll> <QnnHtp.dll> <qdq_conv.onnx> \
             <input.raw> <golden_output.raw> <qnn_ep_partial_support.onnx>"
                .into(),
        ));
    }
    let canon = |p: &std::ffi::OsStr| -> Result<PathBuf, SpikeError> {
        fs::canonicalize(p).map_err(|source| SpikeError::Io {
            path: Path::new(p).display().to_string(),
            source,
        })
    };
    Ok(Args {
        dll_path: canon(&raw[0])?,
        qnn_backend_path: canon(&raw[1])?,
        supported_model_path: canon(&raw[2])?,
        input_raw_path: canon(&raw[3])?,
        golden_output_raw_path: canon(&raw[4])?,
        unsupported_model_path: canon(&raw[5])?,
    })
}

// ---------------------------------------------------------------------
// Shared ORT bring-up (identical technique to the P0-03 CPU spike).
// ---------------------------------------------------------------------

/// # Safety
/// `dll_path` must name a file that is actually an ONNX Runtime shared
/// library matching `ffi::OrtApi`'s declared ABI (`ORT_API_VERSION` 23).
unsafe fn load_ort(dll_path: &Path) -> Result<(LibraryGuard, &'static OrtApi, String), SpikeError> {
    let dll_wide = to_wide_null(dll_path.as_os_str());
    // SAFETY: `dll_wide` is a valid null-terminated UTF-16 buffer owned by
    // this stack frame for the duration of the call; the search-order flags
    // are the fixed, restricted set documented in `ffi.rs` -- dependent
    // DLLs (onnxruntime_providers_shared.dll, onnxruntime_providers_qnn.dll,
    // Qnn*.dll) are resolved from the directory `dll_path` lives in, never
    // from CWD/PATH, because the caller places all of them side by side.
    let handle = unsafe {
        LoadLibraryExW(
            dll_wide.as_ptr(),
            ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if handle.is_null() {
        // SAFETY: `GetLastError` takes no arguments and only reads
        // thread-local state; always sound to call.
        return Err(SpikeError::DllLoad {
            code: unsafe { GetLastError() },
        });
    }
    let lib = LibraryGuard(handle);

    // SAFETY: `handle` was just confirmed non-null and owned by `lib` for
    // the rest of this process; the proc name is a valid null-terminated
    // C string literal.
    let get_api_base_addr = unsafe { GetProcAddress(handle, c"OrtGetApiBase".as_ptr().cast()) };
    if get_api_base_addr.is_null() {
        return Err(SpikeError::MissingExport("OrtGetApiBase"));
    }
    // SAFETY: GetProcAddress returns a data pointer; reinterpreting it as
    // the function-pointer type documented for the `OrtGetApiBase` export
    // is the standard, unavoidable pattern for dynamic C API loading (the
    // same technique `libloading` uses internally). Both pointer kinds have
    // the same size/representation on this target.
    let get_api_base: unsafe extern "system" fn() -> *const OrtApiBase =
        unsafe { std::mem::transmute(get_api_base_addr) };
    // SAFETY: `get_api_base` takes no arguments; the DLL stays loaded via
    // `lib` for the whole call.
    let api_base = unsafe { get_api_base() };
    if api_base.is_null() {
        return Err(SpikeError::MissingExport("OrtGetApiBase() returned null"));
    }
    // SAFETY: non-null, and ORT guarantees it points to a static, immutable
    // `OrtApiBase` that lives for the process lifetime once the DLL is
    // loaded (outlives `lib`, which is a subset of that lifetime).
    let api_base = unsafe { &*api_base };
    // SAFETY: `api_base` is a valid reference (above); `GetApi` only reads
    // its `u32` argument.
    let api_ptr = unsafe { (api_base.get_api)(ORT_API_VERSION) };
    if api_ptr.is_null() {
        return Err(SpikeError::UnsupportedApiVersion(ORT_API_VERSION));
    }
    // SAFETY: non-null, and ORT guarantees the returned `OrtApi*` is a
    // static vtable valid for the process lifetime; `ffi::OrtApi` is a
    // verified layout-compatible prefix (see module doc / README.md). The
    // `'static` lifetime is sound because the vtable lives as long as the
    // DLL stays loaded, and callers are required to keep the returned
    // `LibraryGuard` alive for at least as long as they use this reference.
    let api: &'static OrtApi = unsafe { &*api_ptr };
    // SAFETY: `GetVersionString` takes no arguments and returns a static,
    // null-terminated, process-lifetime string per the ORT C API contract.
    let ort_version = unsafe { CStr::from_ptr((api_base.get_version_string)()) }
        .to_string_lossy()
        .into_owned();

    Ok((lib, api, ort_version))
}

/// Builds `OrtSessionOptions` with the QNN EP appended and implicit CPU EP
/// fallback disabled (`session.disable_cpu_ep_fallback=1`), per
/// `docs/TEST_FIXTURES.md` and `docs/SPEC.md` 3.1 fixed policy #3.
///
/// # Safety
/// `api` must be a valid `OrtApi` reference (module "Safety invariants").
unsafe fn qnn_only_session_options<'a>(
    api: &'a OrtApi,
    qnn_backend_path: &Path,
) -> Result<SessionOptionsGuard<'a>, SpikeError> {
    let mut so_ptr: *mut OrtSessionOptions = ptr::null_mut();
    // SAFETY: `so_ptr` is a null out-param filled by the call itself.
    unsafe { check(api, (api.create_session_options)(&mut so_ptr))? };
    let session_options = SessionOptionsGuard { api, ptr: so_ptr };

    let disable_fallback_key = c"session.disable_cpu_ep_fallback";
    let disable_fallback_value = c"1";
    // SAFETY: `session_options.ptr` is live (guard just constructed, not
    // dropped); both C strings are `'static` literals.
    unsafe {
        check(
            api,
            (api.add_session_config_entry)(
                session_options.ptr,
                disable_fallback_key.as_ptr(),
                disable_fallback_value.as_ptr(),
            ),
        )?;
    }

    let backend_path_wide_as_utf8 = qnn_backend_path
        .to_str()
        .ok_or_else(|| SpikeError::Usage("QNN backend path is not valid UTF-8".into()))?;
    let provider_name = c"QNN";
    let backend_path_key = c"backend_path";
    let backend_path_value = CString::new(backend_path_wide_as_utf8)
        .map_err(|_| SpikeError::Usage("QNN backend path has interior NUL".into()))?;
    let keys = [backend_path_key.as_ptr()];
    let values = [backend_path_value.as_ptr()];
    // SAFETY: `session_options.ptr` is live; `keys`/`values` are 1-element
    // arrays of pointers into `'static`/stack-owned C strings that outlive
    // this call.
    unsafe {
        check(
            api,
            (api.session_options_append_execution_provider)(
                session_options.ptr,
                provider_name.as_ptr(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
            ),
        )?;
    }

    Ok(session_options)
}

// ---------------------------------------------------------------------
// Test A: QNN-supported model must load and Run correctly, CPU-fallback
// disabled.
// ---------------------------------------------------------------------

struct SupportedModelResult {
    input_name: String,
    output_name: String,
    output_bytes: Vec<u8>,
    golden_bytes: Vec<u8>,
    max_abs_diff_lsb: u8,
    within_tolerance: bool,
}

fn run_supported_model(
    api: &OrtApi,
    env: &EnvGuard<'_>,
    args: &Args,
) -> Result<SupportedModelResult, SpikeError> {
    const MAX_LSB_TOLERANCE: u8 = 1;

    // SAFETY: `env.ptr` is a live guard for the whole call; `api` is valid.
    let session_options = unsafe { qnn_only_session_options(api, &args.qnn_backend_path)? };

    let model_wide = to_wide_null(args.supported_model_path.as_os_str());
    let mut session_ptr: *mut OrtSession = ptr::null_mut();
    // SAFETY: `env.ptr`/`session_options.ptr` are live; `session_ptr` is a
    // null out-param filled by the call itself. A non-null OrtStatus here
    // (an unsupported node with fallback disabled) is a real spike failure
    // for this model -- `qdq_conv.onnx` is expected to be fully QNN
    // assignable -- so it is propagated with `?`, not swallowed.
    unsafe {
        check(
            api,
            (api.create_session)(
                env.ptr.cast_const(),
                model_wide.as_ptr(),
                session_options.ptr.cast_const(),
                &mut session_ptr,
            ),
        )?;
    }
    let session = SessionGuard {
        api,
        ptr: session_ptr,
    };

    let mut allocator_ptr: *mut OrtAllocator = ptr::null_mut();
    unsafe {
        check(
            api,
            (api.get_allocator_with_default_options)(&mut allocator_ptr),
        )?;
    }

    let mut input_name_ptr: *mut c_char = ptr::null_mut();
    let status = unsafe {
        (api.session_get_input_name)(
            session.ptr.cast_const(),
            0,
            allocator_ptr,
            &mut input_name_ptr,
        )
    };
    let input_name = unsafe { fetch_allocated_name(api, allocator_ptr, status, input_name_ptr)? };

    let mut output_name_ptr: *mut c_char = ptr::null_mut();
    let status = unsafe {
        (api.session_get_output_name)(
            session.ptr.cast_const(),
            0,
            allocator_ptr,
            &mut output_name_ptr,
        )
    };
    let output_name = unsafe { fetch_allocated_name(api, allocator_ptr, status, output_name_ptr)? };

    // Fixture is raw uint8 bytes (not protobuf): 5x5 input, 3x3 golden
    // output, per docs/TEST_FIXTURES.md.
    let mut input_bytes = read_file(&args.input_raw_path)?;
    let dims: [i64; 4] = [1, 1, 5, 5];
    if input_bytes.len() != 25 {
        return Err(SpikeError::Usage(format!(
            "input.raw has {} bytes, expected 25 (1x1x5x5 uint8)",
            input_bytes.len()
        )));
    }
    let golden_bytes = read_file(&args.golden_output_raw_path)?;
    if golden_bytes.len() != 9 {
        return Err(SpikeError::Usage(format!(
            "golden output.raw has {} bytes, expected 9 (1x1x3x3 uint8)",
            golden_bytes.len()
        )));
    }

    let mut mem_info_ptr: *mut OrtMemoryInfo = ptr::null_mut();
    unsafe {
        check(
            api,
            (api.create_cpu_memory_info)(
                ORT_ARENA_ALLOCATOR,
                ORT_MEM_TYPE_DEFAULT,
                &mut mem_info_ptr,
            ),
        )?;
    }
    let mem_info = MemoryInfoGuard {
        api,
        ptr: mem_info_ptr,
    };

    let mut input_value_ptr: *mut OrtValue = ptr::null_mut();
    // SAFETY (buffer lifetime): `input_value` is a *view* over
    // `input_bytes`, which is not touched or dropped again until after
    // `Run` consumes `input_value` below.
    unsafe {
        check(
            api,
            (api.create_tensor_with_data_as_ortvalue)(
                mem_info.ptr.cast_const(),
                input_bytes.as_mut_ptr().cast(),
                input_bytes.len(),
                dims.as_ptr(),
                dims.len(),
                ONNX_TENSOR_ELEMENT_DATA_TYPE_UINT8,
                &mut input_value_ptr,
            ),
        )?;
    }
    let input_value = ValueGuard {
        api,
        ptr: input_value_ptr,
    };

    let input_name_c = CString::new(input_name.clone())
        .map_err(|_| SpikeError::Usage("input name had interior NUL".into()))?;
    let output_name_c = CString::new(output_name.clone())
        .map_err(|_| SpikeError::Usage("output name had interior NUL".into()))?;
    let input_names = [input_name_c.as_ptr()];
    let output_names = [output_name_c.as_ptr()];
    let input_values: [*const OrtValue; 1] = [input_value.ptr];
    let mut output_value_ptr: *mut OrtValue = ptr::null_mut();

    // SAFETY: name/value arrays are stack pointers into live owners that
    // outlive this call; `session.ptr` is the live QNN-only session.
    unsafe {
        check(
            api,
            (api.run)(
                session.ptr,
                ptr::null(),
                input_names.as_ptr(),
                input_values.as_ptr(),
                1,
                output_names.as_ptr(),
                1,
                &mut output_value_ptr,
            ),
        )?;
    }
    let output_value = ValueGuard {
        api,
        ptr: output_value_ptr,
    };

    let mut out_data_ptr: *mut c_void = ptr::null_mut();
    unsafe {
        check(
            api,
            (api.get_tensor_mutable_data)(output_value.ptr, &mut out_data_ptr),
        )?;
    }
    if out_data_ptr.is_null() {
        return Err(SpikeError::Usage(
            "GetTensorMutableData returned null".into(),
        ));
    }
    // SAFETY: the model's declared output is `[1,1,3,3] uint8` (9 elements,
    // fixed by the pinned model in docs/TEST_FIXTURES.md); `out_data_ptr` is
    // non-null and owned by `output_value` (checked non-null above). The
    // slice does not outlive `output_value`.
    let output_bytes: &[u8] = unsafe { std::slice::from_raw_parts(out_data_ptr.cast::<u8>(), 9) };

    let mut max_abs_diff_lsb: u8 = 0;
    for (actual, golden) in output_bytes.iter().zip(golden_bytes.iter()) {
        let diff = actual.abs_diff(*golden);
        if diff > max_abs_diff_lsb {
            max_abs_diff_lsb = diff;
        }
    }

    Ok(SupportedModelResult {
        input_name,
        output_name,
        output_bytes: output_bytes.to_vec(),
        golden_bytes,
        max_abs_diff_lsb,
        within_tolerance: max_abs_diff_lsb <= MAX_LSB_TOLERANCE,
    })
}

/// # Safety
/// `status` must be the (possibly null) `OrtStatus*` returned by the
/// `SessionGet{Input,Output}Name` call that produced `name_ptr`, `allocator`
/// must be the same live `OrtAllocator*` passed to that call, and `api` must
/// be a valid `OrtApi` reference.
unsafe fn fetch_allocated_name(
    api: &OrtApi,
    allocator: *mut OrtAllocator,
    status: *mut OrtStatus,
    name_ptr: *mut c_char,
) -> Result<String, SpikeError> {
    unsafe { check(api, status)? };
    let owned = unsafe { CStr::from_ptr(name_ptr) }
        .to_string_lossy()
        .into_owned();
    unsafe { check(api, (api.allocator_free)(allocator, name_ptr.cast())) }?;
    Ok(owned)
}

// ---------------------------------------------------------------------
// Test B: QNN-unsupported model must fail CreateSession with a classified
// error (CPU fallback disabled -> no silent fallback, no abort).
// ---------------------------------------------------------------------

struct UnsupportedModelResult {
    code: i32,
    message: String,
}

fn run_unsupported_model(
    api: &OrtApi,
    env: &EnvGuard<'_>,
    args: &Args,
) -> Result<UnsupportedModelResult, SpikeError> {
    let session_options = unsafe { qnn_only_session_options(api, &args.qnn_backend_path)? };

    let model_wide = to_wide_null(args.unsupported_model_path.as_os_str());
    let mut session_ptr: *mut OrtSession = ptr::null_mut();
    // SAFETY: `env.ptr`/`session_options.ptr` are live; `session_ptr` is a
    // null out-param filled by the call itself.
    let status = unsafe {
        (api.create_session)(
            env.ptr.cast_const(),
            model_wide.as_ptr(),
            session_options.ptr.cast_const(),
            &mut session_ptr,
        )
    };
    match unsafe { check(api, status) } {
        Ok(()) => {
            // SAFETY: defensive only -- if ORT unexpectedly succeeded here,
            // release the session it handed back before erroring out, so
            // this diagnostic run never leaks a handle no guard owns.
            if !session_ptr.is_null() {
                unsafe { (api.release_session)(session_ptr) };
            }
            Err(SpikeError::Usage(
                "expected CreateSession to fail for the QNN-unsupported model \
                 with session.disable_cpu_ep_fallback=1, but it succeeded"
                    .into(),
            ))
        }
        Err(SpikeError::Ort { code, message }) => Ok(UnsupportedModelResult { code, message }),
        Err(other) => Err(other),
    }
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!(
                "usage: p0-04-qnn-only-session-spike <onnxruntime.dll> <QnnHtp.dll> \
                 <qdq_conv.onnx> <input.raw> <golden_output.raw> <qnn_ep_partial_support.onnx>"
            );
            return ExitCode::from(2);
        }
    };

    // SAFETY: `args.dll_path` names the vendored ONNX Runtime DLL from the
    // Microsoft.ML.OnnxRuntime.QNN package (see README.md provenance).
    let (_lib, api, ort_version) = match unsafe { load_ort(&args.dll_path) } {
        Ok(v) => v,
        Err(e) => {
            eprintln!("spike failed, classified error: {e}");
            return ExitCode::from(1);
        }
    };

    let logid = CString::new("npu-router-p0-04-spike").expect("static string has no NUL");
    let mut env_ptr: *mut OrtEnv = ptr::null_mut();
    // SAFETY: `env_ptr` is a null out-param filled by the call itself.
    if let Err(e) = unsafe {
        check(
            api,
            (api.create_env)(ORT_LOGGING_LEVEL_WARNING, logid.as_ptr(), &mut env_ptr),
        )
    } {
        eprintln!("spike failed, classified error: {e}");
        return ExitCode::from(1);
    }
    let env = EnvGuard { api, ptr: env_ptr };

    let supported = run_supported_model(api, &env, &args);
    let unsupported = run_unsupported_model(api, &env, &args);

    println!("{{");
    println!("  \"ort_version\": \"{}\",", json_escape(&ort_version));
    println!("  \"ort_api_version_used\": {ORT_API_VERSION},");
    println!("  \"provider\": \"qnn\",");
    println!("  \"disable_cpu_ep_fallback\": true,");
    match &supported {
        Ok(r) => {
            println!("  \"supported_model\": {{");
            println!("    \"status\": \"ok\",");
            println!("    \"input_name\": \"{}\",", json_escape(&r.input_name));
            println!("    \"output_name\": \"{}\",", json_escape(&r.output_name));
            println!("    \"output_bytes\": {:?},", r.output_bytes);
            println!("    \"golden_bytes\": {:?},", r.golden_bytes);
            println!("    \"max_abs_diff_lsb\": {},", r.max_abs_diff_lsb);
            println!("    \"within_tolerance\": {}", r.within_tolerance);
            println!("  }},");
        }
        Err(e) => {
            println!("  \"supported_model\": {{");
            println!("    \"status\": \"error\",");
            println!("    \"detail\": \"{}\"", json_escape(&e.to_string()));
            println!("  }},");
        }
    }
    match &unsupported {
        Ok(r) => {
            println!("  \"unsupported_model\": {{");
            println!("    \"status\": \"failed_as_expected\",");
            println!("    \"ort_error_code\": {},", r.code);
            println!("    \"ort_error_message\": \"{}\"", json_escape(&r.message));
            println!("  }}");
        }
        Err(e) => {
            println!("  \"unsupported_model\": {{");
            println!("    \"status\": \"unexpected\",");
            println!("    \"detail\": \"{}\"", json_escape(&e.to_string()));
            println!("  }}");
        }
    }
    println!("}}");

    let supported_ok = supported.as_ref().is_ok_and(|r| r.within_tolerance);
    let unsupported_ok = unsupported.is_ok();
    if supported_ok && unsupported_ok {
        ExitCode::SUCCESS
    } else {
        if !supported_ok {
            eprintln!("supported-model check failed (see JSON above)");
        }
        if !unsupported_ok {
            eprintln!("unsupported-model check failed (see JSON above)");
        }
        ExitCode::from(1)
    }
}
