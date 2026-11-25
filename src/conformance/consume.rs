//! Drives a `consume` vector: its plugins built through the host and opened with no quarantine of the driver's
//! making, then its steps in order, each through the plugin's context or the host.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::observations::{Observer, kit_refusal};
use super::shapes::{self, DriverFailure, Members};
use super::world::{DriverWorld, InFlight, manifest};
use crate::context::{CapabilityWatcher, Object, PluginEvent};
use crate::core::{Answer, EdgeSource, LedgerRow, PluginStatus, Refusal};
use crate::standard::Json;

const WATCH: &str = "watch";
const PROVIDE: &str = "provide";
const VERIFY: &str = "verify";
const RELEASE: &str = "release";

/// The object a plugin provides: the kit names an implementation by its provider.
pub(crate) struct Implementation {
    pub(crate) provider: String,
}

/// A watch that records each change it is handed and, when throwing, panics on that call and every other.
struct RecordingWatch {
    capability: String,
    changes: Arc<Mutex<Vec<Json>>>,
    throwing: bool,
}

impl RecordingWatch {
    fn hear(&self, implementation: &Object, change: &str) {
        let provider = provider_of(implementation);
        self.changes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(Json::Array(vec![
                Json::Text(provider.clone()),
                Json::Text(change.to_owned()),
            ]));
        if self.throwing {
            panic!(
                "the throwing listener of {} was handed {provider} {change}",
                self.capability
            );
        }
    }
}

impl CapabilityWatcher for RecordingWatch {
    fn provided(&self, implementation: &Object) {
        self.hear(implementation, PROVIDE);
    }

    fn withdrawn(&self, implementation: &Object) {
        self.hear(implementation, "withdraw");
    }
}

pub(crate) fn provider_of(implementation: &Object) -> String {
    implementation.downcast_ref::<Implementation>().map_or_else(
        || "an implementation of no driver's".to_owned(),
        |found| found.provider.clone(),
    )
}

#[derive(Default)]
pub(crate) struct ConsumeRun {
    heard: Vec<Arc<Mutex<Vec<Json>>>>,
    reported: Arc<Mutex<Vec<Json>>>,
    refusals: Vec<Refusal>,
}

pub(crate) fn drive(given: &Members, world: &mut DriverWorld) -> Result<ConsumeRun, DriverFailure> {
    let place = "the vector's given";
    shapes::refuse_unknown_keys(place, given, &["plugins", "steps"])?;
    let mut run = ConsumeRun::default();
    run.build(place, &shapes::objects(place, given, "plugins")?, world)?;
    for (index, step) in shapes::objects(place, given, "steps")?.iter().enumerate() {
        run.perform(&format!("{place}'s step {index}"), step, world)?;
    }
    Ok(run)
}

impl ConsumeRun {
    fn build(
        &mut self,
        place: &str,
        plugins: &[&Members],
        world: &mut DriverWorld,
    ) -> Result<(), DriverFailure> {
        let mut names = Vec::new();
        for plugin in plugins {
            let id = shapes::text(place, plugin, "id")?;
            let plugin_place = format!("{place}'s plugin {id}");
            shapes::refuse_unknown_keys(&plugin_place, plugin, &["id", "capabilities"])?;
            let capabilities = shapes::object_at(&plugin_place, plugin, "capabilities")?;
            shapes::refuse_unknown_keys(&plugin_place, capabilities, &["provides", "consumes"])?;
            let provides = shapes::texts(&plugin_place, capabilities, "provides")?;
            let consumes = shapes::texts(&plugin_place, capabilities, "consumes")?;
            world.register(
                manifest(&id, &[], &[], &provides, &consumes),
                Arc::new(InFlight),
            )?;
            names.push(id);
        }
        world.open_in_flight(&names, EdgeSource::SERVICES)?;
        for name in names {
            let reported = self.reported.clone();
            let listener_name = name.clone();
            world.context(&name)?.listen(move |event| {
                if let PluginEvent::ReachRefused { id, .. } = event {
                    reported
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(Json::Array(vec![
                            Json::Text(listener_name.clone()),
                            Json::Text(id.clone()),
                        ]));
                }
            });
        }
        Ok(())
    }

