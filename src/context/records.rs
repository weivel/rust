//! What a plugin hands the crate and hears from it, and the records the handles table keeps for each handle.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::Context;
use super::pending::Settle;
use crate::core::{Answer, Handle, Number, ReachKind, Refusal};
use crate::handles::{Handles, Reservation};

/// A provider's own object: a service or an implementation of a capability. A consumer is handed exactly this
/// object, never a copy or a wrapper, so identity is [`Arc::ptr_eq`].
pub type Object = Arc<dyn Any + Send + Sync>;

/// Hears the registrations and unregistrations of a watched service.
pub trait ServiceWatcher: Send + Sync {
    /// The service was registered, and this is the provider's object.
    fn registered(&self, service: &Object);
    /// The service was unregistered.
    fn unregistered(&self);
}

/// Hears the implementations of a watched capability join and leave.
pub trait CapabilityWatcher: Send + Sync {
    /// A verified implementation joined.
    fn provided(&self, implementation: &Object);
    /// That implementation left.
    fn withdrawn(&self, implementation: &Object);
}

/// Answers a constrained want's questions about one registration, offered when the service is registered. It is
/// called by the one thread dispatching, outside every lock of the crate's; a panic goes to the error sink and leaves
/// the question unanswered, which the core reads as not served for now.
pub trait CapacityAnswerer: Send + Sync {
    /// What the provider offers now, laid by the core over what its manifest advertises, name by name.
    fn attributes(&self) -> Vec<Number>;
    /// Whether the provider serves a consumer under its constraints.
    fn serves(&self, constraints: &[Number]) -> bool;
}

/// What a plugin held was withdrawn, so it must stop using the object.
#[derive(Clone)]
pub struct Revocation {
    /// The object it held.
    pub object: Object,
    /// The registration or implementation it held.
    pub registration: Handle,
    /// Whether the registration was made awaiting releases, so the core keeps the holding until the plugin hands this
    /// revocation to [`crate::context::Context::release_holding`], once it no longer uses the object nor anything it
    /// answered.
    pub awaits_release: bool,
}

/// An address a provider in another binary or language registered as its object: a C library's definition, or the
/// object a plugin across the plugin seam registered. Identity is the address, so two of them name one provider's
/// object exactly when their addresses are equal. Reading through it is the reader's `unsafe`, as a C plugin's cast
/// is, and it is valid only while what answered it keeps it so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ForeignObject {
    address: usize,
}

impl ForeignObject {
    /// The object at `address`.
    pub fn at(address: usize) -> ForeignObject {
        ForeignObject { address }
    }

    /// Its address.
    pub fn address(&self) -> usize {
        self.address
    }
}

/// The address that stands for `object` where it crosses to another binary: a [`ForeignObject`]'s own address, and
/// for any other object the address of the provider's own value, which is the same for every clone of it.
pub fn address_of(object: &Object) -> usize {
    match object.downcast_ref::<ForeignObject>() {
        Some(foreign) => foreign.address(),
        None => Arc::as_ptr(object) as *const () as usize,
    }
}

/// What the core reports about a plugin.
#[derive(Clone)]
pub enum PluginEvent {
    /// What the plugin held was withdrawn, so it must stop using it.
    Revoked(Revocation),
    /// Nothing holds what this plugin registered awaiting releases under `id` any more, so whatever stood behind the
    /// object may be freed.
    HoldingsReleased {
        /// The id it was registered under.
        id: String,
        /// The object it registered.
        service: Object,
    },
    /// The plugin reached for what it may not.
    ReachRefused {
        /// What it reached for.
        kind: ReachKind,
        /// The id it reached for.
        id: String,
        /// Why it may not.
        refusal: Refusal,
    },
    /// A constrained want settled with a provider that met its constraints only in part.
    Degraded {
        /// The service id it wanted.
        id: String,
        /// The provider's object it settled with.
        service: Object,
        /// The constraint left unmet, as the core reported it.
        unmet_constraint: Refusal,
    },
}

