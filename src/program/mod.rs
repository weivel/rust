//! The host program, as the core's `docs/contract/program.md` asks of a language's `program` module: it carries out
//! `weivel/host.h` of the standard and nothing more, every decision the provider's.
//!
//! [`start_host`] starts a run on a thread of its own and answers a [`RunningHost`] once `started` has been told every
//! plugin's outcome, so a caller with work of its own, such as drawing, goes on while the run does; [`run_host`] runs
//! one to its end; [`main`] is the command line of the `weivel-host` binary, `weivel-host <host library>
//! <arguments...>`, each terminate signal of its platform a stop request. A plugin's entry is the library its plan
//! names, built as a library of its own and loaded across the plugin seam by [`RunHostOptions::entry_symbol`],
//! `weivel_rust_entry` by default. A plan naming `action_service` and `action_method` calls the method of that name of
//! the service object its plugin registered, which is a [`crate::host::Methods`]; one naming `action_name` alone, the
//! action of that name its entry offers, [`crate::host::Entry::actions`].

#[forbid(unsafe_code)]
mod command_line;
#[forbid(unsafe_code)]
mod options;
#[forbid(unsafe_code)]
mod plugins;
#[forbid(unsafe_code)]
mod run;
#[forbid(unsafe_code)]
mod scopes;
mod signals;
#[forbid(unsafe_code)]
mod stop;

pub use command_line::{main, process_exit_code};
pub use options::{DECLARATION_FILE_NAME, FAILED_RUN_EXIT, RunHostOptions};
pub use run::{RunningHost, run_host, start_host};
pub use stop::StopRequest;
