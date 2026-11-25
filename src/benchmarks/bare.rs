//! The calls each gate measures weivel against, made past the layer: the core's own function with the core the
//! wrapper calls, and the echo library's functions reached through the platform's loader and called directly.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;

use crate::core::binary::SharedLibrary;
use crate::core::{Core, Handle};

type GetService = unsafe extern "C" fn(*mut c_void, u64, *const c_char, *mut u64, *mut u32) -> i32;
type EchoBytes = unsafe extern "C" fn(*const u8, usize, *mut *mut u8, *mut usize) -> i32;
type EchoRelease = unsafe extern "C" fn(*mut u8);
type AddressSymbol = unsafe extern "C" fn() -> *const c_void;

/// `weivel_get_service` of the core's table, called with the very core the wrapper calls, nothing around it.
pub(crate) struct BareGetService {
    function: GetService,
    core: *mut c_void,
    plugin: u64,
    id: CString,
}

impl BareGetService {
    /// Answers `None` once the core is destroyed or for an id holding a NUL.
    pub(crate) fn new(core: &Core, plugin: Handle, id: &str) -> Option<BareGetService> {
        Some(BareGetService {
            function: core.functions().weivel_get_service,
            core: core.bare_pointer()?,
            plugin: plugin.0,
            id: CString::new(id).ok()?,
        })
    }

    /// Calls it, answering the handle the core answered, or the status and the events left waiting when it answered
    /// anything but OK with nothing queued.
    pub(crate) fn call(&mut self) -> Result<u64, (i32, u32)> {
        let mut handle = 0u64;
        let mut waiting = 0u32;
        // SAFETY: the core lives while the benchmark holds its host, the id lives in `self`, and the out pointers are
        // this call's own; the core serialises calls on its own lock.
        let status = unsafe {
            (self.function)(
                self.core,
                self.plugin,
                self.id.as_ptr(),
                &mut handle,
                &mut waiting,
            )
        };
        if status == 0 && waiting == 0 {
            Ok(handle)
        } else {
            Err((status, waiting))
        }
    }
}

fn resolved(library: &SharedLibrary, symbol: &str) -> Option<*mut c_void> {
    library.symbol(&CString::new(symbol).ok()?)
}

/// The echo library opened through the platform's loader, as a caller without weivel opens it.
pub(crate) struct BareEcho {
    library: SharedLibrary,
    echo: EchoBytes,
    release: EchoRelease,
}

impl BareEcho {
    /// Opens the echo library at `path`, answering `None` when it is no library or lacks `echo_echo` or
    /// `echo_release`.
    pub(crate) fn open(path: &Path) -> Option<BareEcho> {
        let library = SharedLibrary::open(path)?;
        let echo = resolved(&library, "echo_echo")?;
        let release = resolved(&library, "echo_release")?;
        // SAFETY: `echo.h` declares both symbols with exactly these signatures.
        let (echo, release) = unsafe {
            (
                std::mem::transmute::<*mut c_void, EchoBytes>(echo),
                std::mem::transmute::<*mut c_void, EchoRelease>(release),
            )
        };
        Some(BareEcho {
            library,
            echo,
            release,
        })
    }

    /// Calls `echo_echo` with `input` and releases its answer through `echo_release`, answering the status and how
    /// many bytes it answered.
    pub(crate) fn echo(&self, input: &[u8]) -> (i32, usize) {
        let mut output: *mut u8 = std::ptr::null_mut();
        let mut length = 0usize;
        // SAFETY: the input lives across the call, and the out pointers are this call's own.
        let status = unsafe { (self.echo)(input.as_ptr(), input.len(), &mut output, &mut length) };
        if !output.is_null() {
            // SAFETY: the body `echo_echo` answered, released once, here.
            unsafe { (self.release)(output) };
        }
        (status, length)
    }

    /// The address the definition symbol `symbol` answers, `None` for a symbol the library lacks.
    pub(crate) fn answered_address(&self, symbol: &str) -> Option<usize> {
        let address = resolved(&self.library, symbol)?;
        // SAFETY: a definition symbol is `const void *f(void)`, as `library.h` declares it.
        let answered = unsafe { std::mem::transmute::<*mut c_void, AddressSymbol>(address)() };
        Some(answered as usize)
    }
}

/// `echo_definition`'s head, declared by hand from `echo.h`: its `size`, its context and the two operations a call
/// makes.
#[repr(C)]
struct EchoDefinitionHead {
    size: u32,
    context: *mut c_void,
    echo: unsafe extern "C" fn(*mut c_void, *const u8, usize, *mut *mut u8, *mut usize) -> i32,
    release: unsafe extern "C" fn(*mut c_void, *mut u8),
}

/// The echo definition's `echo` and `release`, read out of its struct once, before any timed call, from a binary
/// its [`BareEcho`] keeps open.
pub(crate) struct DefinitionEcho<'a> {
    context: *mut c_void,
    echo: unsafe extern "C" fn(*mut c_void, *const u8, usize, *mut *mut u8, *mut usize) -> i32,
    release: unsafe extern "C" fn(*mut c_void, *mut u8),
    _open: &'a BareEcho,
}

impl BareEcho {
    /// Reads the operations of the definition at `address`, which must be the very address this load's
    /// `echo_service_definition` answers: `None` for any other, or for a struct too small to carry them.
    pub(crate) fn definition_at(&self, address: usize) -> Option<DefinitionEcho<'_>> {
        if self.answered_address("echo_service_definition")? != address {
            return None;
        }
        // SAFETY: the address is what the symbol answered, from a binary `self` keeps open; the struct begins with
        // its `size`, read before anything past it.
        let size = unsafe { std::ptr::read(address as *const u32) } as usize;
        if size < size_of::<EchoDefinitionHead>() {
            return None;
        }
        // SAFETY: the struct is at least the head long and lives while the binary is open.
        let head = unsafe { &*(address as *const EchoDefinitionHead) };
        Some(DefinitionEcho {
            context: head.context,
            echo: head.echo,
            release: head.release,
            _open: self,
        })
    }
}

impl DefinitionEcho<'_> {
    /// Calls `echo` with `input` and releases its answer through the definition's `release`, answering the status
    /// and how many bytes it answered.
    pub(crate) fn echo(&self, input: &[u8]) -> (i32, usize) {
        let mut output: *mut u8 = std::ptr::null_mut();
        let mut length = 0usize;
        // SAFETY: the binary is open while the `BareEcho` this borrows lives, the input lives across the call, and the
        // out pointers are this call's own.
        let status = unsafe {
            (self.echo)(
                self.context,
                input.as_ptr(),
                input.len(),
                &mut output,
                &mut length,
            )
        };
        if !output.is_null() {
            // SAFETY: the body the definition answered, released once, through the definition.
            unsafe { (self.release)(self.context, output) };
        }
        (status, length)
    }
}

/// The text of a NUL-terminated C string the library answered, `None` for NULL or text that is not UTF-8.
///
/// # Safety
/// `text` is NULL or points at a NUL-terminated string that lives across the call.
pub(crate) unsafe fn text_at(text: *const c_char) -> Option<String> {
    if text.is_null() {
        return None;
    }
    // SAFETY: as the caller promised.
    let text = unsafe { CStr::from_ptr(text) };
    text.to_str().ok().map(str::to_owned)
}
