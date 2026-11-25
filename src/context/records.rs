//! What a plugin hands the crate and hears from it, and the records the handles table keeps for each handle.

use std::any::Any;
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

/// What the core reports about a plugin.
#[derive(Clone)]
pub enum PluginEvent {
    /// What the plugin held of `registration` was withdrawn, so it must stop using `object`.
    Revoked {
        /// The object it held.
        object: Object,
        /// The registration or implementation it held.
        registration: Handle,
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
