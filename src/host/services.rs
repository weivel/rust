//! Host services: a service of the host's own under `WEIVEL_HOST`, offered to an activation plan as provided by the
//! host.

use std::sync::{Arc, Weak};

use super::state::{Host, HostShared, change_count};
use crate::context::{Object, ServiceRegistration};
use crate::core::Answer;

/// A service the host registered of its own, until it is unregistered or the host destroyed.
pub struct HostService {
    id: String,
    registration: ServiceRegistration,
    host: Weak<HostShared>,
}

impl std::fmt::Debug for HostService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "HostService({})", self.id)
    }
}

impl HostService {
    /// The id it is registered under.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Unregisters it, which revokes it to every holder.
    pub fn unregister(&self) -> Result<(), Answer> {
        let Some(shared) = self.host.upgrade() else {
            return Err(Answer::Destroyed);
        };
        self.registration
            .unregister_then(|| change_count(&mut shared.records().host_services, &self.id, false))
    }
}

impl Host {
    /// Registers a service the host itself supplies under `WEIVEL_HOST`: an activation plan counts it as provided by
    /// the host, and a consumer resolving it receives `service` itself. Answers the registration; [`Answer::Mistake`]
    /// for an id a C library the host loaded declares; the core's refusal; [`Answer::Destroyed`] once the destroy has
    /// been called.
    pub fn register_host_service(&self, id: &str, service: Object) -> Result<HostService, Answer> {
        if self.shared.is_destroy_called() {
            return Err(Answer::Destroyed);
        }
        let mut claimed = false;
        let shared = &self.shared;
        let registered = shared.contexts.register_host_service(
            id,
            service,
            || {
                let mut records = shared.records();
                if records.libraries.iter().any(|library| library.declares(id)) {
                    return Err(Answer::Mistake(format!(
                        "the service id {id} is declared by a C library the host has loaded"
                    )));
                }
                change_count(&mut records.host_service_claims, id, true);
                claimed = true;
                Ok(())
            },
            || change_count(&mut shared.records().host_services, id, true),
        );
        if claimed {
            change_count(&mut shared.records().host_service_claims, id, false);
        }
        Ok(HostService {
            id: id.to_owned(),
            registration: registered?,
            host: Arc::downgrade(shared),
        })
    }
}
