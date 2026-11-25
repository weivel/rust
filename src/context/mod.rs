//! What a plugin sees of the core, as the core's `docs/contract/context.md` asks of a language's `context` module.
//!
//! [`Context`] gets, wants, watches, lists and registers services, provides, lists and watches capabilities,
//! listens to what the core reports about its plugin, writes to the host's log under the plugin's name and reads its
//! manifest. A resolved service or implementation is the provider's own [`Object`]; a want's settlement is a
//! [`Pending`], settled once from a dispatch step with the core's answer. What a plugin's listener, watcher or
//! answerer panics with goes to the error sink. [`Contexts`] is the surface a host builds them through.

mod capabilities;
mod contexts;
mod log;
mod pending;
mod plugin;
mod records;
mod routing;
mod services;
mod state;

pub use contexts::{Contexts, Verification};
pub use log::{HostLog, LogRecord, PluginLog};
pub use pending::{Pending, Settle, pending};
pub use plugin::{Context, Listening};
pub use records::{
    CapabilityWatcher, CapacityAnswerer, Object, PluginEvent, PluginListener, ServiceWatcher,
    WantSettlement,
};
pub use services::{ServiceRegistration, Want, WantLimits, Watching};