    fn perform(
        &mut self,
        place: &str,
        step: &Members,
        world: &DriverWorld,
    ) -> Result<(), DriverFailure> {
        let action = shapes::the_one_named(place, step, &[WATCH, PROVIDE, VERIFY, RELEASE])?;
        if action == WATCH {
            shapes::refuse_unknown_keys(place, step, &["plugin", WATCH, "throws"])?;
        } else {
            shapes::refuse_unknown_keys(place, step, &["plugin", action])?;
        }
        let plugin = shapes::text(place, step, "plugin")?;
        let answer = match action {
            WATCH => {
                let capability = shapes::text(place, step, WATCH)?;
                let throwing = shapes::flag(place, step, "throws", false)?;
                self.watch(world, &plugin, capability, throwing)
            }
            PROVIDE => {
                let capability = shapes::text(place, step, PROVIDE)?;
                let implementation: Object = Arc::new(Implementation {
                    provider: plugin.clone(),
                });
                world.context(&plugin)?.provide(&capability, implementation)
            }
            VERIFY => {
                shapes::require_true(place, step, VERIFY)?;
                world.host.verify(world.plugin(&plugin)?).map(drop)
            }
            _ => {
                shapes::require_true(place, step, RELEASE)?;
                let release = world.host.release(world.plugin(&plugin)?);
                release
                    .value()
                    .ok_or_else(|| format!("the release of {plugin} left something to run"))?
                    .map(drop)
            }
        };
        self.answered(answer)
    }

    fn watch(
        &mut self,
        world: &DriverWorld,
        plugin: &str,
        capability: String,
        throwing: bool,
    ) -> Result<(), Answer> {
        let changes = Arc::new(Mutex::new(Vec::new()));
        self.heard.push(changes.clone());
        let context = world.context(plugin).map_err(Answer::Mistake)?;
        context
            .watch_capability(
                &capability.clone(),
                Arc::new(RecordingWatch {
                    capability,
                    changes,
                    throwing,
                }),
            )
            .map(drop)
    }

    /// Keeps the core's refusal of a step, and fails the run on an answer no well-made step can have.
    fn answered(&mut self, answer: Result<(), Answer>) -> Result<(), DriverFailure> {
        match answer {
            Ok(()) => Ok(()),
            Err(Answer::Refused {
                refusal: Some(refusal),
                ..
            }) => {
                self.refusals.push(refusal);
                Ok(())
            }
            Err(other) => Err(format!("a step answered {other}")),
        }
    }

    pub(crate) fn observers<'a>(
        &'a self,
        world: &'a DriverWorld,
    ) -> BTreeMap<&'static str, Observer<'a>> {
        let mut observers: BTreeMap<&'static str, Observer<'a>> = BTreeMap::new();
        observers.insert(
            "heard",
            Box::new(|_| {
                Ok(Json::Array(
                    self.heard
                        .iter()
                        .map(|changes| {
                            Json::Array(
                                changes
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                                    .clone(),
                            )
                        })
                        .collect(),
                ))
            }),
        );
        observers.insert(
            "reported",
            Box::new(|_| {
                Ok(Json::Array(
                    self.reported
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .clone(),
                ))
            }),
        );
        observers.insert(
            "refusal",
            Box::new(|_| Ok(self.refusals.first().map_or(Json::Null, kit_refusal))),
        );
        observers.insert(
            "quarantined",
            Box::new(move |_| {
                Ok(Json::Array(
                    quarantined_rows(world)?
                        .into_iter()
                        .map(|row| Json::Text(row.import_name))
                        .collect(),
                ))
            }),
        );
        observers.insert(
            "quarantineReason",
            Box::new(move |_| {
                Ok(quarantined_rows(world)?
                    .first()
                    .filter(|row| row.refusal.code != crate::core::RefusalCode::NONE)
                    .map_or(Json::Null, |row| kit_refusal(&row.refusal)))
            }),
        );
        observers
    }
}

fn quarantined_rows(world: &DriverWorld) -> Result<Vec<LedgerRow>, DriverFailure> {
    let ledger = world
        .host
        .ledger()
        .map_err(|answer| format!("the host answered no ledger: {answer}"))?;
    Ok(ledger
        .into_iter()
        .filter(|row| row.status == PluginStatus::QUARANTINED)
        .collect())
}
