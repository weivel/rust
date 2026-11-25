//! The contexts of every plugin of one host, and the surface a host builds, opens, ends and detaches them through.

use std::sync::{Arc, Weak};

use super::log::HostLog;
use super::plugin::Context;
use super::records::{Object, Record, Releases, Table};
use super::services::{Registering, ServiceRegistration};
use super::state::ContextState;
use crate::core::{
    Answer, Core, Event, FailureKind, Handle, ListenerRegistration, Manifest, QuarantineReason,
    Refusal, RefusalStatus,
};
use crate::handles::HandleKind;
use crate::timer::CoreTimer;

const ENDED_WITH_THE_PLUGIN: &[HandleKind] = &[
    HandleKind::ServiceRegistration,
    HandleKind::CapabilityImplementation,
    HandleKind::Watch,
];

/// What a verification came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verification {
    /// The core verified the activation.
    Verified,
    /// The core refused it, so the plugin was quarantined in the same hold of the layer lock, with no reason of the
    /// crate's; this is the core's refusal of the verification.
    Quarantined(Option<Refusal>),
}

/// The contexts of one host's plugins, over what they share: the core, the table of its handles, its timer and the
/// host's log. Every event is routed to the plugin it concerns through one listener on the core. Any thread may call.
/// Dropping it frees every entry the contexts made, so nothing the table holds keeps a context alive.
pub struct Contexts {
    state: Arc<ContextState>,
    routing: ListenerRegistration,
    host: Context,
}

impl Contexts {
    /// Makes the contexts of one host, adding the one listener that routes the core's events to them.
    pub fn create(
        core: &Core,
        timer: Arc<CoreTimer>,
        host_log: Arc<dyn HostLog>,
    ) -> Result<Contexts, Answer> {
        let state = Arc::new(ContextState {
            core: core.clone(),
            handles: Table::default(),
            timer,
            host_log,
            releases: Releases::default(),
            contracts: Default::default(),
        });
        let routed: Weak<ContextState> = Arc::downgrade(&state);
        let routing = core.add_listener(Arc::new(move |_: &Core, event: &Event| {
            if let Some(state) = routed.upgrade() {
                state.route(event);
            }
        }))?;
        let host = Context::new(
            state.clone(),
            Handle::HOST,
            Manifest {
                import_name: "host".to_owned(),
                ..Manifest::default()
            },
        );
        Ok(Contexts {
            state,
            routing,
            host,
        })
    }

    /// Registers a service the host itself supplies under `WEIVEL_HOST`, whose object is answered to a consumer as any
    /// provider's is; no release or quarantine ends it, only its unregistration or the core's destroy. `guard` runs
    /// under the layer lock before the core is asked, and an answer it gives is the registration's, the core not
    /// called; `registered` runs under the layer lock once the core answered. Neither may call into the core.
    pub fn register_host_service(
        &self,
        id: &str,
        service: Object,
        guard: impl FnOnce() -> Result<(), Answer>,
        registered: impl FnOnce(),
    ) -> Result<ServiceRegistration, Answer> {
        self.host.register_with(
            id,
            service,
            Registering {
                guard: Some(Box::new(guard)),
                registered: Some(Box::new(registered)),
                ..Registering::default()
            },
        )
    }

    /// Registers a service of the host's own as [`Contexts::register_host_service`] does, awaiting releases:
    /// unregistering it revokes it to every holder, each holding lasting until its holder lets go of it or is
    /// released, quarantined or forgotten. Once the last has ended `released` runs, from the dispatch, or when the
    /// contexts are closed for one the destroy found still registered.
    pub fn register_host_service_awaiting_releases(
        &self,
        id: &str,
        service: Object,
        registered: impl FnOnce(),
        released: impl FnOnce() + Send + 'static,
    ) -> Result<ServiceRegistration, Answer> {
        self.host.register_with(
            id,
            service,
            Registering {
                awaiting: Some(Some(Box::new(released))),
                registered: Some(Box::new(registered)),
                ..Registering::default()
            },
        )
    }

