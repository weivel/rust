//! The plan's plugins as one run activates them: each registered from its manifest, its scope opened, and activated
//! alone, in the plan's order; every one released on the way out, then its scope.

use std::path::Path;
use std::sync::{Arc, Mutex};

use super::options::{ProgramLog, text_for_c};
use super::scopes::{OpenedScope, PluginScopes, ScopeOpening};
use crate::context::Object;
use crate::core::{Answer, EdgeSource, Handle, Refusal};
use crate::host::{ActivationState, Entry, Host, LoadedLibrary, MethodResult, Outcome, methods_of};
use crate::standard::{HostOutcome, PlannedPlugin, Standard, plugin_states};

/// One plugin of the plan the core registered.
struct RunPlugin {
    planned: PlannedPlugin,
    import_name: String,
    handle: Handle,
    entry: Arc<dyn Entry>,
    scope: Option<OpenedScope>,
    activated: bool,
}

/// A detail as an outcome carries it: set and not empty whenever a reason was given.
fn detail_of(reason: Option<&str>) -> Option<String> {
    reason.filter(|reason| !reason.is_empty()).map(text_for_c)
}

fn outcome(planned: &PlannedPlugin, state: u32, detail: Option<&str>) -> HostOutcome {
    HostOutcome {
        manifest: planned.manifest.clone(),
        state,
        detail: detail_of(detail),
    }
}

fn refused(planned: &PlannedPlugin, detail: &str) -> HostOutcome {
    outcome(planned, plugin_states::REFUSED, Some(detail))
}

fn answer_detail(answer: &Answer) -> String {
    match answer {
        Answer::Refused {
            refusal: Some(refusal),
            ..
        } => refusal.detail.clone(),
        other => other.to_string(),
    }
}

/// How an action the plan names is run: the method or action to call, or why there is none.
pub(super) type Planned = Result<Box<dyn Fn(Option<&crate::host::Tree>) -> MethodResult>, String>;

pub(super) struct PluginRun {
    host: Host,
    standard: Standard,
    entry_symbol: Option<String>,
    log: ProgramLog,
    scopes: PluginScopes,
    registered: Mutex<Vec<RunPlugin>>,
}

impl PluginRun {
    /// The plugins of a run whose scopes are asked of the first of `libraries` that serves `weivel.scope`.
    pub(super) fn new(
        host: Host,
        standard: Standard,
        entry_symbol: Option<String>,
        log: ProgramLog,
        libraries: &[LoadedLibrary],
    ) -> PluginRun {
        let mut scopes =
            PluginScopes::new(host.clone(), standard.service_ids().clone(), log.clone());
        scopes.ask_of(libraries);
        PluginRun {
            host,
            standard,
            entry_symbol,
            log,
            scopes,
            registered: Mutex::new(Vec::new()),
        }
    }

    fn registered(&self) -> std::sync::MutexGuard<'_, Vec<RunPlugin>> {
        self.registered
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Activates one plugin of the plan and answers what became of it: its manifest read and checked, then its scope
    /// opened, then its activation, each step only once the one before it passed.
    pub(super) fn activate(&self, planned: &PlannedPlugin) -> HostOutcome {
        let handle = match self.register(planned) {
            Ok(handle) => handle,
            Err(detail) => return refused(planned, &detail),
        };
        let scope = match self.scopes.open(handle, &planned.manifest) {
            ScopeOpening::Opened(scope) => scope,
            ScopeOpening::Refused(detail) => return refused(planned, &detail),
        };
        self.with_plugin(handle, |plugin| plugin.scope = Some(scope));
        let outcome = self.activate_registered(planned, handle);
        if outcome.state == plugin_states::ACTIVATED {
            self.with_plugin(handle, |plugin| plugin.activated = true);
        }
        outcome
    }

    fn with_plugin(&self, handle: Handle, change: impl FnOnce(&mut RunPlugin)) {
        if let Some(plugin) = self
            .registered()
            .iter_mut()
            .find(|plugin| plugin.handle == handle)
        {
            change(plugin);
        }
    }

    fn register(&self, planned: &PlannedPlugin) -> Result<Handle, String> {
        let json = std::fs::read(&planned.manifest).map_err(|error| {
            format!(
                "its manifest {} could not be read: {error}",
                planned.manifest
            )
        })?;
        let manifest = self.standard.load_manifest(&json).map_err(|refused| {
            format!(
                "the standard refused its manifest {}: {refused}",
                planned.manifest
            )
        })?;
        let import_name = manifest.import_name.clone();
        let entry = self
            .host
            .entry_on_disk(Path::new(&planned.entry), self.entry_symbol.as_deref());
        let handle = self
            .host
            .register(manifest, entry.clone())
            .map_err(|answer| {
                format!(
                    "the core refused to register it: {}",
                    answer_detail(&answer)
                )
            })?;
        self.registered().push(RunPlugin {
            planned: planned.clone(),
            import_name,
            handle,
            entry,
            scope: None,
            activated: false,
        });
        Ok(handle)
    }

