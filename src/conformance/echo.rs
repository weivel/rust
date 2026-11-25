//! The suite's own reading of the core's `echo` test library: its definition's struct laid out by hand from
//! `test/echo/echo.h`, so a `library` vector calls through the very address the host registered, and the address
//! each definition symbol answers, read through a load the suite makes of the same binary.

use std::ffi::{CString, c_void};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use super::shapes::DriverFailure;
use crate::core::binary::SharedLibrary;

/// `echo_definition`, declared by hand from `echo.h`: its `size`, then its context and three operations.
#[repr(C)]
struct EchoDefinition {
    size: u32,
    context: *mut c_void,
    echo: unsafe extern "C" fn(*mut c_void, *const u8, usize, *mut *mut u8, *mut usize) -> i32,
    release: unsafe extern "C" fn(*mut c_void, *mut u8),
    count_unloads: unsafe extern "C" fn(*mut c_void, *mut u64),
}

/// Reads the definition at `address` as an `echo_definition`, refusing one too small to carry every operation.
///
/// # Safety
/// `address` is what an `echo` definition symbol answered, from a binary still open.
unsafe fn definition_at<'a>(address: usize) -> Result<&'a EchoDefinition, DriverFailure> {
    // SAFETY: as the caller promised; the struct begins with its `size`, read before anything past it.
    let size = unsafe { std::ptr::read(address as *const u32) } as usize;
    if size < size_of::<EchoDefinition>() {
        return Err(format!(
            "the definition at {address:#x} is {size} bytes, too small for an echo_definition"
        ));
    }
    // SAFETY: the struct is at least an `echo_definition` long and lives while the binary is open.
    Ok(unsafe { &*(address as *const EchoDefinition) })
}

/// Echoes `text` through the definition at `address`, releasing its answer through the definition.
pub(crate) fn echo_through(address: usize, text: &str) -> Result<String, DriverFailure> {
    // SAFETY: the driver hands only the address a holder resolved, while its holding keeps the binary open.
    let definition = unsafe { definition_at(address)? };
    let mut output: *mut u8 = std::ptr::null_mut();
    let mut length = 0usize;
    // SAFETY: the input lives across the call and the out pointers are the driver's own.
    let status = unsafe {
        (definition.echo)(
            definition.context,
            text.as_ptr(),
            text.len(),
            &mut output,
            &mut length,
        )
    };
    let answered = if output.is_null() {
        Vec::new()
    } else {
        // SAFETY: the library answered `length` bytes at `output`, valid until their release below.
        unsafe { std::slice::from_raw_parts(output, length) }.to_vec()
    };
    // SAFETY: the body the definition answered is released once, through the definition.
    unsafe { (definition.release)(definition.context, output) };
    if status != 0 {
        return Err(format!("the definition's echo answered status {status}"));
    }
    String::from_utf8(answered).map_err(|_| "the definition's echo answered no UTF-8".to_owned())
}

/// Hands the library a counter it adds one to when its binary is closed, which the caller keeps until the host that
/// loaded the binary is destroyed, the latest the binary closes.
pub(crate) fn count_unloads(address: usize, counter: &Arc<AtomicU64>) -> Result<(), DriverFailure> {
    // SAFETY: as for `echo_through`.
    let definition = unsafe { definition_at(address)? };
    // SAFETY: the counter is a `u64` its caller keeps past the binary's close, as `echo.h` asks.
    unsafe { (definition.count_unloads)(definition.context, counter.as_ptr()) };
    Ok(())
}

/// The address each of `symbols` answers, read through a load of the binary the suite makes and closes again while
/// the host's load holds it, so the suite reads the very mapping the host loaded.
pub(crate) fn answered_addresses(
    binary: &Path,
    symbols: &[String],
) -> Result<Vec<usize>, DriverFailure> {
    let library = SharedLibrary::open(binary)
        .ok_or_else(|| format!("the suite could not open {}", binary.display()))?;
    symbols
        .iter()
        .map(|symbol| {
            let name = CString::new(symbol.as_str())
                .map_err(|_| format!("the symbol {symbol:?} holds a NUL"))?;
            let address = library
                .symbol(&name)
                .ok_or_else(|| format!("{} lacks {symbol}", binary.display()))?;
            // SAFETY: an echo definition symbol is `const void *f(void)`.
            let answer = unsafe {
                std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> *const c_void>(address)()
            };
            Ok(answer as usize)
        })
        .collect()
}
