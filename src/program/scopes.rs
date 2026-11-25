//! The scopes a run's plugins are given, from whichever loaded library serves `weivel.scope`, or none.

use std::sync::Arc;

use super::options::ProgramLog;
use crate::context::Object;
use crate::core::Handle;
use crate::host::{BoundDefinition, Host, LoadedLibrary, PluginOffer};
use crate::standard::{Scope, Scopes, StandardServiceIds};

/// A plugin's scope once opened: its definitions offered to it alone, until [`OpenedScope::release`] withdraws them
/// and frees it.
#[derive(Default)]
pub(super) struct OpenedScope {
    offers: Vec<PluginOffer>,
    scope: Option<Scope>,
}

impl OpenedScope {
    /// Withdraws every offer, then frees the scope, each whatever became of the one before, answering every failure.
    pub(super) fn release(&mut self) -> Vec<String> {
        let failures = self
            .offers
            .drain(..)
            .filter_map(|offer| {
                offer
                    .withdraw()
                    .err()
                    .map(|answer| format!("{} could not be withdrawn: {answer}", offer.id()))
            })
            .collect();
        if let Some(scope) = self.scope.take() {
            scope.release();
        }
        failures
    }
}

/// What opening a plugin's scope came to.
pub(super) enum ScopeOpening {
    Opened(OpenedScope),
    /// The provider refused it, or its definitions could not be offered, saying why.
    Refused(String),
}

pub(super) struct PluginScopes {
    host: Host,
    ids: StandardServiceIds,
    provider: Option<Scopes>,
    log: ProgramLog,
}

impl PluginScopes {
    pub(super) fn new(host: Host, ids: StandardServiceIds, log: ProgramLog) -> PluginScopes {
        PluginScopes {
            host,
            ids,
            provider: None,
            log,
        }
    }

    /// Asks every scope of the first of `libraries` that serves a described `weivel.scope`.
    pub(super) fn ask_of(&mut self, libraries: &[LoadedLibrary]) {
        self.provider = libraries
            .iter()
            .find_map(|library| library.described(&self.ids.scope))
            .map(Scopes::of);
    }

    /// Opens the scope of `plugin`, whose manifest the plan names `manifest`, and offers each definition it answers
    /// to that plugin alone, under the standard's id for it; with no provider, it offers nothing. A scope refused, or
    /// one whose definitions cannot be offered, as when a loaded library declares one of the ids, answers why; one
    /// offered in part has every offer withdrawn and is freed again before it answers.
    pub(super) fn open(&self, plugin: Handle, manifest: &str) -> ScopeOpening {
        let Some(provider) = &self.provider else {
            return ScopeOpening::Opened(OpenedScope::default());
        };
        let scope = match provider.open(manifest) {
            Ok(Some(scope)) => scope,
            Ok(None) => return ScopeOpening::Opened(OpenedScope::default()),
            Err(refused) => {
                return ScopeOpening::Refused(
                    refused
                        .detail
                        .clone()
                        .unwrap_or_else(|| refused.to_string()),
                );
            }
        };
        self.offer(plugin, scope)
    }

    fn offer(&self, plugin: Handle, scope: Scope) -> ScopeOpening {
        let members: [(&str, Option<BoundDefinition>); 4] = [
            (&self.ids.config, scope.config.clone()),
            (&self.ids.secrets, scope.secrets.clone()),
            (&self.ids.paths, scope.paths.clone()),
            (&self.ids.log, scope.log.clone()),
        ];
        let mut offers = Vec::new();
        for (id, definition) in members {
            let Some(definition) = definition else {
                continue;
            };
            let object: Object = Arc::new(definition);
            match self.host.offer_to_plugin(plugin, id, object) {
                Ok(offer) => offers.push(offer),
                Err(answer) => {
                    let mut partial = OpenedScope {
                        offers,
                        scope: Some(scope),
                    };
                    for failure in partial.release() {
                        self.log.error(format!(
                            "A scope that could not be offered could not be released either: {failure}"
                        ));
                    }
                    return ScopeOpening::Refused(format!(
                        "its scope could not be offered: {answer}"
                    ));
                }
            }
        }
        ScopeOpening::Opened(OpenedScope {
            offers,
            scope: Some(scope),
        })
    }
}
