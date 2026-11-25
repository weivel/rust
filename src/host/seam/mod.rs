//! The plugin seam: how a host reaches a plugin built as a library of its own, and how that plugin reaches the host,
//! through size-prefixed C tables of `extern "C"` functions rather than Rust types, since two separately built copies
//! of this crate agree on no Rust layout. A service object crosses as its provider's address, identity being the
//! address, and a typed view of one is a `#[repr(C)]` definition both sides compile.

mod abi;
mod host_side;
mod plugin_side;

use std::sync::{Mutex, MutexGuard};

pub use abi::{SeamCompletion, SeamContext, SeamEntry, SeamEvent, SeamText};
pub use host_side::DEFAULT_ENTRY_SYMBOL;
pub(crate) use host_side::PluginLibrary;
pub use plugin_side::seam_entry;

use crate::core::FailureKind;

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The kinds of failure a plugin's side tells the host's sink of across the seam.
fn failure_kind_code(kind: FailureKind) -> u32 {
    match kind {
        FailureKind::Log => 2,
        FailureKind::PluginDeactivate => 3,
        _ => 1,
    }
}

fn failure_kind_of(code: u32) -> FailureKind {
    match code {
        2 => FailureKind::Log,
        3 => FailureKind::PluginDeactivate,
        _ => FailureKind::PluginCallback,
    }
}

/// Exports a plugin's entry from its library for a host to load with [`crate::host::Host::load_entry`]: under
/// [`DEFAULT_ENTRY_SYMBOL`], or under the symbol named first.
///
/// ```ignore
/// weivel_rust::export_entry!(MyEntry::default());
/// weivel_rust::export_entry!(another_entry, AnotherEntry::default());
/// ```
#[macro_export]
macro_rules! export_entry {
    ($symbol:ident, $entry:expr) => {
        /// The plugin's entry table, for the host's `load_entry`.
        #[unsafe(no_mangle)]
        pub extern "C" fn $symbol() -> *const $crate::host::SeamEntry {
            $crate::host::seam_entry(|| ::std::sync::Arc::new($entry))
        }
    };
    ($entry:expr) => {
        $crate::export_entry!(weivel_rust_entry, $entry);
    };
}
