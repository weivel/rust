//! Every function the core exports, resolved by name from its binary into one table. Nothing else in the crate names
//! a function of the core, so one build runs against whichever core binary its caller loads.

use std::ffi::{CStr, c_char, c_void};
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use super::abi;
use super::binary::SharedLibrary;

type CorePointer = *mut c_void;

macro_rules! core_functions {
    ($($name:ident($($argument:ty),* $(,)?);)*) => {
        /// The core's functions, one field each, named as the core exports them.
        pub(crate) struct CoreFunctions {
            $(pub(crate) $name: unsafe extern "C" fn($($argument),*) -> i32,)*
        }

        /// The name of every function the core exports, in the order the table holds them.
        pub(crate) const FUNCTION_NAMES: &[&str] = &[$(stringify!($name)),*];

        impl CoreFunctions {
            fn resolve(library: &SharedLibrary) -> Result<CoreFunctions, &'static str> {
                Ok(CoreFunctions {
                    $($name: {
                        let symbol = concat!(stringify!($name), "\0");
                        let name = CStr::from_bytes_with_nul(symbol.as_bytes()).expect("a symbol ends in its NUL");
                        let address = library.symbol(name).ok_or(stringify!($name))?;
                        // SAFETY: the core's header declares this symbol as a function of exactly this type, and a
                        // function's address fits an object pointer on every supported platform.
                        unsafe {
                            std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($argument),*) -> i32>(address)
                        }
                    },)*
                })
            }
        }
    };
}

core_functions! {
    weivel_core_create(*const abi::CoreOptions, *mut CorePointer, *mut u32);
    weivel_core_destroy(CorePointer);
    weivel_next_event(CorePointer, *mut abi::Event, *mut u32);
    weivel_event_release(*mut abi::Event);
    weivel_next_deadline(CorePointer, *mut i64, *mut u32);
    weivel_advance(CorePointer, i64, *mut u32);
    weivel_supports(CorePointer, *const abi::Manifest, *mut abi::Support, *mut u32);
    weivel_support_release(*mut abi::Support);
    weivel_register_manifest(CorePointer, *const abi::Manifest, *mut u64, *mut u32);
    weivel_plan_activation(CorePointer, *const u64, u32, u32, *const abi::Strings, *mut abi::ActivationPlan, *mut u32);
    weivel_activation_plan_release(*mut abi::ActivationPlan);
    weivel_open_plugin(CorePointer, u64, *const abi::PluginOptions, *mut u32);
    weivel_plugin_payload(CorePointer, u64, *mut u64, *mut u32);
    weivel_release_plugin(CorePointer, u64, *mut u32);
    weivel_quarantine_plugin(CorePointer, u64, *const abi::QuarantineReason, *mut u32);
    weivel_forget_plugin(CorePointer, u64, *mut u32);
    weivel_register_service(CorePointer, u64, *const c_char, *mut u64, *mut u32);
    weivel_register_service_awaiting_releases(CorePointer, u64, *const c_char, *mut u64, *mut u32);
    weivel_release_holding(CorePointer, u64, u64, *mut u32);
    weivel_unregister(CorePointer, u64, *mut u32);
    weivel_handle_state(CorePointer, u64, *mut u32, *mut u32);
    weivel_get_service(CorePointer, u64, *const c_char, *mut u64, *mut u32);
    weivel_want_service(CorePointer, u64, *const c_char, *const abi::WantOptions, *mut u64, *mut u32);
    weivel_report_capacity(CorePointer, u64, u64, *const abi::Capacity, i64, *mut u32);
    weivel_watch_service(CorePointer, u64, *const c_char, *mut u64, *mut u32);
    weivel_service_ids(CorePointer, u64, *mut abi::ServiceList, *mut u32);
    weivel_unbound_services(CorePointer, u64, *mut abi::ServiceList, *mut u32);
    weivel_service_list_release(*mut abi::ServiceList);
    weivel_provide_capability(CorePointer, u64, *const c_char, *mut u64, *mut u32);
    weivel_verify_activation(CorePointer, u64, *mut u32);
    weivel_all_capabilities(CorePointer, u64, *const c_char, *mut abi::CapabilityList, *mut u32);
    weivel_capability_list_release(*mut abi::CapabilityList);
    weivel_watch_capability(CorePointer, u64, *const c_char, *mut u64, *mut u32);
    weivel_ledger(CorePointer, *mut abi::LedgerSnapshot, *mut u32);
    weivel_ledger_snapshot_release(*mut abi::LedgerSnapshot);
    weivel_ledger_entry(CorePointer, u64, *mut abi::LedgerRow, *mut u32);
    weivel_ledger_row_release(*mut abi::LedgerRow);
}

/// Why a core's binary could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadFailure {
    /// The path names no library this process can load.
    NotALibrary,
    /// The library lacks a function of the core, named here: the first one the table found missing.
    MissingFunction(&'static str),
}

impl fmt::Display for LoadFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadFailure::NotALibrary => {
                formatter.write_str("the path names no library this process can load")
            }
            LoadFailure::MissingFunction(name) => {
                write!(formatter, "the library exports no function {name}")
            }
        }
    }
}

impl std::error::Error for LoadFailure {}

pub(crate) struct LoadedCore {
    pub(crate) functions: CoreFunctions,
    _library: SharedLibrary,
}

/// The core's binary, loaded from the path its caller gives and closed when the last core made through it, and the
/// last clone of this value, are gone.
#[derive(Clone)]
pub struct CoreBinary {
    pub(crate) loaded: Arc<LoadedCore>,
}

impl CoreBinary {
    /// Loads the core's shared binary from `path` and resolves every function the core exports.
    pub fn load(path: impl AsRef<Path>) -> Result<CoreBinary, LoadFailure> {
        let library = SharedLibrary::open(path.as_ref()).ok_or(LoadFailure::NotALibrary)?;
        let functions = CoreFunctions::resolve(&library).map_err(LoadFailure::MissingFunction)?;
        Ok(CoreBinary {
            loaded: Arc::new(LoadedCore {
                functions,
                _library: library,
            }),
        })
    }
}

impl fmt::Debug for CoreBinary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CoreBinary")
    }
}
