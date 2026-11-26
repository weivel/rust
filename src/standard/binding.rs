//! The standard's manifest loader, library declaration reader and service ids, resolved by name from the standard's
//! binary and declared by hand from `standard.h`, `library.h`, `services.h`, `scope.h` and `host.h`.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use super::description::{StandardDescriptions, StandardRefusal};
use super::json::StandardJson;
use crate::core::binary::SharedLibrary;
use crate::core::{LoadFailure, Manifest, manifest_at};
use crate::host::{
    FunctionKind, LibraryContract, LibraryDeclaration, LibraryFunction, LibraryServiceDeclaration,
};

const STATUS_OK: c_int = 0;
const FUNCTION_DEFINITION: u32 = 3;

/// `weivel_strings`, declared by hand from the core's `core.h`.
#[repr(C)]
pub(crate) struct Strings {
    pub(crate) items: *const *const c_char,
    pub(crate) count: u32,
}

/// `weivel_standard_validation`, declared by hand from `standard.h`.
#[repr(C)]
pub(crate) struct Validation {
    pub(crate) size: u32,
    pub(crate) valid: u32,
    pub(crate) paths: Strings,
    pub(crate) refusal: StandardRefusal,
    pub(crate) storage: *mut c_void,
}

/// `weivel_standard_library_function`, declared by hand from `library.h`.
#[repr(C)]
pub(crate) struct DeclaredFunction {
    pub(crate) name: *const c_char,
    pub(crate) symbol: *const c_char,
    pub(crate) kind: u32,
    pub(crate) context: u32,
}

/// `weivel_standard_library_service`, declared by hand from `library.h`.
#[repr(C)]
pub(crate) struct DeclaredService {
    pub(crate) id: *const c_char,
    pub(crate) functions: *const DeclaredFunction,
    pub(crate) function_count: usize,
}

/// `weivel_standard_library_contract`, declared by hand from `library.h`.
#[repr(C)]
pub(crate) struct DeclaredContract {
    pub(crate) capability: *const c_char,
    pub(crate) description: *const c_char,
}

/// `weivel_standard_library`, declared by hand from `library.h`.
#[repr(C)]
pub(crate) struct DeclaredLibrary {
    pub(crate) size: u32,
    pub(crate) import_name: *const c_char,
    pub(crate) version: *const c_char,
    pub(crate) release: *const c_char,
    pub(crate) services: *const DeclaredService,
    pub(crate) service_count: usize,
    pub(crate) storage: *mut c_void,
    pub(crate) descriptions: *const *const c_char,
    pub(crate) contracts: *const DeclaredContract,
    pub(crate) contract_count: usize,
}

type LoadManifest = unsafe extern "C" fn(
    *const c_char,
    usize,
    *const c_void,
    *mut *mut c_void,
    *mut Validation,
) -> c_int;
type ManifestView = unsafe extern "C" fn(*const c_void) -> *const c_void;
type ManifestLanguage = unsafe extern "C" fn(*const c_void) -> *const c_char;
type ManifestRelease = unsafe extern "C" fn(*mut c_void);
type ValidationRelease = unsafe extern "C" fn(*mut Validation);
type ReadLibrary =
    unsafe extern "C" fn(*const c_char, usize, *mut DeclaredLibrary, *mut StandardRefusal) -> c_int;
type LibraryRelease = unsafe extern "C" fn(*mut DeclaredLibrary);
type RefusalRelease = unsafe extern "C" fn(*mut StandardRefusal);
type ServiceId = unsafe extern "C" fn() -> *const c_char;

struct StandardFunctions {
    load_manifest: LoadManifest,
    manifest_view: ManifestView,
    manifest_language: ManifestLanguage,
    manifest_release: ManifestRelease,
    validation_release: ValidationRelease,
    read_library: ReadLibrary,
    library_release: LibraryRelease,
    refusal_release: RefusalRelease,
    _library: SharedLibrary,
}

