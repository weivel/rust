//! What a context reaches the core through: the host's own contexts in this binary, or the host's context table
//! across the plugin seam, for a plugin built as a library of its own.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use super::log::LogRecord;
use super::state::ContextState;
use crate::core::{Answer, FailureKind, Handle, LogLevel, panic_text};

/// The calls the plugin seam's context table carries, as the seam's plugin side wraps that table. A context across
/// the seam answers every other call as the caller's mistake, since the host's table carries no function for it.
pub(crate) trait SeamCalls: Send + Sync {
    /// The address of the provider's object registered under `id`.
    fn get(&self, id: &str) -> Result<usize, Answer>;
    /// Registers the object at `address` under `id`, awaiting releases when asked.
    fn register(&self, id: &str, address: usize, awaiting_releases: bool)
    -> Result<Handle, Answer>;
    /// Unregisters a registration this plugin made.
    fn unregister(&self, registration: Handle) -> Result<(), Answer>;
    /// Lets go of a holding of `registration` this plugin heard revoked awaiting its release.
    fn release_holding(&self, registration: Handle) -> Result<bool, Answer>;
    /// Provides the object at `address` as an implementation of the capability `id`.
    fn provide(&self, id: &str, address: usize) -> Result<(), Answer>;
    /// Writes to the host's log under the plugin's name.
    fn write(&self, level: LogLevel, text: &str);
    /// Tells the host's error sink.
    fn report(&self, kind: FailureKind, text: &str);
}

#[derive(Clone)]
pub(crate) enum Link {
    Local(Arc<ContextState>),
    Seam(Arc<dyn SeamCalls>),
}

pub(crate) fn not_carried(what: &str) -> Answer {
    Answer::Mistake(format!(
        "{what} is not carried by the plugin seam: the host's context table has no function for it"
    ))
}

impl Link {
    /// The contexts' shared state, or why a context across the seam cannot make the call `what` names.
    pub(crate) fn local(&self, what: &str) -> Result<&Arc<ContextState>, Answer> {
        match self {
            Link::Local(state) => Ok(state),
            Link::Seam(_) => Err(not_carried(what)),
        }
    }

    pub(crate) fn write(&self, record: &LogRecord) {
        match self {
            Link::Local(state) => state.write(record),
            Link::Seam(seam) => seam.write(record.level, &record.text),
        }
    }

    /// Runs code of a plugin's, telling the sink what it panics with.
    pub(crate) fn call_back(&self, callback: impl FnOnce(), kind: FailureKind, text: &str) {
        match self {
            Link::Local(state) => state.call_back(callback, kind, text, None),
            Link::Seam(seam) => {
                if let Err(payload) = catch_unwind(AssertUnwindSafe(callback)) {
                    seam.report(kind, &format!("{text}: {}", panic_text(&*payload)));
                }
            }
        }
    }
}
