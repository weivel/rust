//! What a context reaches the core through: the host's own contexts in this binary, or the host's context table
//! across the plugin seam, for a plugin built as a library of its own.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use super::log::LogRecord;
use super::records::{
    CapabilityWatcher, CapacityAnswerer, Object, Revocation, ServiceWatcher, WantRecord,
};
use super::services::WantLimits;
use super::state::ContextState;
use crate::core::{Answer, FailureKind, Handle, LogLevel, panic_text};

/// Every call a context makes, as the plugin seam's plugin side carries it over the host's context table. The objects
/// handed in are kept on the plugin's side, the host holding only their addresses.
pub(crate) trait SeamCalls: Send + Sync {
    fn get(&self, id: &str) -> Result<Object, Answer>;
    fn ids(&self) -> Result<Vec<String>, Answer>;
    /// Makes a want whose settlement `record` receives, answering the plugin's own id of it.
    fn want(
        &self,
        id: &str,
        limits: &WantLimits,
        record: Arc<WantRecord>,
    ) -> Result<Handle, Answer>;
    fn withdraw(&self, want: Handle) -> Result<(), Answer>;
    fn watch(&self, id: &str, watcher: Arc<dyn ServiceWatcher>) -> Result<Handle, Answer>;
    fn watch_capability(
        &self,
        id: &str,
        watcher: Arc<dyn CapabilityWatcher>,
    ) -> Result<Handle, Answer>;
    fn stop_watch(&self, watch: Handle) -> Result<(), Answer>;
    fn register(
        &self,
        id: &str,
        service: Object,
        awaiting_releases: bool,
        answerer: Option<Arc<dyn CapacityAnswerer>>,
    ) -> Result<Handle, Answer>;
    fn unregister(&self, registration: Handle) -> Result<(), Answer>;
    fn release_holding(&self, revocation: &Revocation) -> Result<bool, Answer>;
    fn provide(&self, id: &str, implementation: Object) -> Result<(), Answer>;
    fn implementations(&self, id: &str) -> Result<Vec<Object>, Answer>;
    fn capability_enums(&self, id: &str) -> Option<super::DescribedEnums>;
    fn write(&self, level: LogLevel, text: &str);
    fn report(&self, kind: FailureKind, text: &str);
    /// Whether this thread dispatches the host's core now, so a result only its dispatch settles would never settle.
    fn dispatches_here(&self) -> bool;
}

#[derive(Clone)]
pub(crate) enum Link {
    Local(Arc<ContextState>),
    Seam(Arc<dyn SeamCalls>),
}

impl Link {
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

    /// Whether this thread runs the dispatch that settles what this context's calls answer later.
    pub(crate) fn dispatches_here(&self) -> bool {
        match self {
            Link::Local(state) => state.core.dispatches_here(),
            Link::Seam(seam) => seam.dispatches_here(),
        }
    }
}
