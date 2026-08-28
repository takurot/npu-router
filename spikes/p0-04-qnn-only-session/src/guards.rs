//! RAII guards recording ownership/lifetime for each native handle this
//! spike creates.
//!
//! SAFETY (all `Drop` impls below): each guard is constructed exactly once,
//! only after `check()` (see `error.rs`) has confirmed the corresponding
//! `Create*`/`Load*` call succeeded and returned a non-null handle (see
//! `run()` in `main.rs`), and nothing else ever calls the matching
//! `Release*`/`FreeLibrary` function for that handle. So each `Drop::drop`
//! releases a live, uniquely-owned, non-null handle exactly once.

use std::ffi::c_void;

use crate::ffi::{
    FreeLibrary, OrtApi, OrtEnv, OrtMemoryInfo, OrtSession, OrtSessionOptions, OrtValue,
};

pub struct LibraryGuard(pub *mut c_void);
impl Drop for LibraryGuard {
    fn drop(&mut self) {
        unsafe {
            FreeLibrary(self.0);
        }
    }
}

pub struct EnvGuard<'a> {
    pub api: &'a OrtApi,
    pub ptr: *mut OrtEnv,
}
impl Drop for EnvGuard<'_> {
    fn drop(&mut self) {
        unsafe { (self.api.release_env)(self.ptr) }
    }
}

pub struct SessionOptionsGuard<'a> {
    pub api: &'a OrtApi,
    pub ptr: *mut OrtSessionOptions,
}
impl Drop for SessionOptionsGuard<'_> {
    fn drop(&mut self) {
        unsafe { (self.api.release_session_options)(self.ptr) }
    }
}

pub struct SessionGuard<'a> {
    pub api: &'a OrtApi,
    pub ptr: *mut OrtSession,
}
impl Drop for SessionGuard<'_> {
    fn drop(&mut self) {
        unsafe { (self.api.release_session)(self.ptr) }
    }
}

pub struct MemoryInfoGuard<'a> {
    pub api: &'a OrtApi,
    pub ptr: *mut OrtMemoryInfo,
}
impl Drop for MemoryInfoGuard<'_> {
    fn drop(&mut self) {
        unsafe { (self.api.release_memory_info)(self.ptr) }
    }
}

pub struct ValueGuard<'a> {
    pub api: &'a OrtApi,
    pub ptr: *mut OrtValue,
}
impl Drop for ValueGuard<'_> {
    fn drop(&mut self) {
        unsafe { (self.api.release_value)(self.ptr) }
    }
}
