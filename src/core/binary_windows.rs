//! The library calls on `LoadLibraryW`, `GetProcAddress` and `FreeLibrary`.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

const SEM_FAILCRITICALERRORS: u32 = 0x0001;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(file_name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    fn FreeLibrary(module: *mut c_void) -> c_int;
    fn GetThreadErrorMode() -> u32;
    fn SetThreadErrorMode(new_mode: u32, old_mode: *mut u32) -> c_int;
}

fn wide_path(path: &Path) -> Option<Vec<u16>> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return None;
    }
    wide.push(0);
    Some(wide)
}

/// Opens the library with the thread's error mode raised for the call alone, since Windows otherwise answers a
/// damaged image with a dialog.
pub(super) fn open(path: &Path) -> Option<*mut c_void> {
    let wide = wide_path(path)?;
    let mut previous_mode = 0u32;
    // SAFETY: `wide` is NUL-terminated and outlives the call; the error mode is restored to what it was.
    let handle = unsafe {
        SetThreadErrorMode(
            GetThreadErrorMode() | SEM_FAILCRITICALERRORS,
            &mut previous_mode,
        );
        let handle = LoadLibraryW(wide.as_ptr());
        SetThreadErrorMode(previous_mode, std::ptr::null_mut());
        handle
    };
    (!handle.is_null()).then_some(handle)
}

pub(super) fn find(handle: *mut c_void, name: &CStr) -> *mut c_void {
    // SAFETY: `handle` came from `LoadLibraryW` and is still open; `name` is NUL-terminated.
    unsafe { GetProcAddress(handle, name.as_ptr()) }
}

pub(super) fn close(handle: *mut c_void) -> bool {
    // SAFETY: `handle` came from `LoadLibraryW` and is freed exactly once.
    unsafe { FreeLibrary(handle) != 0 }
}
