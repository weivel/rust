//! What a run of the host program is started with, and how the program tells what went wrong.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::stop::StopRequest;
use crate::context::{HostLog, LogRecord};
use crate::core::{ErrorSink, LogLevel};

/// The code the program exits with when `start` did not answer OK, or `stop` answered no code.
pub const FAILED_RUN_EXIT: u32 = 1;

/// The name of the declaration a host program reads beside the host library it was started with.
pub const DECLARATION_FILE_NAME: &str = "library.json";

/// What one run of the host program is started with.
#[derive(Clone)]
pub struct RunHostOptions {
    /// The binary of the library that serves `weivel.host`; its declaration is the `library.json` beside it.
    pub host_library: PathBuf,
    /// Handed to the host library's `start`, in order.
    pub arguments: Vec<String>,
    /// The core's shared library.
    pub core_library: PathBuf,
    /// The standard's shared library.
    pub standard_library: PathBuf,
    /// The run's stop request, honoured from the moment `start` answered OK; the same one the answer of
    /// [`super::start_host`] stops.
    pub stop: StopRequest,
    /// Where the program writes what went wrong and the core's and the plugins' logs; nowhere when `None`.
    pub log: Option<Arc<dyn HostLog>>,
    /// Where the host tells the failures of plugins' callbacks; standard error when `None`.
    pub sink: Option<Arc<dyn ErrorSink>>,
    /// The symbol each plugin's library exports its entry under, `weivel_rust_entry` when `None`.
    pub entry_symbol: Option<String>,
}

impl std::fmt::Debug for RunHostOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunHostOptions")
            .field("host_library", &self.host_library)
            .field("arguments", &self.arguments)
            .field("core_library", &self.core_library)
            .field("standard_library", &self.standard_library)
            .field("entry_symbol", &self.entry_symbol)
            .finish_non_exhaustive()
    }
}

impl RunHostOptions {
    /// A run of the host library at `host_library` with no arguments, over the core and the standard at these paths.
    pub fn new(
        host_library: impl Into<PathBuf>,
        core_library: impl Into<PathBuf>,
        standard_library: impl Into<PathBuf>,
    ) -> RunHostOptions {
        RunHostOptions {
            host_library: host_library.into(),
            arguments: Vec::new(),
            core_library: core_library.into(),
            standard_library: standard_library.into(),
            stop: StopRequest::new(),
            log: None,
            sink: None,
            entry_symbol: None,
        }
    }

    /// The declaration the program reads beside its host library.
    pub(super) fn host_declaration(&self) -> PathBuf {
        self.host_library
            .parent()
            .unwrap_or(Path::new(""))
            .join(DECLARATION_FILE_NAME)
    }
}

/// Where the program writes what went wrong, as a record of no plugin's.
#[derive(Clone)]
pub(super) struct ProgramLog {
    log: Option<Arc<dyn HostLog>>,
}

impl ProgramLog {
    pub(super) fn of(options: &RunHostOptions) -> ProgramLog {
        ProgramLog {
            log: options.log.clone(),
        }
    }

    fn write(&self, level: LogLevel, text: String) {
        if let Some(log) = &self.log {
            log.write(&LogRecord {
                level,
                plugin: None,
                text,
            });
        }
    }

    /// A refusal the provider answered, which ends the run as what it is.
    pub(super) fn warn(&self, text: String) {
        self.write(LogLevel::WARNING, text);
    }

    /// What went wrong otherwise.
    pub(super) fn error(&self, text: String) {
        self.write(LogLevel::ERROR, text);
    }
}

/// A text a plugin or the program wrote, with each NUL written out, so a C text carries it whole.
pub(super) fn text_for_c(text: &str) -> String {
    text.replace('\0', "\\0")
}
