//! Services the host offers each plugin on its own: one registration under `WEIVEL_HOST` per id, each plugin reaching
//! it answered the object offered to it, and a plugin offered nothing under the id, nothing.

use std::collections::BTreeMap;
use std::sync::{Arc, Weak};

use super::services::HostService;
use super::state::{Host, HostShared};
use crate::context::{Object, OfferedToEach};
use crate::core::{Answer, Handle};

/// The registration standing for one id offered to plugins one by one, and what each was offered.
pub(super) struct Offered {
    registration: HostService,
    each: Arc<OfferedToEach>,
}

/// A service the host offered one plugin alone, until it is withdrawn or the host destroyed.
#[derive(Debug)]
pub struct PluginOffer {
    id: String,
    plugin: Handle,
    host: Weak<HostShared>,
}

fn offers_of(each: &OfferedToEach) -> std::sync::MutexGuard<'_, BTreeMap<Handle, Object>> {
    each.offers
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl PluginOffer {
    /// The id it is offered under.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Withdraws it, so the plugin reaches nothing under the id from now on; the last offer of an id withdrawn
    /// unregisters the id's registration, which revokes it to every holder. Answers true; false for an offer withdrawn
    /// already; [`Answer::Destroyed`] once the host is gone.
    pub fn withdraw(&self) -> Result<bool, Answer> {
        let shared = self.host.upgrade().ok_or(Answer::Destroyed)?;
        let mut offered = shared.offered();
        let Some(standing) = offered.get(&self.id) else {
            return Ok(false);
        };
        let withdrawn = offers_of(&standing.each).remove(&self.plugin).is_some();
        if offers_of(&standing.each).is_empty() {
            let standing = offered
                .remove(&self.id)
                .expect("the id's offers were just read");
            standing.registration.unregister()?;
        }
        Ok(withdrawn)
    }
}

impl Host {
    /// Offers `service` under `id` to the registered plugin `plugin` alone. The core decides reach as for any host
    /// service: every plugin offered `id` shares one registration of the host's, which an activation plan counts as
    /// provided by the host, and each plugin that resolves it is answered the object offered to it, a plugin offered
    /// nothing under `id` nothing. Answers the offer; [`Answer::Mistake`] for a plugin the host does not hold, an id a
    /// C library the host loaded declares or the host registered for every plugin, or one offered to `plugin`
    /// already; the core's refusal of the registration; [`Answer::Destroyed`] once the destroy has been called.
    pub fn offer_to_plugin(
        &self,
        plugin: Handle,
        id: &str,
        service: Object,
    ) -> Result<PluginOffer, Answer> {
        if self.shared.is_destroy_called() {
            return Err(Answer::Destroyed);
        }
        if !self.shared.records().plugins.contains_key(&plugin) {
            return Err(Answer::Mistake(format!(
                "plugin {} is not one this host holds, so nothing can be offered to it",
                plugin.0
            )));
        }
        let mut offered = self.shared.offered();
        match offered.get(id) {
            Some(standing) => {
                let mut offers = offers_of(&standing.each);
                if offers.contains_key(&plugin) {
                    return Err(Answer::Mistake(format!(
                        "{id} is offered to plugin {} already",
                        plugin.0
                    )));
                }
                offers.insert(plugin, service);
            }
            None => {
                if self.shared.records().host_services.contains_key(id) {
                    return Err(Answer::Mistake(format!(
                        "{id} is registered by the host for every plugin, so it cannot be offered to one alone"
                    )));
                }
                let each = Arc::new(OfferedToEach::default());
                offers_of(&each).insert(plugin, service);
                let registration = self.register_own_service(id, each.clone())?;
                offered.insert(id.to_owned(), Offered { registration, each });
            }
        }
        Ok(PluginOffer {
            id: id.to_owned(),
            plugin,
            host: Arc::downgrade(&self.shared),
        })
    }

    /// The object the plugin `plugin` itself registered under `id` and still has registered, or `None`: a
    /// registration under the id by another plugin or by the host is never answered.
    pub fn service_registered_by(&self, plugin: Handle, id: &str) -> Option<Object> {
        if self.shared.is_destroy_called() {
            return None;
        }
        self.shared.contexts.registered_by(plugin, id)
    }
}
