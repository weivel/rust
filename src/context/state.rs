//! What every context of one host shares: the core, the table of its handles, its timer and the host's log.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use std::collections::BTreeMap;

use super::log::HostLog;
use super::plugin::ContextInner;
use super::records::{Object, Record, Releases, Table, WantRecord, WantSettlement};
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
    /// The operations each published contract asks of an implementation, by its capability id.
    pub(crate) contracts: Mutex<BTreeMap<String, Vec<String>>>,
}

/// What a registration's object is to one plugin reaching it.
pub(crate) enum Reached {
    /// The object.
    Object(Object),
    /// A service the host offers each plugin on its own, offered this plugin nothing.
    NotOffered,
    /// No object the layer holds.
    Unknown,
}

/// The object the host registers once for a service it offers each plugin on its own: each plugin reaching the
/// registration is answered the object offered to it, and a plugin offered none, nothing.
#[derive(Default)]
pub(crate) struct OfferedToEach {
    pub(crate) offers: Mutex<BTreeMap<Handle, Object>>,
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

    /// What the registration `registration` reaches for `plugin`: its object, or the one offered to `plugin` alone
    /// where the host offers the service to each plugin on its own.
    pub(crate) fn reached_by(&self, plugin: Handle, registration: Handle) -> Reached {
        let Some(object) = self.object_of_registration(registration) else {
            return Reached::Unknown;
        };
        let Some(offered) = object.downcast_ref::<OfferedToEach>() else {
            return Reached::Object(object);
        };
        match offered
            .offers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&plugin)
        {
            Some(object) => Reached::Object(object.clone()),
            None => Reached::NotOffered,
        }
    }

    /// The first operation the contract published for `capability` asks that `implementation` does not offer.
    pub(crate) fn missing_operation(
        &self,
        capability: &str,
        implementation: &Object,
    ) -> Option<String> {
        let contracts = self
            .contracts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let operations = contracts.get(capability)?;
        let offered = offered_operations(implementation);
        operations
            .iter()
            .find(|operation| !offered.contains(operation))
            .cloned()
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

/// The operations an implementation offers by name: a [`crate::host::Methods`]'s names, or a bound definition's
/// operations; none for any other object.
fn offered_operations(implementation: &Object) -> Vec<String> {
    if let Some(methods) = crate::host::methods_of(implementation) {
        return methods.names();
    }
    implementation
        .downcast_ref::<crate::host::BoundDefinition>()
        .map(|bound| {
            bound
                .operation_names()
                .into_iter()
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
