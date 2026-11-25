//! Drives a `lifecycle` vector: each plugin's entry runs the actions the vector gives it and records them, and the
//! host's steps run in order through its public surface. Every call runs on the suite's thread, so once a step
//! returns the crate has nothing left to run but what waits on a want no step can settle.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use super::consume::{Implementation, provider_of};
use super::observations::{self, Observer, code_name};
use super::shapes::{self, DriverFailure, Members, texts_json};
use super::world::{DriverWorld, manifest};
use crate::context::{Context, Object, Pending, ServiceWatcher, WantLimits, pending};
use crate::core::{Answer, EdgeSource, PluginStatus};
use crate::host::{Entry, Finished};
use crate::standard::Json;

const REGISTER: &str = "register";
const WANT: &str = "want";
const GET: &str = "get";
const WATCH: &str = "watch";
const PROVIDE: &str = "provide";
const ALL: &str = "all";
const CAPABILITIES: &str = "capabilities";
const ACTIONS: &[&str] = &[REGISTER, WANT, GET, WATCH, PROVIDE, ALL];

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What every entry of a run records into, and the contexts their activates were handed.
#[derive(Default)]
struct Records {
    trace: Mutex<Vec<String>>,
    pending: Mutex<BTreeSet<String>>,
    contexts: Mutex<HashMap<String, Context>>,
}

impl Records {
    fn trace(&self, record: String) {
        locked(&self.trace).push(record);
    }

    fn context_of(&self, name: &str) -> Context {
        locked(&self.contexts)
            .get(name)
            .cloned()
            .unwrap_or_else(|| panic!("a step names {name}, whose activate was never called"))
    }
}

/// One plugin's entry, running the actions its vector gives it.
struct ActingEntry {
    name: String,
    records: Arc<Records>,
    activate: Arc<Vec<Members>>,
    deactivate: Option<Arc<Vec<Members>>>,
}

impl Entry for ActingEntry {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        self.records.trace(format!("activate {}", self.name));
        locked(&self.records.contexts).insert(self.name.clone(), context.clone());
        run_actions(&self.records, &self.name, context, &self.activate, 0)
    }

    fn deactivate(&self) -> Option<Pending<Finished>> {
        let actions = self.deactivate.as_ref()?;
        self.records.trace(format!("deactivate {}", self.name));
        let context = self.records.context_of(&self.name);
        Some(run_actions(&self.records, &self.name, &context, actions, 0))
    }
}

/// Runs `actions` from `index` on, waiting only where an action waits for its want to end.
fn run_actions(
    records: &Arc<Records>,
    name: &str,
    context: &Context,
    actions: &Arc<Vec<Members>>,
    index: usize,
) -> Pending<Finished> {
    for next in index..actions.len() {
        let action = &actions[next];
        let Some(id) = ACTIONS
            .iter()
            .find_map(|key| action.get(*key).and_then(Json::as_text))
        else {
            return Pending::settled(Err(format!("{name}'s action {next} names no action")));
        };
        if action.contains_key(WANT) {
            let ended = want(records, name, context, id);
            if action.get("await").and_then(Json::as_bool).unwrap_or(true) {
                let (finished, settle) = pending();
                let (records, name, context, actions) = (
                    records.clone(),
                    name.to_owned(),
                    context.clone(),
                    actions.clone(),
                );
                ended.on_settled(move |_| {
                    run_actions(&records, &name, &context, &actions, next + 1)
                        .on_settled(move |rest| settle.settle(rest.clone()));
                });
                return finished;
            }
        } else if let Err(failure) = act(records, name, context, action, id) {
            return Pending::settled(Err(failure));
        }
    }
    Pending::settled(Ok(()))
}

