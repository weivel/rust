//! Registers plugins written in Rust, activates, verifies and releases them, offers host services, loads C libraries
//! as host services, loads plugins across the plugin seam, and destroys them with the core, as the core's
//! `docs/contract/host.md` asks of a language's `host` module.
//!
//! [`Host::register`] registers a plugin from a manifest value and an [`Entry`]; [`Host::activate`] asks the core for
//! the plan and starts each plugin in its order without waiting for one before the next; [`Host::verify`],
//! [`Host::release`] and [`Host::destroy`] follow the contract's order of a verification, a release and a destroy.
//! [`Host::register_host_service`] offers a service of the host's own, [`Host::load_library`] a C library's services,
//! and [`Host::load_entry`] a plugin built as a library of its own, reached through the plugin seam's C tables.

#[forbid(unsafe_code)]
mod activation;
#[forbid(unsafe_code)]
mod declaration;
#[forbid(unsafe_code)]
mod entry;
#[forbid(unsafe_code)]
mod libraries;
#[forbid(unsafe_code)]
mod release;
mod seam;
#[forbid(unsafe_code)]
mod services;
#[forbid(unsafe_code)]
mod state;

pub use crate::core::{DescribedDefinition, LibraryBytes};
pub use declaration::{
    FunctionKind, LibraryContract, LibraryDeclaration, LibraryFunction, LibraryServiceDeclaration,
};
pub use entry::{
    Activation, ActivationState, DescriptionCheck, Entry, Finished, HostOptions, HostScheduler,
    Outcome,
};
pub use libraries::{LibraryAnswer, LibraryService, LoadedLibrary};
pub use seam::{
    DEFAULT_ENTRY_SYMBOL, SeamCompletion, SeamContext, SeamEntry, SeamEvent, SeamText, seam_entry,
};
pub use services::HostService;
pub use state::Host;