    /// Publishes the contract of `capability`: from now on a plugin's provide of it is refused as
    /// [`Answer::ContractMismatch`] when its implementation does not offer every one of `operations`. Answers
    /// [`Answer::Mistake`] for a capability whose contract is published already.
    pub fn publish_contract(
        &self,
        capability: &str,
        operations: Vec<String>,
    ) -> Result<(), Answer> {
        let mut contracts = self
            .state
            .contracts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if contracts.contains_key(capability) {
            return Err(Answer::Mistake(format!(
                "a contract for the capability {capability} is published already"
            )));
        }
        contracts.insert(capability.to_owned(), operations);
        Ok(())
    }

    /// Withdraws the contract of `capability`, so a provide of it is checked against none.
    pub fn withdraw_contract(&self, capability: &str) {
        self.state
            .contracts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(capability);
    }

    /// The object `plugin` itself registered under `id` and still has registered: a registration under the id by
    /// another plugin or the host is never answered.
    pub fn registered_by(&self, plugin: Handle, id: &str) -> Option<Object> {
        let entry = self.state.handles.find_first(|entry| {
            entry.owner == Some(plugin)
                && matches!(&entry.record, Record::Service { id: registered, .. } if registered == id)
        })?;
        match entry.record {
            Record::Service { object, .. } => Some(object),
            _ => None,
        }
    }

    /// Builds the context of a plugin the core registered, which hears no event drained before it was made. Answers
    /// [`Answer::Mistake`] when the table holds the plugin's handle already, as it does once its context is made.
    pub fn make(&self, plugin: Handle, manifest: Manifest) -> Result<Context, Answer> {
        if self.state.handles.find(plugin).is_some() {
            return Err(Answer::Mistake(format!(
                "plugin {} has its context, or another entry, already",
                plugin.0
            )));
        }
        let room = self.state.handles.reserve()?;
        let context = Context::new(self.state.clone(), plugin, manifest);
        let entry = room.add(
            HandleKind::Plugin,
            plugin,
            None,
            Record::Plugin(context.clone()),
        );
        context.set_token(entry.token);
        Ok(context)
    }

    /// The context a payload the plugin was opened with leads to; `None` once its plugin is forgotten.
    pub fn by_payload(&self, payload: u64) -> Option<Context> {
        match self.state.handles.find_by_token(payload)?.record {
            Record::Plugin(context) => Some(context),
            _ => None,
        }
    }

    /// Opens the plugin in the core with its context's token as payload, running `opened` once the core answered the
    /// open, under the layer lock and before any event of it is dispatched; it may not call into the core. A plugin
    /// opened again after a quarantine hears what is queued for it from this open on.
    pub fn open(&self, context: &Context, opened: impl FnOnce()) -> Result<(), Answer> {
        let unsilenced = context.clone();
        self.state.call(&context.inner, |hold| {
            hold.open_plugin(context.plugin(), context.token())?;
            opened();
            hold.defer(Box::new(move |_| unsilenced.unsilence()));
            Ok(())
        })
    }

    /// Verifies the plugin's activation and, when the core refuses it, quarantines the plugin in the same hold of the
    /// layer lock, with no reason of the crate's, ending its entries once the quarantine's own events are dispatched.
    pub fn verify(&self, context: &Context) -> Result<Verification, Answer> {
        let ended = context.clone();
        let state = self.state.clone();
        self.state.call(&context.inner, |hold| {
            let refusal = match hold.verify_activation(context.plugin()) {
                Ok(()) => return Ok(Verification::Verified),
                Err(Answer::Refused {
                    status: RefusalStatus::Refused,
                    refusal,
                }) => refusal,
                Err(other) => return Err(other),
            };
            let verification_refused = Answer::Refused {
                status: RefusalStatus::Refused,
                refusal: refusal.clone(),
            };
            hold.quarantine_plugin(context.plugin(), None)
                .map_err(|_| verification_refused)?;
            hold.defer(Box::new(move |_| end_entries(&state, &ended)));
            Ok(Verification::Quarantined(refusal))
        })
    }

