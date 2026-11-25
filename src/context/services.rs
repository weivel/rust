//! A context's calls about services. Each takes the room its record and its holding need before the core is asked,
//! and makes the record in the same hold of the layer lock as the call, before any event of the call is dispatched.

use std::sync::{Arc, Mutex};

use super::pending::{Pending, pending};
use super::plugin::Context;
use super::records::{
    CapacityAnswerer, Object, Record, ServiceWatcher, WantRecord, WantSettlement,
};
use crate::core::{Answer, FailureKind, Handle, WantOptions};
use crate::handles::HandleKind;

/// How long a want may wait, each limit handed to the core unchanged with the timer's time as its `now`. A want with
/// none waits for as long as it takes; one with any needs the time, so on a host with no clock it is the caller's
/// mistake.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WantLimits {
    /// How long after now the want gives up.
    pub timeout_millis: Option<i64>,
    /// How long a constrained want polls before it degrades or is refused.
    pub budget_millis: Option<i64>,
    /// How often a constrained want polls.
    pub poll_millis: Option<i64>,
}

impl WantLimits {
    /// A want that waits for as long as it takes.
    pub fn unbounded() -> WantLimits {
        WantLimits::default()
    }

    /// A want that gives up `timeout_millis` after now.
    pub fn timeout(timeout_millis: i64) -> WantLimits {
        WantLimits {
            timeout_millis: Some(timeout_millis),
            ..WantLimits::default()
        }
    }

    fn needs_the_time(&self) -> bool {
        self.timeout_millis.is_some() || self.budget_millis.is_some() || self.poll_millis.is_some()
    }
}

/// A want of a service: its settlement, completed once from a dispatch step and never inside the call that made it.
pub struct Want {
    context: Context,
    want: Option<Handle>,
    settlement: Pending<WantSettlement>,
}

impl Want {
    /// The want's settlement: the provider's own object, the core's refusal, or what the call answered in place of a
    /// want when the core made none. A want withdrawn while it waits never settles.
    pub fn settlement(&self) -> Pending<WantSettlement> {
        self.settlement.clone()
    }

    /// Withdraws the want while it still waits, so the core never settles it. Answers the core's refusal for one
    /// settled already, whose settlement still completes, and [`Answer::Absent`] for one the core never made.
    pub fn withdraw(&self) -> Result<(), Answer> {
        let Some(want) = self.want else {
            return Err(Answer::Absent);
        };
        let state = &self.context.inner.state;
        let ended = state.call(&self.context.inner, |hold| {
            hold.unregister(want)?;
            Ok(state.handles.end(want))
        })?;
        if let Some(Record::Want(record)) = ended.map(|entry| entry.record) {
            drop(record.take_holding_room());
        }
        Ok(())
    }
}

/// A watch of a service or a capability, heard until it stops.
pub struct Watching {
    context: Context,
    watch: Handle,
}

impl Watching {
    pub(crate) fn new(context: Context, watch: Handle) -> Watching {
        Watching { context, watch }
    }

    /// Stops the watch; it hears nothing after.
    pub fn stop(&self) -> Result<(), Answer> {
        let state = &self.context.inner.state;
        state.call(&self.context.inner, |hold| {
            hold.unregister(self.watch)?;
            state.handles.end(self.watch);
            Ok(())
        })
    }
}

/// A service a plugin registered, until it unregisters it or stops.
pub struct ServiceRegistration {
    context: Context,
    registration: Handle,
}

impl ServiceRegistration {
    /// The registration's handle.
    pub fn handle(&self) -> Handle {
        self.registration
    }

    /// Unregisters the service.
    pub fn unregister(&self) -> Result<(), Answer> {
        let state = &self.context.inner.state;
        state.call(&self.context.inner, |hold| {
            hold.unregister(self.registration)?;
            state.handles.end(self.registration);
            Ok(())
        })
    }
}

impl Context {
    /// The provider's own object registered under `id` now, if this plugin may reach it: [`Answer::Absent`] when
    /// nothing is registered there, which is no refusal, and the core's refusal when it refuses the get.
    pub fn get(&self, id: &str) -> Result<Object, Answer> {
        let state = &self.inner.state;
        let room = state.handles.reserve()?;
        let plugin = self.plugin();
        let found = state.call(&self.inner, |hold| {
            let registration = hold.get_service(plugin, id)?;
            let object = state.object_of_registration(registration);
            if let Some(object) = &object {
                room.hold(plugin, registration, object.clone());
            }
            Ok(object)
        })?;
        found.ok_or_else(|| {
            let detail =
                "the core answered a get with a registration the layer holds no object for";
            state.report(FailureKind::ObjectUnknown, detail.to_owned(), None);
            Answer::Defect(detail.to_owned())
        })
    }

