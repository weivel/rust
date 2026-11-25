//! The standard's defaults, reached through the standard's own binary, which its caller names by path, as the core's
//! `docs/contract/standard.md` asks of a language's `standard` module.
//!
//! [`Standard`] binds the standard's manifest loader, its library declaration reader and its service ids, with its
//! JSON reader, [`StandardJson`], which reads a document into a [`Json`] value of the crate's own so the crate reads
//! JSON without a parser of its own, and its description checks, [`StandardDescriptions`], the host's description
//! check. [`Config`], [`Secrets`], [`Paths`] and [`Log`] are typed access to the four services of `services.h`, and
//! [`Scopes`] and [`HostProvider`] to `scope.h` and `host.h`, each a wrapper over the definition its description
//! bound. [`declarations`] lists the layout of every struct of the standard's headers the crate declares by hand, for
//! a test to hold to the compiler. Nothing here is used by the core's wrapper or the contexts: a JSON document reaches
//! them only as the values the standard read.

mod binding;
#[doc(hidden)]
pub mod declarations;
mod description;
mod json;
#[forbid(unsafe_code)]
mod services;

pub use binding::{Standard, StandardRefused, StandardServiceIds};
pub use description::StandardDescriptions;
pub use json::{Json, JsonRefusal, StandardJson};
pub use services::{
    ActionEnding, Config, HostOutcome, HostPlan, HostProvider, Log, LogLevel, NextTick, Paths,
    PlannedLibrary, PlannedPlugin, Scope, Scopes, Secrets, Setting, host_modes, plugin_states,
};
