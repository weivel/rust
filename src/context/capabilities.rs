//! A context's calls about capabilities, each recording what the core answered in the hold of its call.

use std::sync::Arc;

use super::link::Link;
use super::plugin::Context;
use super::records::{CapabilityWatcher, Object, Record, Table};
use super::services::Watching;
use super::state::ContextState;
use crate::core::{Answer, CapabilityImplementation, FailureKind, Handle};
use crate::handles::{HandleKind, Reservation};

const FIRST_LISTING_ROOM: usize = 4;

/// What one listing found, held in room taken before the call.
struct Listing {
    rooms: Vec<Reservation<Record, Object>>,
    found: Vec<Object>,
    unknown: usize,
    short_of: usize,
}

impl Listing {
    fn with_room(handles: &Table, room: usize) -> Result<Listing, Answer> {
        Ok(Listing {
            rooms: (0..room)
                .map(|_| handles.reserve())
                .collect::<Result<_, _>>()?,
            found: Vec::new(),
            unknown: 0,
            short_of: 0,
        })
    }

    fn hold(
        &mut self,
        state: &ContextState,
        plugin: Handle,
        listed: Vec<CapabilityImplementation>,
    ) {
        if listed.len() > self.rooms.len() {
            self.short_of = listed.len();
            return;
        }
        for each in listed {
            let Some(object) = state.object_of_implementation(each.implementation) else {
                self.unknown += 1;
                continue;
            };
            let room = self
                .rooms
                .pop()
                .expect("a room was taken for every listed implementation");
            room.hold(plugin, each.implementation, object.clone());
            self.found.push(object);
        }
    }
}

impl Context {
    /// Provides `implementation` of the capability `id`; it reaches consumers once the plugin's activation is
    /// verified. Answers [`Answer::ContractMismatch`], the core not asked, when a loaded library publishes a contract
    /// for `id` and the implementation does not offer one of its operations, as the host's `docs/host.md` says.
    pub fn provide(&self, id: &str, implementation: Object) -> Result<(), Answer> {
        let state = match &self.inner.link {
            Link::Local(state) => state,
            Link::Seam(seam) => return seam.provide(id, implementation),
        };
        if let Some(operation) = state.missing_operation(id, &implementation) {
            return Err(Answer::ContractMismatch {
                capability: id.to_owned(),
                operation,
            });
        }
        let room = state.handles.reserve()?;
        let plugin = self.plugin();
        state.call(&self.inner, |hold| {
            let provided = hold.provide_capability(plugin, id)?;
            room.add(
                HandleKind::CapabilityImplementation,
                provided,
                Some(plugin),
                Record::Implementation {
                    object: implementation,
                },
            );
            Ok(())
        })
    }

    /// The verified implementations of the capability `id`, each the provider's own object, in the core's order. An
    /// implementation the crate holds no object for is left out and the error sink told.
    pub fn implementations(&self, id: &str) -> Result<Vec<Object>, Answer> {
        let state = match &self.inner.link {
            Link::Local(state) => state,
            Link::Seam(seam) => return seam.implementations(id),
        };
        let plugin = self.plugin();
        let mut room = FIRST_LISTING_ROOM;
        loop {
            let mut listing = Listing::with_room(&state.handles, room)?;
            state.call(&self.inner, |hold| {
                let listed = hold.all_capabilities(plugin, id)?;
                listing.hold(state, plugin, listed);
                Ok(())
            })?;
            if listing.short_of > 0 {
                room = listing.short_of;
                continue;
            }
            if listing.unknown > 0 {
                state.report(
                    FailureKind::ObjectUnknown,
                    "the core listed an implementation the layer holds no object for, which was left out"
                        .to_owned(),
                    None,
                );
            }
            return Ok(listing.found);
        }
    }

    /// Watches the capability `id`: every implementation that joins and the very one that leaves, from now on.
    pub fn watch_capability(
        &self,
        id: &str,
        watcher: Arc<dyn CapabilityWatcher>,
    ) -> Result<Watching, Answer> {
        let state = match &self.inner.link {
            Link::Local(state) => state,
            Link::Seam(seam) => {
                return Ok(Watching::new(
                    self.clone(),
                    seam.watch_capability(id, watcher)?,
                ));
            }
        };
        let room = state.handles.reserve()?;
        let plugin = self.plugin();
        let watch = state.call(&self.inner, |hold| {
            let watch = hold.watch_capability(plugin, id)?;
            room.add(
                HandleKind::Watch,
                watch,
                Some(plugin),
                Record::CapabilityWatch(watcher),
            );
            Ok(watch)
        })?;
        Ok(Watching::new(self.clone(), watch))
    }
}