/// Hears what the core reports about a plugin.
pub type PluginListener = Arc<dyn Fn(&PluginEvent) + Send + Sync>;

/// A want's settlement: the provider's object, or why not.
pub type WantSettlement = Result<Object, Answer>;

pub(crate) struct WantRecord {
    pub(crate) id: String,
    pub(crate) settle: Mutex<Option<Settle<WantSettlement>>>,
    pub(crate) holding_room: Mutex<Option<Reservation<Record, Object>>>,
}

impl WantRecord {
    /// Settles the want once; a second settlement finds nothing to settle.
    pub(crate) fn settle(&self, settlement: WantSettlement) {
        let settle = self
            .settle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(settle) = settle {
            settle.settle(settlement);
        }
    }

    pub(crate) fn take_holding_room(&self) -> Option<Reservation<Record, Object>> {
        self.holding_room
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

/// What the table keeps for one handle.
#[derive(Clone)]
pub(crate) enum Record {
    Plugin(Context),
    Service {
        id: String,
        object: Object,
        answerer: Option<Arc<dyn CapacityAnswerer>>,
    },
    Implementation {
        object: Object,
    },
    ServiceWatch(Arc<dyn ServiceWatcher>),
    CapabilityWatch(Arc<dyn CapabilityWatcher>),
    Want(Arc<WantRecord>),
}

pub(crate) type Table = Handles<Record, Object>;

/// What runs once nothing holds a registration made awaiting releases.
pub(crate) type Released = Box<dyn FnOnce() + Send>;

/// A registration made awaiting releases, kept from the core's answer until its holdings-released event is dispatched.
pub(crate) struct Awaiting {
    pub(crate) id: String,
    pub(crate) service: Object,
    pub(crate) released: Option<Released>,
}

/// The registrations made awaiting releases, and each revoked holding of one, kept from its revocation until its holder
/// lets go of the object or the holder's release, quarantine or forget ends it in the core.
#[derive(Default)]
pub(crate) struct Releases {
    awaiting: Mutex<HashMap<u64, Awaiting>>,
    revoked: Mutex<HashMap<(u64, u64), Object>>,
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Releases {
    pub(crate) fn await_releases(&self, registration: Handle, awaiting: Awaiting) {
        locked(&self.awaiting).insert(registration.0, awaiting);
    }

    pub(crate) fn awaits(&self, registration: Handle) -> bool {
        locked(&self.awaiting).contains_key(&registration.0)
    }

    pub(crate) fn released(&self, registration: Handle) -> Option<Awaiting> {
        locked(&self.awaiting).remove(&registration.0)
    }

    pub(crate) fn keep_revoked(&self, holder: Handle, registration: Handle, object: Object) {
        locked(&self.revoked).insert((holder.0, registration.0), object);
    }

    /// Takes the revoked holding `holder` keeps of `registration`, once, and only of that very object.
    pub(crate) fn take_revoked(
        &self,
        holder: Handle,
        registration: Handle,
        object: &Object,
    ) -> bool {
        let mut revoked = locked(&self.revoked);
        let key = (holder.0, registration.0);
        if revoked
            .get(&key)
            .is_some_and(|kept| Arc::ptr_eq(kept, object))
        {
            revoked.remove(&key);
            return true;
        }
        false
    }

    /// Drops every revoked holding of `holder`, which the core ended with the holder's release, quarantine or forget.
    pub(crate) fn drop_revoked_of(&self, holder: Handle) {
        locked(&self.revoked).retain(|(kept_by, _), _| *kept_by != holder.0);
    }

    /// Takes every registration still awaiting, once the core is destroyed and no event of theirs is left to come.
    pub(crate) fn take_every_awaiting(&self) -> Vec<Awaiting> {
        locked(&self.awaiting)
            .drain()
            .map(|(_, awaiting)| awaiting)
            .collect()
    }
}