/// The ids of the standard's services, as the standard's binary answers them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardServiceIds {
    /// `weivel.config`.
    pub config: String,
    /// `weivel.secrets`.
    pub secrets: String,
    /// `weivel.paths`.
    pub paths: String,
    /// `weivel.log`.
    pub log: String,
    /// `weivel.scope`.
    pub scope: String,
    /// `weivel.host`.
    pub host: String,
}

/// A manifest the standard validated and read: the core's `Manifest`, and beside it the language the manifest names,
/// the weivel implementation that loads the plugin, which the core's manifest has no place for.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedManifest {
    /// The manifest as the core takes it.
    pub manifest: Manifest,
    /// The weivel implementation that loads the plugin, such as `rust`.
    pub language: String,
}

/// Why the standard refused a document: its code of `standard.h`, what happened and what fixes it in English, and the
/// path of the field, empty for the whole document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardRefused {
    /// A `weivel_standard_refusal_code`.
    pub code: u32,
    /// What happened.
    pub detail: String,
    /// What would make it succeed.
    pub fix: String,
    /// The field, as `services.consumes[1].provider`.
    pub path: String,
}

impl fmt::Display for StandardRefused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            write!(formatter, "{}", self.detail)
        } else {
            write!(formatter, "{}: {}", self.path, self.detail)
        }
    }
}

impl std::error::Error for StandardRefused {}

/// The standard, bound from its shared binary: its manifest loader, its library declaration reader, its service ids,
/// its JSON reader and its description checks. Cloning it shares the binding.
#[derive(Clone)]
pub struct Standard {
    functions: Arc<StandardFunctions>,
    service_ids: StandardServiceIds,
    json: StandardJson,
    descriptions: StandardDescriptions,
}

impl fmt::Debug for Standard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Standard")
    }
}

fn resolve(library: &SharedLibrary, name: &'static CStr) -> Result<*mut c_void, LoadFailure> {
    library.symbol(name).ok_or(LoadFailure::MissingFunction(
        name.to_str().unwrap_or("a standard function"),
    ))
}

/// The text at `pointer`, empty for NULL, read where a text is not UTF-8 with each such byte replaced, since only an
/// identifier or an English detail the standard wrote is read so.
fn text_at(pointer: *const c_char) -> String {
    optional_text_at(pointer).unwrap_or_default()
}

