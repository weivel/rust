//! The host's log, which a plugin writes to under its own name and the core's own log events reach at their level.

use super::link::Link;
use super::state::ContextState;
use crate::core::{FailureKind, LogLevel};

/// One line of the host's log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRecord {
    /// How much it matters.
    pub level: LogLevel,
    /// The import name of the plugin that wrote it; `None` for the core's own.
    pub plugin: Option<String>,
    /// What it says.
    pub text: String,
}

/// Where the host's log goes, the host's to direct. What it panics with goes to the error sink.
pub trait HostLog: Send + Sync {
    /// Writes one record.
    fn write(&self, record: &LogRecord);
}

/// A plugin's log, writing to the host's under the plugin's import name.
#[derive(Clone)]
pub struct PluginLog {
    pub(crate) link: Link,
    pub(crate) name: String,
}

impl PluginLog {
    /// Writes `text` at `level`.
    pub fn write(&self, level: LogLevel, text: impl Into<String>) {
        self.link.write(&LogRecord {
            level,
            plugin: Some(self.name.clone()),
            text: text.into(),
        });
    }
}

impl ContextState {
    pub(crate) fn write(&self, record: &LogRecord) {
        let log = &self.host_log;
        self.call_back(
            || log.write(record),
            FailureKind::Log,
            "the host's log panicked",
            None,
        );
    }
}
