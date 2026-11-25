//! A shared library opened by path with the platform's own loader, declared by hand so the crate links nothing for it.

use std::ffi::{CStr, c_void};
use std::path::Path;

#[cfg(unix)]
#[path = "binary_posix.rs"]
mod platform;
#[cfg(windows)]
#[path = "binary_windows.rs"]
mod platform;

/// An open shared library, closed when dropped or by [`SharedLibrary::close`].
pub(crate) struct SharedLibrary {
    handle: *mut c_void,
}

// SAFETY: a library handle names process-wide state that the platform's loader guards itself.
unsafe impl Send for SharedLibrary {}
// SAFETY: as above; looking up a symbol through a shared handle is thread-safe on every supported loader.
unsafe impl Sync for SharedLibrary {}

impl SharedLibrary {
    /// Opens the library at `path`, or answers `None` when it is no library this process can load.
    pub(crate) fn open(path: &Path) -> Option<SharedLibrary> {
        if path.as_os_str().is_empty() {
            return None;
        }
        let handle = platform::open(path)?;
        Some(SharedLibrary { handle })
    }

    /// The address of the exported symbol `name`, or `None` when the library exports none of that name.
    pub(crate) fn symbol(&self, name: &CStr) -> Option<*mut c_void> {
        let address = platform::find(self.handle, name);
        (!address.is_null()).then_some(address)
    }

    /// Closes the library now, answering whether the platform's loader closed it.
    pub(crate) fn close(mut self) -> bool {
        let handle = std::mem::replace(&mut self.handle, std::ptr::null_mut());
        platform::close(handle)
    }
}

impl Drop for SharedLibrary {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            platform::close(self.handle);
        }
    }
}
