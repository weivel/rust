//! The table itself: entries by handle and by token, holdings by the pair of a plugin and what it holds, and the room
//! reservations take before the core is asked.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::core::{Answer, Handle, RefusalStatus};

/// What a handle stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandleKind {
    /// A plugin.
    Plugin,
    /// A service registration.
    ServiceRegistration,
    /// An implementation of a capability.
    CapabilityImplementation,
    /// A watch of a service or a capability.
    Watch,
    /// A want of a service.
    Want,
}

/// One entry: what a handle stands for, the plugin that owns it, the token it was answered with, and its record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry<R> {
    /// The handle the core answered.
    pub handle: Handle,
    /// What it stands for.
    pub kind: HandleKind,
    /// The plugin that owns it, `None` for a plugin's own entry.
    pub owner: Option<Handle>,
    /// A token that leads back to this entry without a search, never reissued.
    pub token: u64,
    /// The caller's record.
    pub record: R,
}

struct Table<R, H> {
    entries: HashMap<u64, Entry<R>>,
    tokens: HashMap<u64, u64>,
    holdings: HashMap<(u64, u64), H>,
    reserved: usize,
    last_token: u64,
}

impl<R, H> Table<R, H> {
    fn take_room(&mut self) -> Result<(), Answer> {
        let wanted = self.reserved + 1;
        let out_of_memory = |_| Answer::Refused {
            status: RefusalStatus::OutOfMemory,
            refusal: None,
        };
        self.entries.try_reserve(wanted).map_err(out_of_memory)?;
        self.tokens.try_reserve(wanted).map_err(out_of_memory)?;
        self.holdings.try_reserve(wanted).map_err(out_of_memory)?;
        self.reserved = wanted;
        Ok(())
    }

    fn remove(&mut self, handle: u64) -> Option<Entry<R>> {
        let ended = self.entries.remove(&handle)?;
        self.tokens.remove(&ended.token);
        Some(ended)
    }
}

/// The table, shared by every clone. Any thread may call; its lock is held only inside its own methods, never while
/// anything outside it runs, so it may be taken under the layer lock and is always the innermost.
pub struct Handles<R, H> {
    table: Arc<Mutex<Table<R, H>>>,
}

impl<R, H> Clone for Handles<R, H> {
    fn clone(&self) -> Self {
        Handles {
            table: self.table.clone(),
        }
    }
}

impl<R, H> Default for Handles<R, H> {
    fn default() -> Self {
        Handles {
            table: Arc::new(Mutex::new(Table {
                entries: HashMap::new(),
                tokens: HashMap::new(),
                holdings: HashMap::new(),
                reserved: 0,
                last_token: 0,
            })),
        }
    }
}

impl<R: Clone, H: Clone> Handles<R, H> {
    fn lock(&self) -> MutexGuard<'_, Table<R, H>> {
        self.table
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Takes the room one later addition needs, an entry or a holding, so making it cannot fail. It is what a caller
    /// does before it asks the core for a handle; a reservation left unused gives its room back when dropped.
    pub fn reserve(&self) -> Result<Reservation<R, H>, Answer> {
        self.lock().take_room()?;
        Ok(Reservation {
            handles: self.clone(),
            used: false,
        })
    }

    /// The entry of `handle`; `None` for one the table does not hold, ended or never made.
    pub fn find(&self, handle: Handle) -> Option<Entry<R>> {
        self.lock().entries.get(&handle.0).cloned()
    }

    /// The entry `token` was answered for; `None` once it ended, or for a token this table never answered.
    pub fn find_by_token(&self, token: u64) -> Option<Entry<R>> {
        let table = self.lock();
        let handle = table.tokens.get(&token)?;
        table.entries.get(handle).cloned()
    }

    /// Ends the entry of `handle`, answering it as it was.
    pub fn end(&self, handle: Handle) -> Option<Entry<R>> {
        self.lock().remove(handle.0)
    }

