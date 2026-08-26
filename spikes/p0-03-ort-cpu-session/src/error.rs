//! Error classification (AC: failures must be classified, never abort).

use std::ffi::CStr;
use std::fmt;

use crate::ffi::{OrtApi, OrtStatus};

#[derive(Debug)]
pub enum SpikeError {
    Usage(String),
    Io {
        path: String,
        source: std::io::Error,
    },
    Protobuf(String),
    DllLoad {
        code: u32,
    },
    MissingExport(&'static str),
    UnsupportedApiVersion(u32),
    Ort {
        code: i32,
        message: String,
    },
}

impl fmt::Display for SpikeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpikeError::Usage(m) => write!(f, "usage error: {m}"),
            SpikeError::Io { path, source } => write!(f, "io error on {path}: {source}"),
            SpikeError::Protobuf(m) => write!(f, "protobuf parse error: {m}"),
            SpikeError::DllLoad { code } => {
                write!(f, "LoadLibraryExW failed, GetLastError={code}")
            }
            SpikeError::MissingExport(name) => write!(f, "missing DLL export: {name}"),
            SpikeError::UnsupportedApiVersion(v) => write!(
                f,
                "OrtApiBase::GetApi returned null for ORT_API_VERSION={v}"
            ),
            SpikeError::Ort { code, message } => {
                write!(f, "ORT status code={code} message={message}")
            }
        }
    }
}

/// Converts a (possibly null) `OrtStatus*` into `Result`, releasing it.
/// A null status means success, per the ORT C API contract.
///
/// # Safety
/// `status` must be either null or a live `OrtStatus*` returned by an
/// `OrtApi` call through `api` (never previously released), and `api` must
/// be a valid `OrtApi` reference (see crate root "Safety invariants").
pub unsafe fn check(api: &OrtApi, status: *mut OrtStatus) -> Result<(), SpikeError> {
    if status.is_null() {
        return Ok(());
    }
    // SAFETY: `status` is confirmed non-null above and, per this fn's
    // safety contract, is a live status from `api`; `get_error_code` and
    // `get_error_message` only read it, matching their C signatures.
    let code = unsafe { (api.get_error_code)(status.cast_const()) };
    let msg_ptr = unsafe { (api.get_error_message)(status.cast_const()) };
    let message = if msg_ptr.is_null() {
        String::from("<no message>")
    } else {
        // SAFETY: ORT guarantees `GetErrorMessage` returns a valid
        // null-terminated UTF-8 string owned by `status`, live until
        // `status` is released (which happens after this line, below).
        unsafe { CStr::from_ptr(msg_ptr) }
            .to_string_lossy()
            .into_owned()
    };
    // SAFETY: `status` is released exactly once, here, after both reads
    // above have finished using it; nothing else holds or reuses it.
    unsafe { (api.release_status)(status) };
    Err(SpikeError::Ort { code, message })
}
