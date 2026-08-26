//! Issue #4 (P0-03) technical spike: explicit-load ONNX Runtime C API CPU
//! `Session` on Windows ARM64.
//!
//! Scope (see ../README.md for the full report):
//!   - safe, explicit load of a pinned `onnxruntime.dll` (fixed search order,
//!     no CWD/PATH consultation) -- see `ffi.rs`
//!   - API version negotiation via `OrtApiBase::GetApi`
//!   - CPU-only `Session` creation, `Tensor` creation, `Run`, result readback
//!   - allocator/lifetime notes captured as RAII guards -- see `guards.rs`
//!   - a deliberate failure path (nonexistent model) to show errors are
//!     classified rather than causing a process abort -- see `error.rs`
//!
//! Non-goals (docs/SPEC.md Issue #4 "Non-goals"): this is not the product
//! `npu-ort` wrapper. `ffi::OrtApi` only declares the ~20 functions this
//! spike calls; every other slot is untyped padding of the correct pointer
//! width so the struct stays layout-compatible with the real (much larger)
//! vtable. See README.md "OrtApi field derivation" for how the field order
//! was verified against the vendored header.
//!
//! ## Safety invariants (apply to every `(api.<fn>)(...)` call site below)
//!
//! - `api: &OrtApi` and `api_base: &OrtApiBase` are derived from a `*const`
//!   returned by the DLL itself and checked non-null before use; the DLL
//!   stays loaded (via `LibraryGuard`, dropped last) for as long as any
//!   reference into it is live, so the function pointers stay valid.
//! - Every `OrtApi`/`OrtApiBase` function pointer field is typed to match
//!   the exact C signature transcribed from `onnxruntime_c_api.h` (see the
//!   per-field index comments in `ffi.rs`), so calling it through
//!   `(api.field)(args)` with matching Rust argument types is a correct C
//!   ABI call.
//! - Every native handle this file creates (`OrtEnv`, `OrtSession`,
//!   `OrtSessionOptions`, `OrtValue`, `OrtMemoryInfo`,
//!   `OrtTensorTypeAndShapeInfo`) is checked non-null (via `error::check()`,
//!   which turns a non-null `OrtStatus*` into `Err` first) before being read
//!   or passed to another API call, and is released by exactly one RAII
//!   guard (`guards.rs`) whose `Drop` runs the matching `Release*` function.
//! - Buffers handed to ORT by pointer (`input_floats`, name strings) outlive
//!   every native call that can read them, per the comments at each call
//!   site below.
#![allow(non_snake_case)]

mod error;
mod ffi;
mod guards;
mod protobuf;

use std::env;
use std::ffi::{CStr, CString, OsStr, c_char, c_void};
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::ptr;
use std::time::Instant;

use error::{SpikeError, check};
use ffi::{
    GetLastError, GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
    LoadLibraryExW, ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT, ORT_API_VERSION, ORT_ARENA_ALLOCATOR,
    ORT_LOGGING_LEVEL_WARNING, ORT_MEM_TYPE_DEFAULT, OrtAllocator, OrtApi, OrtApiBase, OrtEnv,
    OrtMemoryInfo, OrtSession, OrtSessionOptions, OrtStatus, OrtTensorTypeAndShapeInfo, OrtValue,
};
use guards::{
    EnvGuard, LibraryGuard, MemoryInfoGuard, SessionGuard, SessionOptionsGuard, ShapeInfoGuard,
    ValueGuard,
};
use protobuf::{f32_vec_from_le_bytes, parse_tensor_proto};