    /// Ends every entry `owner` owns whose kind is one of `kinds`, answering them in no particular order.
    pub fn end_owned(&self, owner: Handle, kinds: &[HandleKind]) -> Vec<Entry<R>> {
        let mut table = self.lock();
        let ended: Vec<u64> = table
            .entries
            .values()
            .filter(|entry| entry.owner == Some(owner) && kinds.contains(&entry.kind))
            .map(|entry| entry.handle.0)
            .collect();
        ended
            .into_iter()
            .filter_map(|handle| table.remove(handle))
            .collect()
    }

    /// The first entry `matches` accepts, found by walking every entry, so for a rare question.
    pub fn find_first(&self, matches: impl Fn(&Entry<R>) -> bool) -> Option<Entry<R>>
    where
        R: Clone,
    {
        self.lock()
            .entries
            .values()
            .find(|entry| matches(entry))
            .cloned()
    }

    /// Whether an entry `matches` accepts is held, read by walking every entry, so for a rare question.
    pub fn holds_any(&self, matches: impl Fn(&Entry<R>) -> bool) -> bool {
        self.lock().entries.values().any(matches)
    }

    /// The object `plugin` was handed for `implementation`.
    pub fn held(&self, plugin: Handle, implementation: Handle) -> Option<H> {
        self.lock()
            .holdings
            .get(&(plugin.0, implementation.0))
            .cloned()
    }

    /// Drops the holding a revocation names, answering the object; `plugin` may name a plugin already forgotten.
    pub fn revoke(&self, plugin: Handle, implementation: Handle) -> Option<H> {
        self.lock().holdings.remove(&(plugin.0, implementation.0))
    }

    /// Drops every holding of `plugin`.
    pub fn drop_holdings(&self, plugin: Handle) {
        self.lock()
            .holdings
            .retain(|(holder, _), _| *holder != plugin.0);
    }

    /// Ends every entry and holding, answering the entries.
    pub fn clear(&self) -> Vec<Entry<R>> {
        let mut table = self.lock();
        table.tokens.clear();
        table.holdings.clear();
        table.entries.drain().map(|(_, entry)| entry).collect()
    }

    /// How many entries the table holds.
    pub fn entry_count(&self) -> usize {
        self.lock().entries.len()
    }

    /// How many holdings the table holds.
    pub fn holding_count(&self) -> usize {
        self.lock().holdings.len()
    }
}

/// Room for one addition, taken before the core was asked; used once by [`Reservation::add`] or
/// [`Reservation::hold`], or given back when dropped.
pub struct Reservation<R: Clone, H: Clone> {
    handles: Handles<R, H>,
    used: bool,
}

impl<R: Clone, H: Clone> Reservation<R, H> {
    /// Makes the entry of a handle the core answered, in the room reserved, answering it with its token.
    pub fn add(
        mut self,
        kind: HandleKind,
        handle: Handle,
        owner: Option<Handle>,
        record: R,
    ) -> Entry<R> {
        let mut table = self.handles.lock();
        table.reserved -= 1;
        self.used = true;
        table.last_token += 1;
        let entry = Entry {
            handle,
            kind,
            owner,
            token: table.last_token,
            record,
        };
        if let Some(replaced) = table.entries.insert(handle.0, entry.clone()) {
            table.tokens.remove(&replaced.token);
        }
        table.tokens.insert(entry.token, handle.0);
        entry
    }

    /// Keeps what `plugin` was handed for `implementation`, in the room reserved.
    pub fn hold(mut self, plugin: Handle, implementation: Handle, object: H) {
        let mut table = self.handles.lock();
        table.reserved -= 1;
        self.used = true;
        table.holdings.insert((plugin.0, implementation.0), object);
    }
}

impl<R: Clone, H: Clone> Drop for Reservation<R, H> {
    fn drop(&mut self) {
        if !self.used {
            self.handles.lock().reserved -= 1;
        }
    }
}