    /// The ids this plugin may reach that are registered now, in the order the core answers them.
    pub fn ids(&self) -> Result<Vec<String>, Answer> {
        let plugin = self.plugin();
        self.inner
            .state
            .call(&self.inner, |hold| hold.service_ids(plugin))
    }

    /// Wants the service registered under `id`, now or whenever it arrives.
    pub fn want(&self, id: &str, limits: WantLimits) -> Want {
        let (settlement, settle) = pending();
        let record = Arc::new(WantRecord {
            id: id.to_owned(),
            settle: Mutex::new(Some(settle)),
            holding_room: Mutex::new(None),
        });
        let want = self.make_want(id, limits, &record);
        if let Err(answer) = &want {
            self.inner.state.settle_later(record, Err(answer.clone()));
        }
        Want {
            context: self.clone(),
            want: want.ok(),
            settlement,
        }
    }

    fn make_want(
        &self,
        id: &str,
        limits: WantLimits,
        record: &Arc<WantRecord>,
    ) -> Result<Handle, Answer> {
        let state = &self.inner.state;
        if self.is_detached() {
            return Err(Answer::Detached);
        }
        let options = self.want_options(limits)?;
        let entry_room = state.handles.reserve()?;
        *record
            .holding_room
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(state.handles.reserve()?);
        let plugin = self.plugin();
        let made = state.call(&self.inner, |hold| {
            let want = hold.want_service(plugin, id, &options)?;
            entry_room.add(
                HandleKind::Want,
                want,
                Some(plugin),
                Record::Want(record.clone()),
            );
            Ok(want)
        });
        if made.is_err() {
            drop(record.take_holding_room());
        }
        made
    }

    fn want_options(&self, limits: WantLimits) -> Result<WantOptions, Answer> {
        if !limits.needs_the_time() {
            return Ok(WantOptions::default());
        }
        let now = self.inner.state.timer.time().ok_or_else(|| {
            Answer::Mistake(
                "a want with a timeout, a budget or a poll interval needs the time, and the host's scheduler has no clock"
                    .to_owned(),
            )
        })?;
        Ok(WantOptions {
            timed: limits.timeout_millis.map(|_| true),
            now,
            timeout_millis: limits.timeout_millis.unwrap_or(0),
            budgeted: limits.budget_millis.map(|_| true),
            budget_millis: limits.budget_millis.unwrap_or(0),
            poll_millis: limits.poll_millis.unwrap_or(0),
        })
    }

    /// Watches `id`: every later registration the core reports for it is heard with the provider's own object, and
    /// every unregistration with nothing, until the watch stops.
    pub fn watch(&self, id: &str, watcher: Arc<dyn ServiceWatcher>) -> Result<Watching, Answer> {
        let state = &self.inner.state;
        let room = state.handles.reserve()?;
        let plugin = self.plugin();
        let watch = state.call(&self.inner, |hold| {
            let watch = hold.watch_service(plugin, id)?;
            room.add(
                HandleKind::Watch,
                watch,
                Some(plugin),
                Record::ServiceWatch(watcher),
            );
            Ok(watch)
        })?;
        Ok(Watching::new(self.clone(), watch))
    }

    /// Registers `service` under `id`, kept as the provider's own object until the plugin unregisters it or stops.
    /// It offers no capacity answerer, so a constrained want's questions about it are answered with no live
    /// attributes and that it serves every consumer.
    pub fn register(&self, id: &str, service: Object) -> Result<ServiceRegistration, Answer> {
        self.register_with(id, service, None)
    }

    /// Registers `service` under `id` as [`Context::register`] does, with what answers a constrained want's
    /// questions about it.
    pub fn register_answering(
        &self,
        id: &str,
        service: Object,
        answerer: Arc<dyn CapacityAnswerer>,
    ) -> Result<ServiceRegistration, Answer> {
        self.register_with(id, service, Some(answerer))
    }

    fn register_with(
        &self,
        id: &str,
        service: Object,
        answerer: Option<Arc<dyn CapacityAnswerer>>,
    ) -> Result<ServiceRegistration, Answer> {
        let state = &self.inner.state;
        let room = state.handles.reserve()?;
        let plugin = self.plugin();
        let registration = state.call(&self.inner, |hold| {
            let registration = hold.register_service(plugin, id)?;
            room.add(
                HandleKind::ServiceRegistration,
                registration,
                Some(plugin),
                Record::Service {
                    id: id.to_owned(),
                    object: service,
                    answerer,
                },
            );
            Ok(registration)
        })?;
        Ok(ServiceRegistration {
            context: self.clone(),
            registration,
        })
    }
}
