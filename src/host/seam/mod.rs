//! The plugin seam: how a host reaches a plugin built as a library of its own, and how that plugin reaches the host,
//! through size-prefixed C tables of `extern "C"` functions rather than Rust types, since two separately built copies
//! of this crate agree on no Rust layout. A service object crosses as its provider's address, identity being the
//! address, and a typed view of one is a `#[repr(C)]` definition both sides compile.
//!
//! Neither side keeps a pointer into the other's memory past a call: each table it receives is copied, every text and
//! array is lent for one call, and a context or a completion crosses as the host's id of it, so a call through either
//! after the host let go of it finds nothing and answers destroyed.

mod abi;
mod host_side;
mod plugin_side;
#[cfg(test)]
mod tests;
#[forbid(unsafe_code)]
mod trees;

use std::sync::{Mutex, MutexGuard};

pub use abi::{
    SeamAnswer, SeamAnswered, SeamCapacity, SeamCompletion, SeamContext, SeamEntry, SeamEvent,
    SeamManifest, SeamNumber, SeamOffer, SeamRefusal, SeamText, SeamTexts, SeamWant,
    SeamWantLimits,
};
pub use host_side::DEFAULT_ENTRY_SYMBOL;
pub(crate) use host_side::SeamRegistry;
pub use plugin_side::seam_entry;

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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
