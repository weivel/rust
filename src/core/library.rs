//! A C library a host loads as host services: its binary opened by path, each declared symbol resolved by name, a
//! bytes function called, a definition and a description answered once at the load.

use std::ffi::{CString, c_void};
use std::fmt;
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;

use super::answer::Answer;
use super::binary::SharedLibrary;

type BytesSymbol = unsafe extern "C" fn(*const u8, usize, *mut *mut u8, *mut usize) -> i32;
type ReleaseSymbol = unsafe extern "C" fn(*mut u8);
type AddressSymbol = unsafe extern "C" fn() -> *const c_void;

/// The smallest `size` a definition's struct may have: its `size` and one pointer under the platform's alignment.
pub(crate) const DEFINITION_FLOOR: usize = 2 * size_of::<*const c_void>();

/// What runs when the platform refuses to close a library's binary.
pub(crate) type RefusedClose = Box<dyn Fn() + Send + Sync>;

/// An open library binary. Every holder of it holds an `Arc`, so the binary closes when the last one is dropped.
pub(crate) struct LibraryBinary {
    library: Option<SharedLibrary>,
    release: Option<ReleaseSymbol>,
    refused_close: RefusedClose,
}

/// A bytes function, resolved from a binary its caller keeps open across each call.
#[derive(Clone, Copy)]
pub(crate) struct BytesFunction(BytesSymbol);

/// What a bytes function answered: its status and its body, the library's own bytes.
pub(crate) struct BytesCall {
    pub(crate) status: i32,
    pub(crate) body: LibraryBytes,
}

/// Bytes a C library's function answered, read in place and released through the library's release symbol when
/// dropped. They keep the library's binary open until then, so they are never read after their release.
pub struct LibraryBytes {
    binary: Arc<LibraryBinary>,
    pointer: *mut u8,
    length: usize,
}

// SAFETY: the bytes are the library's, written before the call answered and never changed after; reading them from any
// thread is reading immutable memory, and the release symbol frees memory any thread may free.
unsafe impl Send for LibraryBytes {}
// SAFETY: as above; a shared reference only reads them.
unsafe impl Sync for LibraryBytes {}

/// A definition and the description its service declares for it, both addresses a library answered at the load. A
/// check reads the description through them while the library is open; neither is valid once its binary closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescribedDefinition {
    description: usize,
    definition: usize,
}

impl DescribedDefinition {
    /// The address of the `weivel_standard_description` the description symbol answered.
    pub fn description_address(&self) -> usize {
        self.description
    }

    /// The address the definition symbol answered.
    pub fn definition_address(&self) -> usize {
        self.definition
    }
}

fn symbol_name(symbol: &str) -> Result<CString, Answer> {
    CString::new(symbol).map_err(|_| {
        Answer::Mistake(format!(
            "the symbol name {symbol:?} holds a NUL character, so it cannot cross to the platform's loader"
        ))
    })
}

impl LibraryBinary {
    /// Opens the binary at `path` and resolves its release symbol when the declaration names one.
    pub(crate) fn open(
        path: &Path,
        release: Option<&str>,
        refused_close: RefusedClose,
    ) -> Result<Arc<LibraryBinary>, Answer> {
        let library = SharedLibrary::open(path).ok_or_else(|| {
            Answer::Mistake(format!(
                "{} names no library this process can load",
                path.display()
            ))
        })?;
        let mut binary = LibraryBinary {
            library: Some(library),
            release: None,
            refused_close,
        };
        if let Some(release) = release {
            let address = binary.address(release)?;
            // SAFETY: `library.h` declares a release symbol as `void release(uint8_t *buffer)`.
            binary.release =
                Some(unsafe { std::mem::transmute::<*mut c_void, ReleaseSymbol>(address) });
        }
        Ok(Arc::new(binary))
    }

    fn address(&self, symbol: &str) -> Result<*mut c_void, Answer> {
        let name = symbol_name(symbol)?;
        self.library
            .as_ref()
            .and_then(|library| library.symbol(&name))
            .ok_or_else(|| {
                Answer::Mistake(format!("the library lacks its declared symbol {symbol}"))
            })
    }

