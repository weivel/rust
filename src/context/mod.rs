//! What a plugin sees of the core, as the core's `docs/contract/context.md` asks of a language's `context` module.
//!
//! [`Context`] gets, wants, watches, lists and registers services, provides, lists and watches capabilities,
//! listens to what the core reports about its plugin, writes to the host's log under the plugin's name, reads its
//! manifest, and reads the [`DescribedEnums`] of a definition a description bound. A resolved service or
//! implementation is the provider's own [`Object`]; a want's settlement is a [`Pending`], settled once from a dispatch
//! step with the core's answer. What a plugin's listener, watcher or answerer panics with goes to the error sink. [`Contexts`] is the surface a host builds them through.

mod capabilities;
mod contexts;
mod enums;
mod link;
mod log;
mod pending;
mod plugin;
mod records;
mod routing;
mod services;
mod state;

pub use contexts::{Contexts, Verification};
pub use enums::DescribedEnums;
pub(crate) use link::SeamCalls;
pub use log::{HostLog, LogRecord, PluginLog};
pub(crate) use pending::pending_settled_by;
pub use pending::{Pending, Settle, pending};
pub use plugin::{Context, Listening};
pub(crate) use records::WantRecord;
pub use records::{
    CapabilityWatcher, CapacityAnswerer, ForeignObject, Object, PluginEvent, PluginListener,
    Revocation, ServiceWatcher, WantSettlement, address_of,
};
pub(crate) use services::Registering;
pub use services::{ServiceRegistration, Want, WantLimits, Watching};