fn act(
    records: &Arc<Records>,
    name: &str,
    context: &Context,
    action: &Members,
    id: &str,
) -> Finished {
    if action.contains_key(REGISTER) {
        let service: Object = Arc::new(());
        context
            .register(id, service)
            .map_err(|answer| format!("{name}'s registration of {id} answered {answer}"))?;
        records.trace(format!("registered {name} {id}"));
    } else if action.contains_key(GET) {
        get(records, name, context, id);
    } else if action.contains_key(WATCH) {
        let watcher = Arc::new(TracingWatch {
            records: records.clone(),
            heard: format!("heard {name} {id}"),
        });
        context
            .watch(id, watcher)
            .map_err(|answer| format!("{name}'s watch of {id} answered {answer}"))?;
    } else if action.contains_key(PROVIDE) {
        let implementation: Object = Arc::new(Implementation {
            provider: name.to_owned(),
        });
        context
            .provide(id, implementation)
            .map_err(|answer| format!("{name}'s provision of {id} answered {answer}"))?;
        records.trace(format!("provided {name} {id}"));
    } else {
        let listed = context
            .implementations(id)
            .map_err(|answer| format!("{name}'s listing of {id} answered {answer}"))?;
        let mut line = format!("listed {name} {id}");
        for implementation in &listed {
            line.push(' ');
            line.push_str(&provider_of(implementation));
        }
        records.trace(line);
    }
    Ok(())
}

/// A watch that traces each registration it hears; an unregistration is not recorded.
struct TracingWatch {
    records: Arc<Records>,
    heard: String,
}

impl ServiceWatcher for TracingWatch {
    fn registered(&self, _: &Object) {
        self.records.trace(self.heard.clone());
    }

    fn unregistered(&self) {}
}

/// Starts a want and answers when it ends, traced as served or rejected with the code the kit writes.
fn want(records: &Arc<Records>, name: &str, context: &Context, id: &str) -> Pending<()> {
    let wanted = format!("{name} {id}");
    locked(&records.pending).insert(wanted.clone());
    let (ended, settle) = pending();
    let records = records.clone();
    context
        .want(id, WantLimits::unbounded())
        .settlement()
        .on_settled(move |settlement| {
            locked(&records.pending).remove(&wanted);
            records.trace(match settlement {
                Ok(_) => format!("settled {wanted}"),
                Err(Answer::Refused {
                    refusal: Some(refusal),
                    ..
                }) => format!("rejected {wanted} {}", code_name(refusal)),
                Err(other) => format!("rejected {wanted} {other}"),
            });
            settle.settle(());
        });
    ended
}

/// Traces a get that answered no object, absent or refused, as missed.
fn get(records: &Records, name: &str, context: &Context, id: &str) {
    let got = if context.get(id).is_ok() {
        "got"
    } else {
        "missed"
    };
    records.trace(format!("{got} {name} {id}"));
}

pub(crate) struct LifecycleRun {
    records: Arc<Records>,
    plans: Vec<Json>,
    verifies: Vec<Json>,
    destroying: bool,
    loud_before_the_destroy: usize,
    thrown_since_the_destroy: usize,
}

pub(crate) fn drive(
    given: &Members,
    world: &mut DriverWorld,
) -> Result<LifecycleRun, DriverFailure> {
    let place = "the vector's given";
    shapes::refuse_unknown_keys(place, given, &["edgeSource", "plugins", "steps"])?;
    let source = edge_source(place, &shapes::text(place, given, "edgeSource")?)?;
    let mut run = LifecycleRun {
        records: Arc::new(Records::default()),
        plans: Vec::new(),
        verifies: Vec::new(),
        destroying: false,
        loud_before_the_destroy: 0,
        thrown_since_the_destroy: 0,
    };
    for plugin in shapes::objects(place, given, "plugins")? {
        run.build(place, plugin, world)?;
    }
    for (index, step) in shapes::objects(place, given, "steps")?.iter().enumerate() {
        run.step(&format!("{place}'s step {index}"), step, source, world)?;
    }
    Ok(run)
}

fn edge_source(place: &str, named: &str) -> Result<EdgeSource, DriverFailure> {
    match named {
        "capabilities" => Ok(EdgeSource::CAPABILITIES),
        "services" => Ok(EdgeSource::SERVICES),
        _ => Err(format!(
            "{place} names the edge source {named}, which no driver knows"
        )),
    }
}

