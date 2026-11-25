//! The threads a plugin across the seam starts, kept per entry so its `free` returns only after every one has ended:
//! the host may close the plugin's library once `free` has returned, and a thread still running the library's code
//! then would run unmapped code.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use super::locked;

/// The threads one entry started through [`plugin_thread`].
#[derive(Default)]
pub(super) struct PluginThreads {
    handles: Mutex<Vec<JoinHandle<()>>>,
}

thread_local! {
    /// The threads of the entry whose code this thread runs now, while the seam calls into it or a thread it started
    /// runs.
    static CURRENT: RefCell<Option<Arc<PluginThreads>>> = const { RefCell::new(None) };
}

impl PluginThreads {
    /// Runs `run` as a call into this entry, so a thread it starts through [`plugin_thread`] is kept by the entry.
    pub(super) fn within<T>(self: &Arc<Self>, run: impl FnOnce() -> T) -> T {
        let previous = CURRENT.with(|current| current.replace(Some(self.clone())));
        let ran = run();
        CURRENT.with(|current| *current.borrow_mut() = previous);
        ran
    }

    /// Joins every thread the entry started, those they started included, but the calling thread itself.
    pub(super) fn join_all(&self) {
        let calling = std::thread::current().id();
        loop {
            let handles = std::mem::take(&mut *locked(&self.handles));
            if handles.is_empty() {
                return;
            }
            for handle in handles {
                if handle.thread().id() != calling {
                    let _ = handle.join();
                }
            }
        }
    }
}

/// Starts `work` on a thread of its own. Started while the seam calls into a plugin's entry, from its activate, its
/// deactivate, an event, an action or a method, or from a thread started so, the thread is that entry's: the entry's
/// `free` returns only once it has ended, so the host never closes the plugin's library under it. Started anywhere
/// else, as in a plugin running in the host's own process, it is an ordinary thread.
pub fn plugin_thread(work: impl FnOnce() + Send + 'static) -> std::io::Result<()> {
    let owner = CURRENT.with(|current| current.borrow().clone());
    let Some(owner) = owner else {
        std::thread::Builder::new().spawn(work)?;
        return Ok(());
    };
    let inherited = owner.clone();
    let mut handles = locked(&owner.handles);
    let handle = std::thread::Builder::new().spawn(move || inherited.within(work))?;
    handles.push(handle);
    Ok(())
}