    /// Releases the plugin in the core and ends its entries once the release's own events have been dispatched.
    pub fn release(&self, context: &Context) -> Result<(), Answer> {
        let ended = context.clone();
        let state = self.state.clone();
        self.state.call(&context.inner, |hold| {
            hold.release_plugin(context.plugin())?;
            hold.defer(Box::new(move |_| end_entries(&state, &ended)));
            Ok(())
        })
    }

    /// Quarantines the plugin in the core and ends its entries once the quarantine's own events have been
    /// dispatched. The plugin keeps its context, and [`Contexts::open`] opens it again.
    pub fn quarantine(
        &self,
        context: &Context,
        reason: Option<&QuarantineReason>,
    ) -> Result<(), Answer> {
        let ended = context.clone();
        let state = self.state.clone();
        self.state.call(&context.inner, |hold| {
            hold.quarantine_plugin(context.plugin(), reason)?;
            hold.defer(Box::new(move |_| end_entries(&state, &ended)));
            Ok(())
        })
    }

    /// Forgets the plugin in the core and, in the same hold of the layer lock, detaches its context, so no call
    /// through it reaches the core after; its entries end once every event drained before the forget is dispatched.
    pub fn forget(&self, context: &Context) -> Result<(), Answer> {
        let ended = context.clone();
        let state = self.state.clone();
        self.state.call(&context.inner, |hold| {
            hold.forget_plugin(context.plugin())?;
            context.detach();
            hold.defer(Box::new(move |_| end_forgotten(&state, &ended)));
            Ok(())
        })
    }

    /// Whether any registration of `id` the crate recorded still stands, read from the crate's own table.
    pub fn holds_a_registration_of(&self, id: &str) -> bool {
        self.state.handles.holds_any(|entry| {
            matches!(&entry.record, Record::Service { id: registered, .. } if registered == id)
        })
    }

    /// Frees what the contexts hold once the core is destroyed: the listener, every entry, and the settlement of
    /// every want left waiting, settled as [`Answer::Destroyed`] since no dispatch is left to run.
    pub fn close(&self) -> Result<(), Answer> {
        if !self.state.core.is_destroyed() {
            return Err(Answer::Mistake(
                "the core is not destroyed; destroy it before closing its contexts".to_owned(),
            ));
        }
        self.free();
        Ok(())
    }

    fn free(&self) {
        let _ = self.state.core.remove_listener(self.routing);
        for awaiting in self.state.releases.take_every_awaiting() {
            if let Some(released) = awaiting.released {
                self.state.call_back(
                    released,
                    FailureKind::LayerDefect,
                    "the record of a released registration panicked",
                    None,
                );
            }
        }
        for entry in self.state.handles.clear() {
            match entry.record {
                Record::Want(want) => {
                    drop(want.take_holding_room());
                    want.settle(Err(Answer::Destroyed));
                }
                Record::Plugin(context) => context.drop_listeners(),
                _ => {}
            }
        }
    }
}

impl Drop for Contexts {
    fn drop(&mut self) {
        self.free();
    }
}

/// Ends the plugin's watches, registrations and implementations in the crate's table once the core released or
/// quarantined it and the events that call queued have been dispatched, calling nothing in the core, which ended them
/// already. A want is left to its settlement, which the core always makes.
fn end_entries(state: &ContextState, context: &Context) {
    context.silence();
    state
        .handles
        .end_owned(context.plugin(), ENDED_WITH_THE_PLUGIN);
    state.releases.drop_revoked_of(context.plugin());
    context.drop_listeners();
}

fn end_forgotten(state: &ContextState, context: &Context) {
    let handles = &state.handles;
    handles.end_owned(context.plugin(), ENDED_WITH_THE_PLUGIN);
    state.releases.drop_revoked_of(context.plugin());
    handles.end(context.plugin());
    handles.drop_holdings(context.plugin());
    context.drop_listeners();
}