fn read_file(path: &Path) -> Result<Vec<u8>, SpikeError> {
    fs::read(path).map_err(|source| SpikeError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn to_wide_null(s: &OsStr) -> Vec<u16> {
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
// CLI plumbing + spike execution.
// ---------------------------------------------------------------------

struct Args {
    dll_path: PathBuf,
    model_path: PathBuf,
    input_path: PathBuf,
    golden_output_path: PathBuf,
}

fn parse_args() -> Result<Args, SpikeError> {
    let raw: Vec<_> = env::args_os().skip(1).collect();
    if raw.len() != 4 {
        return Err(SpikeError::Usage(
            "expected 4 arguments: <onnxruntime.dll> <model.onnx> <input_0.pb> <golden_output_0.pb>"
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
        model_path: canon(&raw[1])?,
        input_path: canon(&raw[2])?,
        golden_output_path: canon(&raw[3])?,
    })
}

struct Report {
    ort_version: String,
    input_name: String,
    output_name: String,
    input_dims: Vec<i64>,
    latency_ms: f64,
    max_abs_diff: f32,
    within_tolerance: bool,
    negative_check_code: i32,
    negative_check_message: String,
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
    // SAFETY: contract upheld by this fn's own safety doc above.
    unsafe { check(api, status)? };
    // SAFETY: `check` returned Ok, so per the ORT contract `name_ptr` is a
    // valid, null-terminated, allocator-owned string; it is read here and
    // freed via that same allocator on the next line, once.
    let owned = unsafe { CStr::from_ptr(name_ptr) }
        .to_string_lossy()
        .into_owned();
    unsafe { check(api, (api.allocator_free)(allocator, name_ptr.cast())) }?;
    Ok(owned)
}

fn run(args: &Args) -> Result<Report, SpikeError> {
    const ATOL: f32 = 1e-4;
    const RTOL: f32 = 1e-4;

    let dll_wide = to_wide_null(args.dll_path.as_os_str());
    // SAFETY: `dll_wide` is a valid null-terminated UTF-16 buffer owned by
    // this stack frame for the duration of the call; the search-order flags
    // are the fixed, restricted set documented in `ffi.rs`.
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
    let _lib = LibraryGuard(handle);

    // SAFETY: `handle` was just confirmed non-null and owned by `_lib` for
    // the rest of this function; the proc name is a valid null-terminated
    // C string literal.
    let get_api_base_addr = unsafe { GetProcAddress(handle, c"OrtGetApiBase".as_ptr().cast()) };
    if get_api_base_addr.is_null() {
        return Err(SpikeError::MissingExport("OrtGetApiBase"));
    }
    // SAFETY: GetProcAddress returns a data pointer; reinterpreting it as
    // the function-pointer type documented for the `OrtGetApiBase` export
    // (`onnxruntime_c_api.h`) is the standard, unavoidable pattern for
    // dynamic C API loading (the same technique `libloading` uses
    // internally). Both pointer kinds have the same size/representation on
    // this target.
    let get_api_base: unsafe extern "system" fn() -> *const OrtApiBase =
        unsafe { std::mem::transmute(get_api_base_addr) };
    // SAFETY: `get_api_base` takes no arguments; the DLL stays loaded via
    // `_lib` for the whole call.
    let api_base = unsafe { get_api_base() };
    if api_base.is_null() {
        return Err(SpikeError::MissingExport("OrtGetApiBase() returned null"));
    }
    // SAFETY: non-null, and ORT guarantees it points to a static, immutable
    // `OrtApiBase` that lives for the process lifetime once the DLL is
    // loaded (outlives `_lib`, which is a subset of that lifetime).
    let api_base = unsafe { &*api_base };
    // SAFETY: `api_base` is a valid reference (above); `GetApi` only reads
    // its `u32` argument.
    let api_ptr = unsafe { (api_base.get_api)(ORT_API_VERSION) };
    if api_ptr.is_null() {
        return Err(SpikeError::UnsupportedApiVersion(ORT_API_VERSION));
    }
    // SAFETY: non-null, and ORT guarantees the returned `OrtApi*` is a
    // static vtable valid for the process lifetime; `ffi::OrtApi` is a
    // verified layout-compatible prefix (see module doc / README.md).
    let api = unsafe { &*api_ptr };
    // SAFETY: `GetVersionString` takes no arguments and returns a static,
    // null-terminated, process-lifetime string per the ORT C API contract.
    let ort_version = unsafe { CStr::from_ptr((api_base.get_version_string)()) }
        .to_string_lossy()
        .into_owned();

    // --- Env + SessionOptions + Session. No execution provider is ever
    // appended to `session_options`, so ORT falls back to its always-present
    // CPU EP only: this is a CPU-only Session by construction. ---
    //
    // SAFETY (every `unsafe { check(api, (api.<fn>)(...)) }` block from here
    // to the end of `run`): `api` is the valid reference established above
    // and live for the rest of this function; each call's arguments are
    // typed to match the `OrtApi` field's C signature (see `ffi.rs`'s
    // per-index comments); every `*mut OrtX` argument is either a
    // `ptr::null_mut()` out-param filled by the call itself, or a `.ptr`
    // field of a guard whose `Drop` has not yet run (so the handle is still
    // live); `.cast_const()` only narrows a live `*mut` to `*const` for a
    // read-only parameter, never changing what it points to.
    let logid = CString::new("npu-router-p0-03-spike").expect("static string has no NUL");
    let mut env_ptr: *mut OrtEnv = ptr::null_mut();
    unsafe {
        check(
            api,
            (api.create_env)(ORT_LOGGING_LEVEL_WARNING, logid.as_ptr(), &mut env_ptr),
        )?;
    }
    let env = EnvGuard { api, ptr: env_ptr };

    let mut so_ptr: *mut OrtSessionOptions = ptr::null_mut();
    unsafe { check(api, (api.create_session_options)(&mut so_ptr))? };
    let session_options = SessionOptionsGuard { api, ptr: so_ptr };

    let model_wide = to_wide_null(args.model_path.as_os_str());
    let mut session_ptr: *mut OrtSession = ptr::null_mut();
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

    let mut input_count = 0usize;
    unsafe {
        check(
            api,
            (api.session_get_input_count)(session.ptr.cast_const(), &mut input_count),
        )?;
    }
    let mut output_count = 0usize;
    unsafe {
        check(
            api,
            (api.session_get_output_count)(session.ptr.cast_const(), &mut output_count),
        )?;
    }
    if input_count != 1 || output_count != 1 {
        return Err(SpikeError::Protobuf(format!(
            "unexpected model IO shape: inputs={input_count} outputs={output_count} \
             (this spike only supports single-input/single-output models)"
        )));
    }

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

    // --- Fixture decode (input_0.pb / golden_output_0.pb). ---
    let input_bytes = read_file(&args.input_path)?;
    let input_tensor = parse_tensor_proto(&input_bytes)?;
    if input_tensor.data_type != ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT {
        return Err(SpikeError::Protobuf(format!(
            "unsupported input data_type={}",
            input_tensor.data_type
        )));
    }
    let mut input_floats = f32_vec_from_le_bytes(&input_tensor.raw_data)?;
    let expected_elems: i64 = input_tensor.dims.iter().product();
    if expected_elems < 0 || input_floats.len() as i64 != expected_elems {
        return Err(SpikeError::Protobuf(format!(
            "input element count {} does not match dims product {}",
            input_floats.len(),
            expected_elems
        )));
    }

    let golden_bytes = read_file(&args.golden_output_path)?;
    let golden_tensor = parse_tensor_proto(&golden_bytes)?;
    let golden_floats = f32_vec_from_le_bytes(&golden_tensor.raw_data)?;

    // --- Input OrtValue: a view over `input_floats`, owned by this stack
    // frame. Per the ORT C API contract, this OrtValue borrows the buffer;
    // it must stay alive at least until Run() returns, which it does here
    // since `input_floats` outlives `input_value`. ---
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

    let byte_len = input_floats.len() * std::mem::size_of::<f32>();
    let mut input_value_ptr: *mut OrtValue = ptr::null_mut();
    // SAFETY (buffer lifetime, on top of the section-level note above):
    // `CreateTensorWithDataAsOrtValue` makes `input_value` a *view* over
    // `input_floats` -- ORT does not copy it. `input_floats` is a local
    // `Vec<f32>` that is not touched or dropped again until after `Run`
    // consumes `input_value` below, so the buffer stays valid and aligned
    // for the tensor's entire lifetime.
    unsafe {
        check(
            api,
            (api.create_tensor_with_data_as_ortvalue)(
                mem_info.ptr.cast_const(),
                input_floats.as_mut_ptr().cast(),
                byte_len,
                input_tensor.dims.as_ptr(),
                input_tensor.dims.len(),
                ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT,
                &mut input_value_ptr,
            ),
        )?;
    }
    let input_value = ValueGuard {
        api,
        ptr: input_value_ptr,
    };

    let mut is_tensor_flag = 0i32;
    unsafe {
        check(
            api,
            (api.is_tensor)(input_value.ptr.cast_const(), &mut is_tensor_flag),
        )?;
    }
    if is_tensor_flag == 0 {
        return Err(SpikeError::Protobuf(
            "CreateTensorWithDataAsOrtValue did not produce a tensor".into(),
        ));
    }

    // --- Run. ---
    let input_name_c = CString::new(input_name.clone())
        .map_err(|_| SpikeError::Protobuf("input name had interior NUL".into()))?;
    let output_name_c = CString::new(output_name.clone())
        .map_err(|_| SpikeError::Protobuf("output name had interior NUL".into()))?;
    let input_names = [input_name_c.as_ptr()];
    let output_names = [output_name_c.as_ptr()];
    let input_values: [*const OrtValue; 1] = [input_value.ptr];
    let mut output_value_ptr: *mut OrtValue = ptr::null_mut();

    let started = Instant::now();
    // SAFETY: `input_names`/`output_names`/`input_values` are stack arrays
    // of pointers into `input_name_c`/`output_name_c`/`input_value`, all of
    // which outlive this call; `session.ptr` is the live session (non-const
    // per `Run`'s own C signature, matching the struct field's param type).
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
    let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    let output_value = ValueGuard {
        api,
        ptr: output_value_ptr,
    };

    // --- Read back, sized from the OrtValue's own shape (never trusts the
    // golden fixture's length for the unsafe slice bound). ---
    let mut shape_info_ptr: *mut OrtTensorTypeAndShapeInfo = ptr::null_mut();
    unsafe {
        check(
            api,
            (api.get_tensor_type_and_shape)(output_value.ptr.cast_const(), &mut shape_info_ptr),
        )?;
    }
    let shape_info = ShapeInfoGuard {
        api,
        ptr: shape_info_ptr,
    };
    let mut elem_count: usize = 0;
    unsafe {
        check(
            api,
            (api.get_tensor_shape_element_count)(shape_info.ptr.cast_const(), &mut elem_count),
        )?;
    }

    let mut out_data_ptr: *mut c_void = ptr::null_mut();
    unsafe {
        check(
            api,
            (api.get_tensor_mutable_data)(output_value.ptr, &mut out_data_ptr),
        )?;
    }
    if out_data_ptr.is_null() {
        return Err(SpikeError::Protobuf(
            "GetTensorMutableData returned null".into(),
        ));
    }
    if elem_count != golden_floats.len() {
        return Err(SpikeError::Protobuf(format!(
            "model output element count {elem_count} does not match golden fixture element count {}",
            golden_floats.len()
        )));
    }
    // SAFETY: `out_data_ptr` is non-null (checked above) and, per the ORT
    // contract, points to a `float`-typed buffer owned by `output_value`
    // (checked non-null via `check` before this) with at least `elem_count`
    // elements -- `elem_count` was just read from that same OrtValue's own
    // `GetTensorShapeElementCount`, not from the golden fixture, so the
    // slice bound cannot exceed the buffer ORT actually allocated. The
    // slice does not outlive `output_value` (dropped at the end of `run`).
    let output_floats: &[f32] =
        unsafe { std::slice::from_raw_parts(out_data_ptr.cast::<f32>(), elem_count) };

    let mut max_abs_diff = 0f32;
    let mut within_tolerance = true;
    for (actual, golden) in output_floats.iter().zip(golden_floats.iter()) {
        let diff = (actual - golden).abs();
        if diff > max_abs_diff {
            max_abs_diff = diff;
        }
        if diff > ATOL + RTOL * golden.abs() {
            within_tolerance = false;
        }
    }

    // --- Negative-path check: a nonexistent model path must yield a
    // classified OrtStatus error, not a crash (AC: no process abort). ---
    let mut bogus_path = args.model_path.clone();
    bogus_path.set_file_name("__p0-03-spike-nonexistent-model__.onnx");
    let bogus_wide = to_wide_null(bogus_path.as_os_str());
    let mut bogus_session_ptr: *mut OrtSession = ptr::null_mut();
    // SAFETY: same reasoning as the earlier `create_session` call; `env` and
    // `session_options` are still live guards at this point in `run`.
    let bogus_status = unsafe {
        (api.create_session)(
            env.ptr.cast_const(),
            bogus_wide.as_ptr(),
            session_options.ptr.cast_const(),
            &mut bogus_session_ptr,
        )
    };
    let (negative_check_code, negative_check_message) = match unsafe { check(api, bogus_status) } {
        Ok(()) => {
            // SAFETY: defensive only -- ORT is not expected to return a
            // non-null session alongside a null status here, but if it
            // ever did, `bogus_session_ptr` would otherwise leak since no
            // guard owns it; release it exactly once before erroring out.
            if !bogus_session_ptr.is_null() {
                unsafe { (api.release_session)(bogus_session_ptr) };
            }
            return Err(SpikeError::Protobuf(
                "negative-path check unexpectedly succeeded: nonexistent model path did not error"
                    .into(),
            ));
        }
        Err(SpikeError::Ort { code, message }) => (code, message),
        Err(other) => return Err(other),
    };

    Ok(Report {
        ort_version,
        input_name,
        output_name,
        input_dims: input_tensor.dims,
        latency_ms,
        max_abs_diff,
        within_tolerance,
        negative_check_code,
        negative_check_message,
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!(
                "usage: p0-03-ort-cpu-session-spike <onnxruntime.dll> <model.onnx> <input_0.pb> <golden_output_0.pb>"
            );
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(report) => {
            println!("{{");
            println!(
                "  \"ort_version\": \"{}\",",
                json_escape(&report.ort_version)
            );
            println!("  \"ort_api_version_used\": {ORT_API_VERSION},");
            println!("  \"provider\": \"cpu\",");
            println!("  \"input_name\": \"{}\",", json_escape(&report.input_name));
            println!(
                "  \"output_name\": \"{}\",",
                json_escape(&report.output_name)
            );
            println!("  \"input_dims\": {:?},", report.input_dims);
            println!("  \"latency_ms\": {:.4},", report.latency_ms);
            println!("  \"max_abs_diff\": {},", report.max_abs_diff);
            println!("  \"within_tolerance\": {},", report.within_tolerance);
            println!(
                "  \"negative_check_ort_error_code\": {},",
                report.negative_check_code
            );
            println!(
                "  \"negative_check_ort_error_message\": \"{}\"",
                json_escape(&report.negative_check_message)
            );
            println!("}}");
            if report.within_tolerance {
                ExitCode::SUCCESS
            } else {
                eprintln!(
                    "golden mismatch: max_abs_diff={} exceeds atol+rtol*|golden|",
                    report.max_abs_diff
                );
                ExitCode::from(1)
            }
        }
        Err(e) => {
            eprintln!("spike failed, classified error: {e}");
            ExitCode::from(1)
        }
    }
}
