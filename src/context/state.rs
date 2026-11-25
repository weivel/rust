//! What every context of one host shares: the core, the table of its handles, its timer and the host's log.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use super::log::HostLog;
use super::plugin::ContextInner;
use super::records::{Record, Releases, Table, WantRecord, WantSettlement};
use crate::core::{Answer, Core, Failure, FailureKind, Handle, Hold, panic_text};
use crate::handles::HandleKind;
use crate::timer::CoreTimer;

fn take<T>(kept: &Mutex<Option<T>>) -> Option<T> {
    kept.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}

pub(crate) struct ContextState {
    pub(crate) core: Core,
    pub(crate) handles: Table,
    pub(crate) timer: Arc<CoreTimer>,
    pub(crate) host_log: Arc<dyn HostLog>,
    pub(crate) releases: Releases,
}

impl ContextState {
    /// Makes calls of the core in one hold of the layer lock, answering [`Answer::Detached`] without reaching the
    /// core once the context's plugin is forgotten, decided under the lock as the call is made.
    pub(crate) fn call<T>(
        &self,
        context: &ContextInner,
        body: impl FnOnce(&mut Hold<'_>) -> Result<T, Answer>,
    ) -> Result<T, Answer> {
        if context.is_detached() {
            return Err(Answer::Detached);
        }
        self.core.hold(|hold| {
            if context.is_detached() {
                return Err(Answer::Detached);
            }
            body(hold)
        })
    }

    pub(crate) fn record_of(&self, handle: Handle, kind: HandleKind) -> Option<Record> {
        self.handles
            .find(handle)
            .filter(|entry| entry.kind == kind)
            .map(|entry| entry.record)
    }

    pub(crate) fn object_of_registration(&self, registration: Handle) -> Option<super::Object> {
        match self.record_of(registration, HandleKind::ServiceRegistration) {
            Some(Record::Service { object, .. }) => Some(object),
            _ => None,
        }
    }

    pub(crate) fn object_of_implementation(&self, implementation: Handle) -> Option<super::Object> {
        match self.record_of(implementation, HandleKind::CapabilityImplementation) {
            Some(Record::Implementation { object }) => Some(object),
            _ => None,
        }
    }

    /// Settles a want from a dispatch step, never inside the call that made it, or at once when the core is gone.
    pub(crate) fn settle_later(&self, want: Arc<WantRecord>, settlement: WantSettlement) {
        let kept = Arc::new(Mutex::new(Some(settlement)));
        let deferred = (want.clone(), kept.clone());
        let deferral = self.core.defer(move |_| {
            let (want, kept) = deferred;
            if let Some(settlement) = take(&kept) {
                want.settle(settlement);
            }
        });
        if deferral.is_err()
            && let Some(settlement) = take(&kept)
        {
            want.settle(settlement);
        }
    }

    /// Runs code of a plugin's or the host's, telling the sink what it panics with.
    pub(crate) fn call_back(
        &self,
        callback: impl FnOnce(),
        kind: FailureKind,
        text: &str,
        sequence: Option<u64>,
    ) {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(callback)) {
            self.report(kind, format!("{text}: {}", panic_text(&*payload)), sequence);
        }
    }

    pub(crate) fn report(&self, kind: FailureKind, text: String, sequence: Option<u64>) {
        self.core.report(&Failure {
            kind,
            text,
            sequence,
        });
    }
}
