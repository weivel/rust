//! A context's calls about services. Each takes the room its record and its holding need before the core is asked,
//! and makes the record in the same hold of the layer lock as the call, before any event of the call is dispatched.

use std::sync::{Arc, Mutex};

use super::link::Link;
use super::pending::{Pending, pending_settled_by};
use super::plugin::Context;
use super::records::{
    Awaiting, CapacityAnswerer, Object, Record, Released, Revocation, ServiceWatcher, WantRecord,
    WantSettlement,
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
        let state = match &self.context.inner.link {
            Link::Local(state) => state,
            Link::Seam(seam) => return seam.withdraw(want),
        };
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
        let state = match &self.context.inner.link {
            Link::Local(state) => state,
            Link::Seam(seam) => return seam.stop_watch(self.watch),
        };
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
        self.unregister_then(|| {})
    }

    /// Unregisters the service, running `unregistered` under the layer lock once the core answered it.
    pub(crate) fn unregister_then(&self, unregistered: impl FnOnce()) -> Result<(), Answer> {
        let state = match &self.context.inner.link {
            Link::Local(state) => state,
            Link::Seam(seam) => return seam.unregister(self.registration),
        };
        state.call(&self.context.inner, |hold| {
            hold.unregister(self.registration)?;
            state.handles.end(self.registration);
            unregistered();
            Ok(())
        })
    }
}

impl Context {
    /// The provider's own object registered under `id` now, if this plugin may reach it: [`Answer::Absent`] when
    /// nothing is registered there, which is no refusal, and the core's refusal when it refuses the get.
    pub fn get(&self, id: &str) -> Result<Object, Answer> {
        let state = match &self.inner.link {
            Link::Local(state) => state,
            Link::Seam(seam) => return seam.get(id),
        };
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
        match &self.inner.link {
            Link::Local(state) => state.call(&self.inner, |hold| hold.service_ids(plugin)),
            Link::Seam(seam) => seam.ids(),
        }
    }

