//! The standard's description checks, `weivel_standard_description_check` and `weivel_standard_description_answers`,
//! resolved by name from the standard's binary and declared by hand, run as the host's [`DescriptionCheck`].

use std::ffi::{CStr, c_char, c_int, c_void};
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use crate::core::binary::SharedLibrary;
use crate::core::{Answer, DescribedDefinition, LoadFailure, RefusalStatus};
use crate::host::DescriptionCheck;

const STATUS_OK: c_int = 0;
const STATUS_OUT_OF_MEMORY: c_int = 4;

/// `weivel_standard_refusal`, declared by hand from `standard.h`.
#[repr(C)]
struct StandardRefusal {
    size: u32,
    code: u32,
    detail: *const c_char,
    fix: *const c_char,
    path: *const c_char,
    storage: *mut c_void,
}

type Check = unsafe extern "C" fn(*const c_void, *mut StandardRefusal) -> c_int;
type Answers = unsafe extern "C" fn(*const c_void, *const c_void, *mut StandardRefusal) -> c_int;
type Release = unsafe extern "C" fn(*mut StandardRefusal);

struct DescriptionFunctions {
    check: Check,
    answers: Answers,
    release: Release,
    _library: SharedLibrary,
}

/// The standard's two description checks, bound from its shared binary: a description is checked whole, then held to
/// the definition its service's symbol answered. Cloning it shares the binding.
#[derive(Clone)]
pub struct StandardDescriptions {
    functions: Arc<DescriptionFunctions>,
}

impl fmt::Debug for StandardDescriptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StandardDescriptions")
    }
}

fn resolve(library: &SharedLibrary, name: &'static CStr) -> Result<*mut c_void, LoadFailure> {
    library.symbol(name).ok_or(LoadFailure::MissingFunction(
        name.to_str().unwrap_or("a description function"),
    ))
}

fn text_at(pointer: *const c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    // SAFETY: a refusal's texts are NUL-terminated and live until its release.
    unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned()
}

impl StandardDescriptions {
    /// Opens the standard's shared binary at `path` and resolves the two checks and the refusal's release.
    pub fn load(path: impl AsRef<Path>) -> Result<StandardDescriptions, LoadFailure> {
        let library = SharedLibrary::open(path.as_ref()).ok_or(LoadFailure::NotALibrary)?;
        let check = resolve(&library, c"weivel_standard_description_check")?;
        let answers = resolve(&library, c"weivel_standard_description_answers")?;
        let release = resolve(&library, c"weivel_standard_refusal_release")?;
        // SAFETY: `description.h` and `standard.h` declare these symbols as functions of exactly these types.
        let functions = unsafe {
            DescriptionFunctions {
                check: std::mem::transmute::<*mut c_void, Check>(check),
                answers: std::mem::transmute::<*mut c_void, Answers>(answers),
                release: std::mem::transmute::<*mut c_void, Release>(release),
                _library: library,
            }
        };
        Ok(StandardDescriptions {
            functions: Arc::new(functions),
        })
    }

    /// Runs one check with an empty refusal of the crate's own, answering its refusal as the caller's mistake.
    fn run(
        &self,
        what: &str,
        call: impl FnOnce(*mut StandardRefusal) -> c_int,
    ) -> Result<(), Answer> {
        let mut refusal = StandardRefusal {
            size: size_of::<StandardRefusal>() as u32,
            code: 0,
            detail: std::ptr::null(),
            fix: std::ptr::null(),
            path: std::ptr::null(),
            storage: std::ptr::null_mut(),
        };
        let status = call(&mut refusal);
        let answer = match status {
            STATUS_OK => Ok(()),
            STATUS_OUT_OF_MEMORY => Err(Answer::Refused {
                status: RefusalStatus::OutOfMemory,
                refusal: None,
            }),
            _ => Err(Answer::Mistake(format!(
                "{what} refused the description at `{}`: {} {}",
                text_at(refusal.path),
                text_at(refusal.detail),
                text_at(refusal.fix)
            ))),
        };
        // SAFETY: the refusal is the crate's own, released exactly once, empty or filled.
        unsafe { (self.functions.release)(&mut refusal) };
        answer
    }
}

impl DescriptionCheck for StandardDescriptions {
    fn check(&self, described: &DescribedDefinition) -> Result<(), Answer> {
        let description = described.description_address() as *const c_void;
        let definition = described.definition_address() as *const c_void;
        // SAFETY: both addresses were answered by a library the host holds open across the check, and the refusal
        // is the crate's own of its size.
        self.run("weivel_standard_description_check", |refusal| unsafe {
            (self.functions.check)(description, refusal)
        })?;
        // SAFETY: as above.
        self.run("weivel_standard_description_answers", |refusal| unsafe {
            (self.functions.answers)(description, definition, refusal)
        })
    }
}
