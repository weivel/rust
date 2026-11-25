//! A plugin's context: what it sees of the core, and the only way it reaches any of it.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::log::PluginLog;
use super::records::{PluginEvent, PluginListener};
use super::state::ContextState;
use crate::core::{FailureKind, Handle, Manifest};

pub(crate) struct ContextInner {
    pub(crate) state: Arc<ContextState>,
    pub(crate) plugin: Handle,
    pub(crate) manifest: Manifest,
    made_after: u64,
    listeners: Mutex<Vec<(u64, PluginListener)>>,
    last_listener: AtomicU64,
    token: AtomicU64,
    detached: AtomicBool,
    silenced: AtomicBool,
}

/// What a plugin sees of the core, and the only way it reaches any of it: services to get, want, watch, list and
/// register, capabilities to provide, list and watch, what the core reports about it, its log and its manifest.
///
/// Every call that reaches the core answers the core's own answer, read as it gave it, and
/// [`crate::core::Answer::Detached`] once the core has forgotten the plugin, without reaching the core. A resolved
/// service or implementation is the provider's own object. A context caches no answer and decides nothing the core
/// decides. Any thread may call; a plugin's listeners, watchers and answerers are called by the one thread
/// dispatching, outside every lock of the crate's, and may call in. Cloning it shares the context.
#[derive(Clone)]
pub struct Context {
    pub(crate) inner: Arc<ContextInner>,
}

impl std::fmt::Debug for Context {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Context({})", self.inner.manifest.import_name)
    }
}

/// What stops a listener, returned by [`Context::listen`].
pub struct Listening {
    context: Context,
    listener: u64,
}

impl Listening {
    /// Stops the listener; one being called now finishes, and hears no more.
    pub fn stop(self) {
        self.context
            .inner
            .listeners()
            .retain(|(listener, _)| *listener != self.listener);
    }
}

impl Context {
    pub(crate) fn new(state: Arc<ContextState>, plugin: Handle, manifest: Manifest) -> Context {
        let made_after = state.core.last_sequence();
        Context {
            inner: Arc::new(ContextInner {
                state,
                plugin,
                manifest,
                made_after,
                listeners: Mutex::new(Vec::new()),
                last_listener: AtomicU64::new(0),
                token: AtomicU64::new(0),
                detached: AtomicBool::new(false),
                silenced: AtomicBool::new(false),
            }),
        }
    }

    /// The plugin's handle, as the core answered it at registration.
    pub fn plugin(&self) -> Handle {
        self.inner.plugin
    }

    /// The plugin's manifest value, as the host registered it.
    pub fn manifest(&self) -> &Manifest {
        &self.inner.manifest
    }

    /// The plugin's log, which the host directs.
    pub fn log(&self) -> PluginLog {
        PluginLog {
            state: self.inner.state.clone(),
            name: self.inner.manifest.import_name.clone(),
        }
    }

    /// Listens to what the core reports about this plugin, behind the listeners added before.
    pub fn listen(&self, listener: impl Fn(&PluginEvent) + Send + Sync + 'static) -> Listening {
        let registration = self.inner.last_listener.fetch_add(1, Ordering::Relaxed) + 1;
        self.inner
            .listeners()
            .push((registration, Arc::new(listener)));
        Listening {
            context: self.clone(),
            listener: registration,
        }
    }

    /// Whether the core has forgotten the plugin, so every call through this context answers detached.
    pub fn is_detached(&self) -> bool {
        self.inner.is_detached()
    }

    pub(crate) fn token(&self) -> u64 {
        self.inner.token.load(Ordering::Acquire)
    }

    pub(crate) fn set_token(&self, token: u64) {
        self.inner.token.store(token, Ordering::Release);
    }

    /// Detaches the context; called under the layer lock, in the hold of the forget.
    pub(crate) fn detach(&self) {
        self.inner.detached.store(true, Ordering::Release);
    }

    /// Whether an event of `sequence` may reach the plugin: one drained after this context was made, dispatched
    /// while the plugin is not silenced by its release or quarantine.
    pub(crate) fn hears(&self, sequence: u64) -> bool {
        sequence > self.inner.made_after && !self.inner.silenced.load(Ordering::Acquire)
    }

    pub(crate) fn silence(&self) {
        self.inner.silenced.store(true, Ordering::Release);
    }

    pub(crate) fn unsilence(&self) {
        self.inner.silenced.store(false, Ordering::Release);
    }

    pub(crate) fn tell(&self, event: &PluginEvent) {
        let listeners: Vec<PluginListener> = self
            .inner
            .listeners()
            .iter()
            .map(|(_, listener)| listener.clone())
            .collect();
        for listener in listeners {
            self.inner.state.call_back(
                || listener(event),
                FailureKind::PluginCallback,
                "a plugin's listener panicked",
                None,
            );
        }
    }

    pub(crate) fn drop_listeners(&self) {
        let dropped = std::mem::take(&mut *self.inner.listeners());
        drop(dropped);
    }
}

impl ContextInner {
    fn listeners(&self) -> MutexGuard<'_, Vec<(u64, PluginListener)>> {
        self.listeners
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn is_detached(&self) -> bool {
        self.detached.load(Ordering::Acquire)
    }
}