    /// Wants the service registered under `id`, now or whenever it arrives.
    pub fn want(&self, id: &str, limits: WantLimits) -> Want {
        let link = self.inner.link.clone();
        let (settlement, settle) = pending_settled_by(Arc::new(move || link.dispatches_here()));
        let record = Arc::new(WantRecord {
            id: id.to_owned(),
            settle: Mutex::new(Some(settle)),
            holding_room: Mutex::new(None),
        });
        let want = self.make_want(id, limits, &record);
        if let Err(answer) = &want {
            match &self.inner.link {
                Link::Local(state) => state.settle_later(record, Err(answer.clone())),
                Link::Seam(_) => record.settle(Err(answer.clone())),
            }
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
        if self.is_detached() {
            return Err(Answer::Detached);
        }
        let state = match &self.inner.link {
            Link::Local(state) => state,
            Link::Seam(seam) => return seam.want(id, &limits, record.clone()),
        };
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
        let Link::Local(state) = &self.inner.link else {
            return Ok(WantOptions::default());
        };
        let now = state.timer.time().ok_or_else(|| {
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
        let state = match &self.inner.link {
            Link::Local(state) => state,
            Link::Seam(seam) => return Ok(Watching::new(self.clone(), seam.watch(id, watcher)?)),
        };
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
        self.register_with(id, service, Registering::default())
    }

    /// Registers `service` under `id` as [`Context::register`] does, with what answers a constrained want's
    /// questions about it.
    pub fn register_answering(
        &self,
        id: &str,
        service: Object,
        answerer: Arc<dyn CapacityAnswerer>,
    ) -> Result<ServiceRegistration, Answer> {
        self.register_with(
            id,
            service,
            Registering {
                answerer: Some(answerer),
                ..Registering::default()
            },
        )
    }

    /// Registers `service` under `id` as [`Context::register`] does, awaiting releases: once it is unregistered, by
    /// the plugin or by its stop, each holder keeps its holding until it lets go of the object, and once the last has,
    /// this plugin hears [`PluginEvent::HoldingsReleased`], so whatever stood behind the object may be freed. It hears
    /// it as any event about it: one its own release or quarantine queued still reaches it, and one queued after
    /// those events, or after its forget, reaches no one.
    ///
    /// [`PluginEvent::HoldingsReleased`]: super::PluginEvent::HoldingsReleased
    pub fn register_awaiting_releases(
        &self,
        id: &str,
        service: Object,
    ) -> Result<ServiceRegistration, Answer> {
        self.register_with(
            id,
            service,
            Registering {
                awaiting: Some(None),
                ..Registering::default()
            },
        )
    }

    /// Lets go of what a revocation that awaits its release withdrew: the plugin no longer uses the object nor
    /// anything it answered, and the core ends the holding. The revocation names the registration, so an object
    /// registered under several is let go of under that one alone. A call through the object made afterwards is the
    /// plugin's defect, as any use of memory after its free is. Answers true; false, calling nothing, for a revocation
    /// that awaits no release, or one this plugin let go of already or was released or forgotten since; the core's
    /// refusal.
    pub fn release_holding(&self, revocation: &Revocation) -> Result<bool, Answer> {
        let state = match &self.inner.link {
            Link::Local(state) => state,
            Link::Seam(seam) => return seam.release_holding(revocation),
        };
        if !revocation.awaits_release
            || !state.releases.take_revoked(
                self.plugin(),
                revocation.registration,
                &revocation.object,
            )
        {
            return Ok(false);
        }
        let plugin = self.plugin();
        state.call(&self.inner, |hold| {
            hold.release_holding(plugin, revocation.registration)
        })?;
        Ok(true)
    }

    pub(crate) fn register_with(
        &self,
        id: &str,
        service: Object,
        registering: Registering<'_>,
    ) -> Result<ServiceRegistration, Answer> {
        let Registering {
            answerer,
            awaiting,
            guard,
            registered,
        } = registering;
        let state = match &self.inner.link {
            Link::Local(state) => state,
            Link::Seam(seam) => {
                let registration = seam.register(id, service, awaiting.is_some(), answerer)?;
                return Ok(ServiceRegistration {
                    context: self.clone(),
                    registration,
                });
            }
        };
        let room = state.handles.reserve()?;
        let plugin = self.plugin();
        let registration = state.call(&self.inner, |hold| {
            if let Some(guard) = guard {
                guard()?;
            }
            let registration = match awaiting {
                Some(_) => hold.register_service_awaiting_releases(plugin, id)?,
                None => hold.register_service(plugin, id)?,
            };
            room.add(
                HandleKind::ServiceRegistration,
                registration,
                Some(plugin),
                Record::Service {
                    id: id.to_owned(),
                    object: service.clone(),
                    answerer,
                },
            );
            if let Some(released) = awaiting {
                state.releases.await_releases(
                    registration,
                    Awaiting {
                        id: id.to_owned(),
                        service,
                        released,
                    },
                );
            }
            if let Some(registered) = registered {
                registered();
            }
            Ok(registration)
        })?;
        Ok(ServiceRegistration {
            context: self.clone(),
            registration,
        })
    }
}

/// How a registration is made beyond its id and object, each part run under the layer lock in the call's hold.
#[derive(Default)]
pub(crate) struct Registering<'a> {
    /// What answers a constrained want's questions about it.
    pub(crate) answerer: Option<Arc<dyn CapacityAnswerer>>,
    /// `Some` to register awaiting releases, with what runs once nothing holds it any more.
    pub(crate) awaiting: Option<Option<Released>>,
    /// The caller's own check before the core is asked: an answer it gives is the registration's, the core not called.
    pub(crate) guard: Option<Box<dyn FnOnce() -> Result<(), Answer> + 'a>>,
    /// The caller's own record of the registration, once the core answered it.
    pub(crate) registered: Option<Box<dyn FnOnce() + 'a>>,
}