fn actions(place: &str, plugin: &Members, key: &str) -> Result<Arc<Vec<Members>>, DriverFailure> {
    let actions = shapes::objects(place, plugin, key)?;
    for (index, action) in actions.iter().enumerate() {
        let action_place = format!("{place} action {index}");
        let named = shapes::the_one_named(&action_place, action, ACTIONS)?;
        let known: &[&str] = if named == WANT {
            &[WANT, "await"]
        } else {
            &[named]
        };
        shapes::refuse_unknown_keys(&action_place, action, known)?;
        shapes::text(&action_place, action, named)?;
        shapes::flag(&action_place, action, "await", true)?;
    }
    Ok(Arc::new(actions.into_iter().cloned().collect()))
}

impl LifecycleRun {
    fn build(
        &mut self,
        place: &str,
        plugin: &Members,
        world: &mut DriverWorld,
    ) -> Result<(), DriverFailure> {
        let id = shapes::text(place, plugin, "id")?;
        let plugin_place = format!("{place}'s plugin {id}");
        shapes::refuse_unknown_keys(
            &plugin_place,
            plugin,
            &[
                "id",
                "provides",
                "consumes",
                CAPABILITIES,
                "activate",
                "deactivate",
            ],
        )?;
        let (provided, consumed) = capabilities_of(&plugin_place, plugin)?;
        let declared = manifest(
            &id,
            &shapes::texts(&plugin_place, plugin, "provides")?,
            &shapes::texts(&plugin_place, plugin, "consumes")?,
            &provided,
            &consumed,
        );
        let deactivate = if plugin.contains_key("deactivate") {
            Some(actions(
                &format!("{plugin_place}'s deactivate"),
                plugin,
                "deactivate",
            )?)
        } else {
            None
        };
        let entry = ActingEntry {
            name: id.clone(),
            records: self.records.clone(),
            activate: actions(&format!("{plugin_place}'s activate"), plugin, "activate")?,
            deactivate,
        };
        world.register(declared, Arc::new(entry))
    }

    fn step(
        &mut self,
        place: &str,
        step: &Members,
        source: EdgeSource,
        world: &DriverWorld,
    ) -> Result<(), DriverFailure> {
        let named = shapes::the_one_named(
            place,
            step,
            &["activate", "release", "destroy", "verify", GET],
        )?;
        let known: &[&str] = if named == GET {
            &["plugin", GET]
        } else {
            &[named]
        };
        shapes::refuse_unknown_keys(place, step, known)?;
        match named {
            "activate" => self.activate(&shapes::texts(place, step, "activate")?, source, world),
            "release" => {
                let plugin = world.plugin(&shapes::text(place, step, "release")?)?;
                match world.host.release(plugin).value() {
                    Some(Ok(_)) => Ok(()),
                    Some(Err(answer)) => self.thrown(format!("the release answered {answer}")),
                    None => self.thrown("the release left something to run".to_owned()),
                }
            }
            "destroy" => {
                shapes::require_true(place, step, "destroy")?;
                self.destroying = true;
                self.loud_before_the_destroy = world.written(true);
                world.host.destroy();
                Ok(())
            }
            "verify" => {
                self.verify(&shapes::text(place, step, "verify")?, world);
                Ok(())
            }
            _ => {
                let plugin = shapes::text(place, step, "plugin")?;
                let context = locked(&self.records.contexts)
                    .get(&plugin)
                    .cloned()
                    .ok_or_else(|| {
                        format!("a step names {plugin}, whose activate was never called")
                    })?;
                get(
                    &self.records,
                    &plugin,
                    &context,
                    &shapes::text(place, step, GET)?,
                );
                Ok(())
            }
        }
    }

    /// A step that answers what no well-made step can fails the run, unless the destroy began, after which it is
    /// counted as loud.
    fn thrown(&mut self, failure: DriverFailure) -> Result<(), DriverFailure> {
        if self.destroying {
            self.thrown_since_the_destroy += 1;
            Ok(())
        } else {
            Err(failure)
        }
    }

