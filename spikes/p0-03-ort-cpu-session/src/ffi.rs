//! kernel32 + ONNX Runtime C API FFI surface (subset, `ORT_API_VERSION` 23 /
//! onnxruntime v1.23.2). See `../README.md` "OrtApi field derivation" for
//! how the `OrtApi` field order was derived and verified, and the crate
//! root's "Safety invariants" doc for how every call site built on top of
//! this module stays sound.

use std::ffi::{c_char, c_void};

// ---------------------------------------------------------------------
// kernel32: explicit DLL load with a fixed, restricted search order.
// Only the directory containing the DLL and System32 are consulted; the
// process CWD and PATH are never used. Matches docs/SPEC.md 18.2.
// ---------------------------------------------------------------------

#[link(name = "kernel32")]
unsafe extern "system" {
    pub fn LoadLibraryExW(file_name: *const u16, reserved: *mut c_void, flags: u32) -> *mut c_void;
    pub fn GetProcAddress(module: *mut c_void, proc_name: *const u8) -> *mut c_void;
    pub fn FreeLibrary(module: *mut c_void) -> i32;
    pub fn GetLastError() -> u32;
}

pub const LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR: u32 = 0x0000_0100;
pub const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0000_0800;

// ---------------------------------------------------------------------
// ONNX Runtime C API opaque handle types + scalar constants.
// ---------------------------------------------------------------------

pub enum OrtStatus {}
pub enum OrtEnv {}
pub enum OrtSession {}
pub enum OrtSessionOptions {}
pub enum OrtValue {}
pub enum OrtMemoryInfo {}
pub enum OrtAllocator {}
pub enum OrtTensorTypeAndShapeInfo {}

pub const ORT_API_VERSION: u32 = 23;
pub const ORT_LOGGING_LEVEL_WARNING: i32 = 2;
pub const ORT_ARENA_ALLOCATOR: i32 = 1;
pub const ORT_MEM_TYPE_DEFAULT: i32 = 0;
pub const ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT: i32 = 1;

#[repr(C)]
pub struct OrtApiBase {
    pub get_api: unsafe extern "system" fn(u32) -> *const OrtApi,
    pub get_version_string: unsafe extern "system" fn() -> *const c_char,
}

