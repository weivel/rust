//! A plugin's entry laid out on disk: the library its plan names, loaded across the plugin seam when its plugin is
//! first activated.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use super::entry::{Entry, Finished};
use super::methods::Methods;
use super::state::{Host, HostShared};
use crate::context::{Context, Pending};

/// The entry of a plugin whose library lies at `path`, loaded by `symbol` at its first activate.
struct EntryOnDisk {
    host: Weak<HostShared>,
    path: PathBuf,
    symbol: Option<String>,
    loaded: Mutex<Option<Arc<dyn Entry>>>,
}

impl EntryOnDisk {
    fn loaded(&self) -> Option<Arc<dyn Entry>> {
        self.loaded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// The entry, loaded now unless an activate loaded it already, or why it cannot be.
    fn load(&self) -> Result<Arc<dyn Entry>, String> {
        if let Some(loaded) = self.loaded() {
            return Ok(loaded);
        }
        if !is_a_regular_file(&self.path) {
            return Err(format!(
                "its entry {} is not a regular file reached through no link",
                self.path.display()
            ));
        }
        let shared = self
            .host
            .upgrade()
            .ok_or_else(|| "the host was destroyed before its entry was loaded".to_owned())?;
        let entry = Host { shared }
            .load_entry(&self.path, self.symbol.as_deref())
            .map_err(|answer| {
                format!(
                    "its entry {} could not be loaded: {answer}",
                    self.path.display()
                )
            })?;
        *self
            .loaded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(entry.clone());
        Ok(entry)
    }
}

/// Whether `path` is a regular file and its last step no link, as the standard reads an entry on disk.
fn is_a_regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file())
}

impl Entry for EntryOnDisk {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        match self.load() {
            Ok(entry) => entry.activate(context),
            Err(failure) => Pending::settled(Err(failure)),
        }
    }

    fn deactivate(&self) -> Option<Pending<Finished>> {
        self.loaded()?.deactivate()
    }

    fn actions(&self) -> Methods {
        self.loaded()
            .map(|entry| entry.actions())
            .unwrap_or_default()
    }

    fn forgotten(&self, context: &Context) {
        if let Some(entry) = self.loaded() {
            entry.forgotten(context);
        }
    }
}

impl Host {
    /// The entry of a plugin built as a library of its own at `path`: its first activate loads the library as
    /// [`Host::load_entry`] does, by `symbol` or the default entry symbol, and fails, saying why, for a path that is
    /// not a regular file reached through no link or a library that does not load; its deactivate, its actions and
    /// what it hears are the loaded entry's from then on.
    pub fn entry_on_disk(&self, path: &Path, symbol: Option<&str>) -> Arc<dyn Entry> {
        Arc::new(EntryOnDisk {
            host: Arc::downgrade(&self.shared),
            path: path.to_owned(),
            symbol: symbol.map(str::to_owned),
            loaded: Mutex::new(None),
        })
    }
}