fn optional_text_at(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: every text the standard answers is NUL-terminated and lives until its struct's release.
    Some(
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// The elements at `items`, none for NULL.
///
/// # Safety
/// `items` holds `count` elements, or is NULL.
unsafe fn elements<'a, T>(items: *const T, count: usize) -> &'a [T] {
    if items.is_null() || count == 0 {
        return &[];
    }
    // SAFETY: as the caller promised.
    unsafe { std::slice::from_raw_parts(items, count) }
}

fn empty_refusal() -> StandardRefusal {
    StandardRefusal {
        size: size_of::<StandardRefusal>() as u32,
        code: 0,
        detail: std::ptr::null(),
        fix: std::ptr::null(),
        path: std::ptr::null(),
        storage: std::ptr::null_mut(),
    }
}

fn refused_from(refusal: &StandardRefusal, status: c_int) -> StandardRefused {
    StandardRefused {
        code: refusal.code,
        detail: optional_text_at(refusal.detail).unwrap_or_else(|| {
            format!("the standard answered status {status} and said nothing more")
        }),
        fix: text_at(refusal.fix),
        path: text_at(refusal.path),
    }
}

fn service_id(library: &SharedLibrary, name: &'static CStr) -> Result<String, LoadFailure> {
    let address = resolve(library, name)?;
    // SAFETY: `services.h`, `scope.h` and `host.h` declare each id function as `const char *f(void)`, answering a
    // static NUL-terminated string.
    let id = unsafe { std::mem::transmute::<*mut c_void, ServiceId>(address)() };
    Ok(text_at(id))
}

fn service_ids(library: &SharedLibrary) -> Result<StandardServiceIds, LoadFailure> {
    Ok(StandardServiceIds {
        config: service_id(library, c"weivel_standard_service_config")?,
        secrets: service_id(library, c"weivel_standard_service_secrets")?,
        paths: service_id(library, c"weivel_standard_service_paths")?,
        log: service_id(library, c"weivel_standard_service_log")?,
        scope: service_id(library, c"weivel_standard_service_scope")?,
        host: service_id(library, c"weivel_standard_service_host")?,
    })
}

fn functions_of(library: SharedLibrary) -> Result<StandardFunctions, LoadFailure> {
    let load_manifest = resolve(&library, c"weivel_standard_load_manifest")?;
    let manifest_view = resolve(&library, c"weivel_standard_manifest_view")?;
    let manifest_language = resolve(&library, c"weivel_standard_manifest_language")?;
    let manifest_release = resolve(&library, c"weivel_standard_manifest_release")?;
    let validation_release = resolve(&library, c"weivel_standard_validation_release")?;
    let read_library = resolve(&library, c"weivel_standard_read_library")?;
    let library_release = resolve(&library, c"weivel_standard_library_release")?;
    let refusal_release = resolve(&library, c"weivel_standard_refusal_release")?;
    // SAFETY: `standard.h` and `library.h` declare these symbols as functions of exactly these types.
    unsafe {
        Ok(StandardFunctions {
            load_manifest: std::mem::transmute::<*mut c_void, LoadManifest>(load_manifest),
            manifest_view: std::mem::transmute::<*mut c_void, ManifestView>(manifest_view),
            manifest_language: std::mem::transmute::<*mut c_void, ManifestLanguage>(
                manifest_language,
            ),
            manifest_release: std::mem::transmute::<*mut c_void, ManifestRelease>(manifest_release),
            validation_release: std::mem::transmute::<*mut c_void, ValidationRelease>(
                validation_release,
            ),
            read_library: std::mem::transmute::<*mut c_void, ReadLibrary>(read_library),
            library_release: std::mem::transmute::<*mut c_void, LibraryRelease>(library_release),
            refusal_release: std::mem::transmute::<*mut c_void, RefusalRelease>(refusal_release),
            _library: library,
        })
    }
}

impl Standard {
    /// Opens the standard's shared binary at `path` and binds every function the crate calls of it.
    pub fn load(path: impl AsRef<Path>) -> Result<Standard, LoadFailure> {
        let path = path.as_ref();
        let library = SharedLibrary::open(path).ok_or(LoadFailure::NotALibrary)?;
        let service_ids = service_ids(&library)?;
        Ok(Standard {
            functions: Arc::new(functions_of(library)?),
            service_ids,
            json: StandardJson::load(path)?,
            descriptions: StandardDescriptions::load(path)?,
        })
    }

    /// The ids of the standard's services.
    pub fn service_ids(&self) -> &StandardServiceIds {
        &self.service_ids
    }

    /// The standard's JSON reader.
    pub fn json(&self) -> &StandardJson {
        &self.json
    }

    /// The standard's description checks, the description check a host takes.
    pub fn descriptions(&self) -> &StandardDescriptions {
        &self.descriptions
    }

    /// Validates a JSON manifest by the standard's schema and its rules, and reads it only when it is valid, as
    /// `weivel_standard_load_manifest` does, with the language it names beside it; answers the first field that is
    /// wrong, or the reader's refusal.
    pub fn load_manifest(&self, json: &[u8]) -> Result<LoadedManifest, StandardRefused> {
        let mut validation = Validation {
            size: size_of::<Validation>() as u32,
            valid: 0,
            paths: Strings {
                items: std::ptr::null(),
                count: 0,
            },
            refusal: empty_refusal(),
            storage: std::ptr::null_mut(),
        };
        let mut loaded: *mut c_void = std::ptr::null_mut();
        let functions = &self.functions;
        // SAFETY: the document is lent for the call, and the answer and the validation are the crate's own.
        let status = unsafe {
            (functions.load_manifest)(
                json.as_ptr().cast(),
                json.len(),
                std::ptr::null(),
                &mut loaded,
                &mut validation,
            )
        };
        let answer = if status == STATUS_OK && !loaded.is_null() {
            // SAFETY: the view and the language of a loaded manifest live until its release, the view a
            // `weivel_manifest`.
            Ok(unsafe {
                LoadedManifest {
                    manifest: manifest_at((functions.manifest_view)(loaded)),
                    language: text_at((functions.manifest_language)(loaded)),
                }
            })
        } else {
            Err(refused_from(&validation.refusal, status))
        };
        // SAFETY: both are released exactly once, filled or empty.
        unsafe {
            (functions.manifest_release)(loaded);
            (functions.validation_release)(&mut validation);
        }
        answer
    }

    /// Reads a C library's declaration and checks it by the declaration's rules, as `weivel_standard_read_library`
    /// does; answers the first field that breaks one.
    pub fn read_library(&self, json: &[u8]) -> Result<LibraryDeclaration, StandardRefused> {
        let mut declared = DeclaredLibrary {
            size: size_of::<DeclaredLibrary>() as u32,
            import_name: std::ptr::null(),
            version: std::ptr::null(),
            release: std::ptr::null(),
            services: std::ptr::null(),
            service_count: 0,
            storage: std::ptr::null_mut(),
            descriptions: std::ptr::null(),
            contracts: std::ptr::null(),
            contract_count: 0,
        };
        let mut refusal = empty_refusal();
        let functions = &self.functions;
        // SAFETY: the document is lent for the call, and the declaration and the refusal are the crate's own.
        let status = unsafe {
            (functions.read_library)(
                json.as_ptr().cast(),
                json.len(),
                &mut declared,
                &mut refusal,
            )
        };
        let answer = if status == STATUS_OK {
            // SAFETY: a declaration read OK points only at what lives until its release.
            Ok(unsafe { declaration_of(&declared) })
        } else {
            Err(refused_from(&refusal, status))
        };
        // SAFETY: both are released exactly once, filled or empty.
        unsafe {
            (functions.library_release)(&mut declared);
            (functions.refusal_release)(&mut refusal);
        }
        answer
    }
}

/// # Safety
/// `declared` was filled by the standard, and everything it points at lives for the call.
unsafe fn declaration_of(declared: &DeclaredLibrary) -> LibraryDeclaration {
    // SAFETY: as the caller promised.
    let services = unsafe { elements(declared.services, declared.service_count) };
    // SAFETY: the descriptions run beside the services, or are NULL.
    let descriptions = unsafe { elements(declared.descriptions, declared.service_count) };
    LibraryDeclaration {
        import_name: text_at(declared.import_name),
        version: text_at(declared.version),
        release: optional_text_at(declared.release),
        services: services
            .iter()
            .enumerate()
            .map(|(index, service)| LibraryServiceDeclaration {
                id: text_at(service.id),
                // SAFETY: as above.
                functions: unsafe { elements(service.functions, service.function_count) }
                    .iter()
                    .map(function_of)
                    .collect(),
                description: descriptions
                    .get(index)
                    .and_then(|symbol| optional_text_at(*symbol)),
            })
            .collect(),
        // SAFETY: as above.
        contracts: unsafe { elements(declared.contracts, declared.contract_count) }
            .iter()
            .map(|contract| LibraryContract {
                capability: text_at(contract.capability),
                description: text_at(contract.description),
            })
            .collect(),
    }
}

fn function_of(function: &DeclaredFunction) -> LibraryFunction {
    LibraryFunction {
        name: text_at(function.name),
        symbol: text_at(function.symbol),
        kind: if function.kind == FUNCTION_DEFINITION {
            FunctionKind::Definition
        } else {
            FunctionKind::Bytes
        },
        context: function.context,
    }
}