/// Field order mirrors `onnxruntime_c_api.h`'s `struct OrtApi` exactly
/// (verified by extracting all ~382 member names from the vendored header
/// in declaration order). Fields this spike never calls are left as
/// `*const c_void` padding of the same count, so the struct is a valid,
/// layout-compatible prefix of the real vtable up through
/// `ReleaseSessionOptions` (index 100).
#[repr(C)]
pub struct OrtApi {
    pub create_status: unsafe extern "system" fn(i32, *const c_char) -> *mut OrtStatus, // 0
    pub get_error_code: unsafe extern "system" fn(*const OrtStatus) -> i32,             // 1
    pub get_error_message: unsafe extern "system" fn(*const OrtStatus) -> *const c_char, // 2
    pub create_env:
        unsafe extern "system" fn(i32, *const c_char, *mut *mut OrtEnv) -> *mut OrtStatus, // 3
    _pad_4_6: [*const c_void; 3], // CreateEnvWithCustomLogger, {Enable,Disable}TelemetryEvents
    pub create_session: unsafe extern "system" fn(
        *const OrtEnv,
        *const u16,
        *const OrtSessionOptions,
        *mut *mut OrtSession,
    ) -> *mut OrtStatus, // 7
    _pad_8: *const c_void,        // CreateSessionFromArray
    pub run: unsafe extern "system" fn(
        *mut OrtSession,
        *const c_void,
        *const *const c_char,
        *const *const OrtValue,
        usize,
        *const *const c_char,
        usize,
        *mut *mut OrtValue,
    ) -> *mut OrtStatus, // 9
    pub create_session_options:
        unsafe extern "system" fn(*mut *mut OrtSessionOptions) -> *mut OrtStatus, // 10
    _pad_11_29: [*const c_void; 19], // SetOptimizedModelFilePath .. RegisterCustomOpsLibrary
    pub session_get_input_count:
        unsafe extern "system" fn(*const OrtSession, *mut usize) -> *mut OrtStatus, // 30
    pub session_get_output_count:
        unsafe extern "system" fn(*const OrtSession, *mut usize) -> *mut OrtStatus, // 31
    _pad_32_35: [*const c_void; 4], // SessionGetOverridableInitializerCount .. SessionGetOverridableInitializerTypeInfo
    pub session_get_input_name: unsafe extern "system" fn(
        *const OrtSession,
        usize,
        *mut OrtAllocator,
        *mut *mut c_char,
    ) -> *mut OrtStatus, // 36
    pub session_get_output_name: unsafe extern "system" fn(
        *const OrtSession,
        usize,
        *mut OrtAllocator,
        *mut *mut c_char,
    ) -> *mut OrtStatus, // 37
    _pad_38_48: [*const c_void; 11], // SessionGetOverridableInitializerName .. CreateTensorAsOrtValue
    pub create_tensor_with_data_as_ortvalue: unsafe extern "system" fn(
        *const OrtMemoryInfo,
        *mut c_void,
        usize,
        *const i64,
        usize,
        i32,
        *mut *mut OrtValue,
    ) -> *mut OrtStatus, // 49
    pub is_tensor: unsafe extern "system" fn(*const OrtValue, *mut i32) -> *mut OrtStatus, // 50
    pub get_tensor_mutable_data:
        unsafe extern "system" fn(*mut OrtValue, *mut *mut c_void) -> *mut OrtStatus, // 51
    _pad_52_63: [*const c_void; 12], // FillStringTensor .. GetSymbolicDimensions
    pub get_tensor_shape_element_count:
        unsafe extern "system" fn(*const OrtTensorTypeAndShapeInfo, *mut usize) -> *mut OrtStatus, // 64
    pub get_tensor_type_and_shape: unsafe extern "system" fn(
        *const OrtValue,
        *mut *mut OrtTensorTypeAndShapeInfo,
    ) -> *mut OrtStatus, // 65
    _pad_66_68: [*const c_void; 3], // GetTypeInfo, GetValueType, CreateMemoryInfo
    pub create_cpu_memory_info:
        unsafe extern "system" fn(i32, i32, *mut *mut OrtMemoryInfo) -> *mut OrtStatus, // 69
    _pad_70_75: [*const c_void; 6], // CompareMemoryInfo .. AllocatorAlloc
    pub allocator_free: unsafe extern "system" fn(*mut OrtAllocator, *mut c_void) -> *mut OrtStatus, // 76
    _pad_77: *const c_void, // AllocatorGetInfo
    pub get_allocator_with_default_options:
        unsafe extern "system" fn(*mut *mut OrtAllocator) -> *mut OrtStatus, // 78
    _pad_79_91: [*const c_void; 13], // AddFreeDimensionOverride .. KernelContext_GetOutput
    pub release_env: unsafe extern "system" fn(*mut OrtEnv), // 92
    pub release_status: unsafe extern "system" fn(*mut OrtStatus), // 93
    pub release_memory_info: unsafe extern "system" fn(*mut OrtMemoryInfo), // 94
    pub release_session: unsafe extern "system" fn(*mut OrtSession), // 95
    pub release_value: unsafe extern "system" fn(*mut OrtValue), // 96
    _pad_97_98: [*const c_void; 2], // ReleaseRunOptions, ReleaseTypeInfo
    pub release_tensor_type_and_shape_info:
        unsafe extern "system" fn(*mut OrtTensorTypeAndShapeInfo), // 99
    pub release_session_options: unsafe extern "system" fn(*mut OrtSessionOptions), // 100
}
