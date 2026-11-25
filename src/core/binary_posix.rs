//! The library calls on `dlopen`, `dlsym` and `dlclose`.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

const RTLD_NOW: c_int = 2;
#[cfg(target_os = "macos")]
const RTLD_LOCAL: c_int = 4;
#[cfg(not(target_os = "macos"))]
const RTLD_LOCAL: c_int = 0;

/// glibc before 2.34 keeps the loader in `libdl`; from 2.34 that library is an empty stub, so naming it costs nothing.
#[cfg_attr(all(target_os = "linux", target_env = "gnu"), link(name = "dl"))]
unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
}

pub(super) fn open(path: &Path) -> Option<*mut c_void> {
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: `path` is NUL-terminated and outlives the call.
    let handle = unsafe { dlopen(path.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
    (!handle.is_null()).then_some(handle)
}

pub(super) fn find(handle: *mut c_void, name: &CStr) -> *mut c_void {
    // SAFETY: `handle` came from `dlopen` and is still open; `name` is NUL-terminated.
    unsafe { dlsym(handle, name.as_ptr()) }
}

pub(super) fn close(handle: *mut c_void) -> bool {
    // SAFETY: `handle` came from `dlopen` and is closed exactly once.
    unsafe { dlclose(handle) == 0 }
}