    fn activate_registered(&self, planned: &PlannedPlugin, handle: Handle) -> HostOutcome {
        let activation = match self.host.activate(&[handle], EdgeSource::CAPABILITIES) {
            Ok(activation) => activation,
            Err(answer) => return refused(planned, &answer_detail(&answer)),
        };
        if let Some(refusal) = activation
            .plan
            .refusals
            .iter()
            .find(|refused| refused.plugin == handle)
        {
            return refused(planned, &refusal.refusal.detail);
        }
        match activation.outcomes.first() {
            Some(outcome) => self.outcome_of(planned, &outcome.wait()),
            None => refused(planned, "the core planned no activation of it"),
        }
    }

    /// A plugin quarantined for what its activate failed with keeps the reason in its ledger row, not its outcome.
    fn outcome_of(&self, planned: &PlannedPlugin, activated: &Outcome) -> HostOutcome {
        match activated.state {
            ActivationState::Active => outcome(planned, plugin_states::ACTIVATED, None),
            ActivationState::Refused => refused(
                planned,
                activated
                    .refusal
                    .as_ref()
                    .map_or("", |refusal| refusal.detail.as_str()),
            ),
            _ => {
                let reason = activated
                    .refusal
                    .as_ref()
                    .map(|refusal| refusal.detail.clone())
                    .filter(|detail| !detail.is_empty())
                    .or_else(|| self.ledger_reason(activated.plugin));
                outcome(planned, plugin_states::FAILED, reason.as_deref())
            }
        }
    }

    fn ledger_reason(&self, plugin: Handle) -> Option<String> {
        let ledger = self.host.ledger().ok()?;
        ledger
            .into_iter()
            .find(|row| row.plugin == plugin)
            .map(|row| row.refusal)
            .map(|refusal: Refusal| refusal.detail)
    }

    fn activated(&self, manifest: &str) -> Option<(Handle, Arc<dyn Entry>)> {
        self.registered()
            .iter()
            .find(|plugin| plugin.planned.manifest == manifest && plugin.activated)
            .map(|plugin| (plugin.handle, plugin.entry.clone()))
    }

    /// The action `name` of the entry of the plugin whose manifest the plan names `manifest`, or why there is none.
    pub(super) fn action_of(&self, manifest: Option<&str>, name: Option<&str>) -> Planned {
        let (Some(manifest), Some(name)) = (manifest, name) else {
            return Err("the plan names no action to run".to_owned());
        };
        let (_, entry) = self
            .activated(manifest)
            .ok_or_else(|| format!("the plugin {manifest} was not activated"))?;
        let actions = entry.actions();
        if !actions.offers(name) {
            return Err(format!("the plugin {manifest} offers no action {name}"));
        }
        let name = name.to_owned();
        Ok(Box::new(move |arguments| {
            actions
                .call(&name, arguments)
                .unwrap_or_else(|| Err(format!("the action {name} is offered no longer")))
        }))
    }

    /// The method `method` of the service object the plugin whose manifest the plan names `manifest` itself
    /// registered under `service`, or why there is none; the entry's actions are never read.
    pub(super) fn method_of(&self, manifest: Option<&str>, service: &str, method: &str) -> Planned {
        let Some(manifest) = manifest else {
            return Err("the plan names no action to run".to_owned());
        };
        let (handle, _) = self
            .activated(manifest)
            .ok_or_else(|| format!("the plugin {manifest} was not activated"))?;
        let object = self
            .host
            .service_registered_by(handle, service)
            .ok_or_else(|| format!("the plugin {manifest} does not provide {service}"))?;
        let methods = methods_of(&object)
            .filter(|methods| methods.offers(method))
            .ok_or_else(|| {
                format!("the service {service} of {manifest} offers no method {method}")
            })?;
        let method = method.to_owned();
        Ok(Box::new(move |arguments| {
            methods
                .call(&method, arguments)
                .unwrap_or_else(|| Err(format!("the method {method} is offered no longer")))
        }))
    }

    /// The object the plugin whose manifest's import name is `import_name` itself registered under `id`.
    pub(super) fn service_of(&self, import_name: &str, id: &str) -> Option<Object> {
        let handle = self
            .registered()
            .iter()
            .find(|plugin| plugin.import_name == import_name)
            .map(|plugin| plugin.handle)?;
        self.host.service_registered_by(handle, id)
    }

    /// Releases every plugin registered, the last first, and then its scope, each whatever became of the one before.
    pub(super) fn release_every(&self) {
        let mut taken = std::mem::take(&mut *self.registered());
        for plugin in taken.iter_mut().rev() {
            if let Err(answer) = self.host.release(plugin.handle).wait() {
                self.log.error(format!(
                    "{} could not be released: {answer}",
                    plugin.planned.manifest
                ));
            }
            if let Some(scope) = plugin.scope.as_mut() {
                for failure in scope.release() {
                    self.log.error(format!(
                        "The scope of {} could not be released: {failure}",
                        plugin.planned.manifest
                    ));
                }
            }
        }
    }
}