    /// Resolves a declared bytes function.
    pub(crate) fn bytes_function(&self, symbol: &str) -> Result<BytesFunction, Answer> {
        let address = self.address(symbol)?;
        // SAFETY: `library.h` declares a bytes function with exactly this signature.
        Ok(BytesFunction(unsafe {
            std::mem::transmute::<*mut c_void, BytesSymbol>(address)
        }))
    }

    fn answered_address(&self, symbol: &str) -> Result<usize, Answer> {
        let address = self.address(symbol)?;
        // SAFETY: `library.h` declares a definition, and `description.h` a description, as `const void *f(void)`.
        let answer = unsafe { std::mem::transmute::<*mut c_void, AddressSymbol>(address)() };
        Ok(answer as usize)
    }

    /// Calls a definition's symbol once and answers the address of its struct, refusing NULL and a struct whose
    /// `size` lies below the floor as the caller's mistake.
    pub(crate) fn definition(&self, symbol: &str) -> Result<usize, Answer> {
        let address = self.answered_address(symbol)?;
        if address == 0 {
            return Err(Answer::Mistake(format!(
                "the definition {symbol} answered NULL"
            )));
        }
        // SAFETY: a definition answers a struct that begins with its `uint32_t size`, which lives while the binary is
        // open.
        let size = unsafe { std::ptr::read(address as *const u32) } as usize;
        if size < DEFINITION_FLOOR {
            return Err(Answer::Mistake(format!(
                "the definition {symbol} answered a struct of size {size}, below the {DEFINITION_FLOOR} bytes of its \
                 size and one pointer"
            )));
        }
        Ok(address)
    }

    /// Calls a description's symbol once and pairs what it answered with the definition it describes.
    pub(crate) fn described(
        &self,
        symbol: &str,
        definition: usize,
    ) -> Result<DescribedDefinition, Answer> {
        let description = self.answered_address(symbol)?;
        if description == 0 {
            return Err(Answer::Mistake(format!(
                "the description {symbol} answered NULL"
            )));
        }
        Ok(DescribedDefinition {
            description,
            definition,
        })
    }

    /// Calls a bytes function with `input` lent for the call, answering its status and its body in place.
    pub(crate) fn call(self: &Arc<Self>, function: BytesFunction, input: &[u8]) -> BytesCall {
        let mut pointer: *mut u8 = std::ptr::null_mut();
        let mut length = 0usize;
        // SAFETY: the input lives across the call, and the out pointers are the crate's own.
        let status =
            unsafe { (function.0)(input.as_ptr(), input.len(), &mut pointer, &mut length) };
        BytesCall {
            status,
            body: LibraryBytes {
                binary: self.clone(),
                pointer,
                length: if pointer.is_null() { 0 } else { length },
            },
        }
    }
}

impl Drop for LibraryBinary {
    fn drop(&mut self) {
        if let Some(library) = self.library.take()
            && !library.close()
        {
            (self.refused_close)();
        }
    }
}

impl LibraryBytes {
    /// The bytes.
    pub fn as_bytes(&self) -> &[u8] {
        if self.pointer.is_null() {
            return &[];
        }
        // SAFETY: the library answered `length` bytes at `pointer`, which stay until this value releases them.
        unsafe { std::slice::from_raw_parts(self.pointer, self.length) }
    }

    /// The bytes as text, where they are well-formed UTF-8.
    pub fn text(&self) -> Option<&str> {
        std::str::from_utf8(self.as_bytes()).ok()
    }
}

impl Deref for LibraryBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl fmt::Debug for LibraryBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.text() {
            Some(text) => write!(formatter, "LibraryBytes({text:?})"),
            None => write!(formatter, "LibraryBytes({:?})", self.as_bytes()),
        }
    }
}

impl Drop for LibraryBytes {
    fn drop(&mut self) {
        if self.pointer.is_null() {
            return;
        }
        if let Some(release) = self.binary.release {
            // SAFETY: the pointer is a body the library answered, released exactly once, here.
            unsafe { release(self.pointer) };
        }
    }
}
