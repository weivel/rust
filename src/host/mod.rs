//! Registers plugins written in Rust, activates, verifies and releases them, and destroys them with the core, as the
//! core's `docs/contract/host.md` asks of a language's `host` module.
//!
//! [`Host::register`] registers a plugin from a manifest value and an [`Entry`]; [`Host::activate`] asks the core for
//! the plan and starts each plugin in its order without waiting for one before the next; [`Host::verify`],
//! [`Host::release`] and [`Host::destroy`] follow the contract's order of a verification, a release and a destroy.
//! Host services, C libraries and the plugin seam are not here yet.

mod activation;
mod entry;
mod release;
mod state;

pub use entry::{
    Activation, ActivationState, Entry, Finished, HostOptions, HostScheduler, Outcome,
};
pub use state::Host;