    fn activate(
        &mut self,
        names: &[String],
        source: EdgeSource,
        world: &DriverWorld,
    ) -> Result<(), DriverFailure> {
        let handles = names
            .iter()
            .map(|name| world.plugin(name))
            .collect::<Result<Vec<_>, _>>()?;
        let activation = match world.host.activate(&handles, source) {
            Ok(activation) => activation,
            Err(answer) => {
                return self.thrown(format!("the host answered the activation {answer}"));
            }
        };
        let plan = activation
            .plan
            .order
            .iter()
            .map(|plugin| world.name_of(*plugin))
            .collect::<Result<Vec<_>, _>>()?;
        self.plans.push(texts_json(plan));
        Ok(())
    }

    /// Records `quiet` when the verify neither answered what no well-made call can nor wrote to the log or the sink.
    fn verify(&mut self, name: &str, world: &DriverWorld) {
        let written = world.written(false);
        let well_made = world.plugin(name).is_ok_and(|plugin| {
            matches!(
                world.host.verify(plugin),
                Ok(_) | Err(Answer::Refused { .. } | Answer::Absent | Answer::Destroyed)
            )
        });
        if !well_made && self.destroying {
            self.thrown_since_the_destroy += 1;
        }
        let quiet = well_made && world.written(false) == written;
        self.verifies
            .push(Json::Text(if quiet { "quiet" } else { "loud" }.to_owned()));
    }

    pub(crate) fn observers<'a>(
        &'a self,
        world: &'a DriverWorld,
    ) -> BTreeMap<&'static str, Observer<'a>> {
        let mut observers: BTreeMap<&'static str, Observer<'a>> = BTreeMap::new();
        observers.insert("plans", Box::new(|_| Ok(Json::Array(self.plans.clone()))));
        observers.insert(
            "trace",
            Box::new(|expectation| observations::before(expectation, &locked(&self.records.trace))),
        );
        observers.insert("active", Box::new(move |_| active(world)));
        observers.insert("forgotten", Box::new(move |_| Ok(forgotten(world))));
        observers.insert(
            "pending",
            Box::new(|_| Ok(texts_json(locked(&self.records.pending).iter().cloned()))),
        );
        observers.insert(
            "verifies",
            Box::new(|_| Ok(Json::Array(self.verifies.clone()))),
        );
        observers.insert("loud", Box::new(move |_| Ok(self.loud(world))));
        observers
    }

    fn loud(&self, world: &DriverWorld) -> Json {
        let loud = if self.destroying {
            world.written(true) - self.loud_before_the_destroy + self.thrown_since_the_destroy
        } else {
            self.thrown_since_the_destroy
        };
        Json::Number(loud as f64)
    }
}

fn capabilities_of(
    place: &str,
    plugin: &Members,
) -> Result<(Vec<String>, Vec<String>), DriverFailure> {
    if !plugin.contains_key(CAPABILITIES) {
        return Ok((Vec::new(), Vec::new()));
    }
    let capabilities_place = format!("{place}'s capabilities");
    let capabilities = shapes::object_at(place, plugin, CAPABILITIES)?;
    shapes::refuse_unknown_keys(&capabilities_place, capabilities, &["provides", "consumes"])?;
    Ok((
        shapes::texts(&capabilities_place, capabilities, "provides")?,
        shapes::texts(&capabilities_place, capabilities, "consumes")?,
    ))
}

fn active(world: &DriverWorld) -> Result<Json, DriverFailure> {
    match world.host.ledger() {
        Ok(rows) => {
            let names: BTreeSet<String> = rows
                .into_iter()
                .filter(|row| row.status == PluginStatus::ACTIVE)
                .map(|row| row.import_name)
                .collect();
            Ok(texts_json(names))
        }
        Err(Answer::Destroyed) => Ok(Json::Array(Vec::new())),
        Err(other) => Err(format!("the host answered no ledger: {other}")),
    }
}

fn forgotten(world: &DriverWorld) -> Json {
    let names: BTreeSet<String> = world
        .names()
        .into_iter()
        .filter(|name| {
            world
                .plugin(name)
                .is_ok_and(|plugin| world.host.manifest(plugin).is_none())
        })
        .collect();
    texts_json(names)
}
