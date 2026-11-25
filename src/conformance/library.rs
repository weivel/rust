//! Drives a `library` vector: the core's `echo` library loaded through the host, each owner and holder a plugin
//! opened with its activate left in flight, and every step run on the suite's thread through the host's and the
//! contexts' public surface, so once a step returns the crate has nothing left to run from it.
//!
//! The host forgets a plugin as the last part of its release, so the only time a released plugin is not yet forgotten
//! is while the events its release queued are dispatched. A `get` a vector makes between a holder's `releaseHolder`
//! and its `forgetHolder` is made then, as a late call, from the holder's listener as it hears the first revocation
//! its release queued, and the `forgetHolder` finds the holder forgotten.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::echo;
use super::observations::Observer;
use super::shapes::{self, DriverFailure, Members, texts_json};
use super::world::{DriverWorld, manifest};
use crate::context::{
    Context, ForeignObject, Object, Pending, PluginEvent, Revocation, Settle, pending,
};
use crate::core::{Answer, EdgeSource};
use crate::host::{
    Entry, Finished, FunctionKind, LibraryAnswer, LibraryContract, LibraryDeclaration,
    LibraryFunction, LibraryService, LibraryServiceDeclaration, LoadedLibrary,
};
use crate::standard::Json;

const STEPS: &[&str] = &[
    "load",
    "get",
    "echo",
    "call",
    "drop",
    "countUnloads",
    "unload",
    "releaseHolding",
    "releaseHolder",
    "quarantine",
    "readUnloads",
    "registerAwaitingReleases",
    "forgetHolder",
];

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// An entry whose activate settles only when the suite settles it, so a step can make it fail.
#[derive(Default)]
struct HeldActivate {
    settle: Mutex<Option<Settle<Finished>>>,
}

impl Entry for HeldActivate {
    fn activate(&self, _: &Context) -> Pending<Finished> {
        let (activating, settle) = pending();
        *locked(&self.settle) = Some(settle);
        activating
    }
}

/// What the listeners of the run's plugins record, and the gets left to make late.
#[derive(Default)]
struct Heard {
    resolved: BTreeMap<String, Vec<(String, Object)>>,
    revocations: BTreeMap<String, Vec<Revocation>>,
    revoked: Vec<(String, String)>,
    holdings_released: Vec<(String, String)>,
    late_gets: HashMap<String, Vec<String>>,
    got: Vec<bool>,
}

impl Heard {
    fn resolve(&mut self, holder: &str, id: &str, object: Object) {
        let held = self.resolved.entry(holder.to_owned()).or_default();
        match held.iter_mut().find(|(kept, _)| kept == id) {
            Some(entry) => entry.1 = object,
            None => held.push((id.to_owned(), object)),
        }
    }

    fn resolved_by(&self, holder: &str, id: &str) -> Option<Object> {
        self.resolved
            .get(holder)?
            .iter()
            .find(|(kept, _)| kept == id)
            .map(|(_, object)| object.clone())
    }
}

pub(crate) struct LibraryRun {
    echo_binary: PathBuf,
    declaration: Option<LibraryDeclaration>,
    activates: HashMap<String, Arc<HeldActivate>>,
    heard: Arc<Mutex<Heard>>,
    counter: &'static AtomicU64,
    load: Option<Result<LoadedLibrary, Answer>>,
    answered_addresses: HashMap<String, usize>,
    echoed: Vec<String>,
    unloads: Vec<u64>,
    kept_answers: HashMap<(String, String), LibraryAnswer>,
    consumed: BTreeSet<usize>,
    reported_from: (usize, usize),
}

pub(crate) fn drive(
    given: &Members,
    world: &mut DriverWorld,
    echo_binary: &Path,
) -> Result<LibraryRun, DriverFailure> {
    let place = "the vector's given";
    shapes::refuse_unknown_keys(place, given, &["declaration", "owners", "holders", "steps"])?;
    let declaration = match given.get("declaration") {
        Some(_) => Some(declaration_of(
            place,
            shapes::object_at(place, given, "declaration")?,
        )?),
        None => None,
    };
    let mut run = LibraryRun {
        echo_binary: echo_binary.to_owned(),
        declaration,
        activates: HashMap::new(),
        heard: Arc::default(),
        // The library writes the count as its binary closes, which may be as late as the host's destroy.
        counter: Box::leak(Box::new(AtomicU64::new(0))),
        load: None,
        answered_addresses: HashMap::new(),
        echoed: Vec::new(),
        unloads: Vec::new(),
        kept_answers: HashMap::new(),
        consumed: BTreeSet::new(),
        reported_from: (0, 0),
    };
    let owners = match given.get("owners") {
        Some(_) => shapes::objects(place, given, "owners")?,
        None => Vec::new(),
    };
    run.open(
        place,
        &owners,
        &shapes::texts(place, given, "holders")?,
        world,
    )?;
    let steps = shapes::objects(place, given, "steps")?;
    for index in 0..steps.len() {
        if !run.consumed.contains(&index) {
            run.step(&format!("{place}'s step {index}"), &steps, index, world)?;
        }
    }
    Ok(run)
}

/// A declaration as a vector writes it, a service's `description` symbol and its contracts included.
pub(crate) fn declaration_of(
    place: &str,
    declaration: &Members,
) -> Result<LibraryDeclaration, DriverFailure> {
    let place = format!("{place}'s declaration");
    shapes::refuse_unknown_keys(
        &place,
        declaration,
        &["importName", "version", "release", "services", "contracts"],
    )?;
    let services = listed(&place, declaration, "services")?
        .into_iter()
        .map(|service| service_of(&place, service))
        .collect::<Result<Vec<_>, _>>()?;
    let contracts = listed(&place, declaration, "contracts")?
        .into_iter()
        .map(|contract| {
            let contract_place = format!("{place}'s contract");
            shapes::refuse_unknown_keys(&contract_place, contract, &["capability", "description"])?;
            Ok(LibraryContract {
                capability: shapes::text(&contract_place, contract, "capability")?,
                description: shapes::text(&contract_place, contract, "description")?,
            })
        })
        .collect::<Result<Vec<_>, DriverFailure>>()?;
    Ok(LibraryDeclaration {
        import_name: shapes::text(&place, declaration, "importName")?,
        version: shapes::text(&place, declaration, "version")?,
        release: optional_text(&place, declaration, "release")?,
        services,
        contracts,
    })
}

fn service_of(place: &str, service: &Members) -> Result<LibraryServiceDeclaration, DriverFailure> {
    let place = format!("{place}'s service");
    shapes::refuse_unknown_keys(&place, service, &["id", "description", "functions"])?;
    let functions = shapes::objects(&place, service, "functions")?
        .into_iter()
        .map(|function| {
            shapes::refuse_unknown_keys(&place, function, &["name", "symbol", "kind"])?;
            Ok(LibraryFunction {
                name: shapes::text(&place, function, "name")?,
                symbol: shapes::text(&place, function, "symbol")?,
                kind: kind_of(&place, &shapes::text(&place, function, "kind")?)?,
                context: 0,
            })
        })
        .collect::<Result<Vec<_>, DriverFailure>>()?;
    Ok(LibraryServiceDeclaration {
        id: shapes::text(&place, service, "id")?,
        functions,
        description: optional_text(&place, service, "description")?,
    })
}

fn kind_of(place: &str, kind: &str) -> Result<FunctionKind, DriverFailure> {
    match kind {
        "bytes" => Ok(FunctionKind::Bytes),
        "definition" => Ok(FunctionKind::Definition),
        other => Err(format!(
            "{place} names the kind {other}, which no driver knows"
        )),
    }
}

fn listed<'a>(
    place: &str,
    owner: &'a Members,
    key: &str,
) -> Result<Vec<&'a Members>, DriverFailure> {
    match owner.get(key) {
        Some(_) => shapes::objects(place, owner, key),
        None => Ok(Vec::new()),
    }
}

fn optional_text(place: &str, owner: &Members, key: &str) -> Result<Option<String>, DriverFailure> {
    match owner.get(key) {
        Some(_) => shapes::text(place, owner, key).map(Some),
        None => Ok(None),
    }
}

fn hear(heard: &Arc<Mutex<Heard>>, context: &Context, plugin: &str, event: &PluginEvent) {
    let late = {
        let mut heard = locked(heard);
        match event {
            PluginEvent::HoldingsReleased { id, .. } => {
                heard
                    .holdings_released
                    .push((plugin.to_owned(), id.clone()));
                return;
            }
            PluginEvent::Revoked(revocation) => {
                heard
                    .revocations
                    .entry(plugin.to_owned())
                    .or_default()
                    .push(revocation.clone());
                let named: Vec<(String, String)> = heard
                    .resolved
                    .get(plugin)
                    .into_iter()
                    .flatten()
                    .filter(|(_, object)| Arc::ptr_eq(object, &revocation.object))
                    .map(|(id, _)| (plugin.to_owned(), id.clone()))
                    .collect();
                heard.revoked.extend(named);
                heard.late_gets.remove(plugin).unwrap_or_default()
            }
            _ => return,
        }
    };
    for id in late {
        get_into(heard, context, plugin, &id);
    }
}

fn get_into(heard: &Arc<Mutex<Heard>>, context: &Context, holder: &str, id: &str) {
    let answer = context.get(id);
    let mut heard = locked(heard);
    heard.got.push(answer.is_ok());
    if let Ok(object) = answer {
        heard.resolve(holder, id, object);
    }
}

impl LibraryRun {
    fn declared_ids(&self) -> Vec<String> {
        self.declaration
            .iter()
            .flat_map(|declaration| {
                declaration
                    .services
                    .iter()
                    .map(|service| service.id.clone())
            })
            .collect()
    }

    /// Opens the owners, then the holders, each holder consuming every id the declaration lists and every id an owner
    /// offers.
    fn open(
        &mut self,
        place: &str,
        owners: &[&Members],
        holders: &[String],
        world: &mut DriverWorld,
    ) -> Result<(), DriverFailure> {
        let mut ids = self.declared_ids();
        let mut opened = Vec::new();
        for owner in owners {
            let owner_place = format!("{place}'s owner");
            shapes::refuse_unknown_keys(&owner_place, owner, &["id", "provides"])?;
            let name = shapes::text(&owner_place, owner, "id")?;
            let provides = shapes::texts(&owner_place, owner, "provides")?;
            ids.extend(provides.iter().cloned());
            self.register_held(world, manifest(&name, &provides, &[], &[], &[]))?;
            opened.push(name);
        }
        for holder in holders {
            self.register_held(world, manifest(holder, &[], &ids, &[], &[]))?;
            opened.push(holder.clone());
        }
        world.open_in_flight(&opened, EdgeSource::CAPABILITIES)?;
        for plugin in &opened {
            let context = world.context(plugin)?;
            let heard = self.heard.clone();
            let name = plugin.clone();
            let listening = context.clone();
            context.listen(move |event| hear(&heard, &listening, &name, event));
        }
        self.reported_from = (world.failures().len(), world.logs().len());
        Ok(())
    }

    fn register_held(
        &mut self,
        world: &mut DriverWorld,
        manifest: crate::core::Manifest,
    ) -> Result<(), DriverFailure> {
        let entry = Arc::new(HeldActivate::default());
        self.activates
            .insert(manifest.import_name.clone(), entry.clone());
        world.register(manifest, entry)
    }

    fn step(
        &mut self,
        place: &str,
        steps: &[&Members],
        index: usize,
        world: &mut DriverWorld,
    ) -> Result<(), DriverFailure> {
        let step = steps[index];
        match shapes::the_one_named(place, step, STEPS)? {
            "load" => self.load_step(place, step, world),
            "unload" => {
                shapes::refuse_unknown_keys(place, step, &["unload"])?;
                shapes::require_true(place, step, "unload")?;
                self.unload(world)
            }
            "readUnloads" => {
                shapes::refuse_unknown_keys(place, step, &["readUnloads"])?;
                shapes::require_true(place, step, "readUnloads")?;
                self.unloads.push(self.counter.load(Ordering::SeqCst));
                Ok(())
            }
            "releaseHolder" => {
                shapes::refuse_unknown_keys(place, step, &["releaseHolder"])?;
                let holder = shapes::text(place, step, "releaseHolder")?;
                self.late_gets_before_the_forget(place, steps, index, &holder)?;
                release_holder(world, &holder)?;
                if locked(&self.heard).late_gets.contains_key(&holder) {
                    return Err(format!(
                        "{holder} heard no revocation its release queued, so its late gets were never made"
                    ));
                }
                Ok(())
            }
            "forgetHolder" => {
                shapes::refuse_unknown_keys(place, step, &["forgetHolder"])?;
                let holder = shapes::text(place, step, "forgetHolder")?;
                match world.host.context(world.plugin(&holder)?) {
                    Some(_) => Err(format!("the host still holds {holder} after its release")),
                    None => Ok(()),
                }
            }
            "registerAwaitingReleases" => {
                shapes::refuse_unknown_keys(place, step, &["owner", "registerAwaitingReleases"])?;
                let owner = shapes::text(place, step, "owner")?;
                let id = shapes::text(place, step, "registerAwaitingReleases")?;
                world
                    .context(&owner)?
                    .register_awaiting_releases(&id, Arc::new(()))
                    .map(|_| ())
                    .map_err(|answer| format!("{owner}'s registration of {id} answered {answer}"))
            }
            "quarantine" => {
                shapes::refuse_unknown_keys(place, step, &["quarantine"])?;
                self.quarantine(&shapes::text(place, step, "quarantine")?)
            }
            named => {
                let holder = shapes::text(place, step, "holder")?;
                self.holder_step(place, step, named, &holder, world)
            }
        }
    }

    fn load_step(
        &mut self,
        place: &str,
        step: &Members,
        world: &DriverWorld,
    ) -> Result<(), DriverFailure> {
        shapes::refuse_unknown_keys(place, step, &["load"])?;
        shapes::require_true(place, step, "load")?;
        let declaration = self
            .declaration
            .clone()
            .ok_or_else(|| format!("{place} loads a library the vector declares none of"))?;
        let load = world.host.load_library(&self.echo_binary, &declaration);
        if load.is_ok() {
            let definitions: Vec<&LibraryServiceDeclaration> = declaration
                .services
                .iter()
                .filter(|service| service.is_a_definition())
                .collect();
            let symbols: Vec<String> = definitions
                .iter()
                .map(|service| service.functions[0].symbol.clone())
                .collect();
            let addresses = echo::answered_addresses(&self.echo_binary, &symbols)?;
            for (service, address) in definitions.iter().zip(addresses) {
                self.answered_addresses.insert(service.id.clone(), address);
            }
        }
        self.load = Some(load);
        Ok(())
    }

    fn holder_step(
        &mut self,
        place: &str,
        step: &Members,
        named: &str,
        holder: &str,
        world: &DriverWorld,
    ) -> Result<(), DriverFailure> {
        match named {
            "get" => {
                shapes::refuse_unknown_keys(place, step, &["holder", "get"])?;
                get_into(
                    &self.heard,
                    &world.context(holder)?,
                    holder,
                    &shapes::text(place, step, "get")?,
                );
                Ok(())
            }
            "echo" => {
                shapes::refuse_unknown_keys(place, step, &["holder", "echo"])?;
                let address = self.first_definition(holder)?;
                let echoed = echo::echo_through(address, &shapes::text(place, step, "echo")?)?;
                self.echoed.push(echoed);
                Ok(())
            }
            "call" => {
                shapes::refuse_unknown_keys(
                    place,
                    step,
                    &["holder", "call", "function", "text", "keep"],
                )?;
                shapes::require_true(place, step, "keep")?;
                self.call(
                    holder,
                    &shapes::text(place, step, "call")?,
                    &shapes::text(place, step, "function")?,
                    &shapes::text(place, step, "text")?,
                )
            }
            "drop" => {
                shapes::refuse_unknown_keys(place, step, &["holder", "drop"])?;
                let id = shapes::text(place, step, "drop")?;
                self.kept_answers
                    .remove(&(holder.to_owned(), id.clone()))
                    .map(drop)
                    .ok_or_else(|| format!("{holder} keeps no answer of {id} to drop"))
            }
            "countUnloads" => {
                shapes::refuse_unknown_keys(place, step, &["holder", "countUnloads"])?;
                shapes::require_true(place, step, "countUnloads")?;
                echo::count_unloads(self.first_definition(holder)?, self.counter)
            }
            _ => {
                shapes::refuse_unknown_keys(place, step, &["holder", "releaseHolding"])?;
                self.release_holding(world, holder, &shapes::text(place, step, "releaseHolding")?)
            }
        }
    }

    /// Takes the `get` steps of `holder` that follow its release and come before its forget, to be made as late calls
    /// while its release's events are dispatched.
    fn late_gets_before_the_forget(
        &mut self,
        place: &str,
        steps: &[&Members],
        release: usize,
        holder: &str,
    ) -> Result<(), DriverFailure> {
        let mut late = Vec::new();
        for (index, step) in steps.iter().enumerate().skip(release + 1) {
            if step.get("forgetHolder").and_then(Json::as_text) == Some(holder) {
                let ids = late
                    .iter()
                    .map(|taken: &usize| {
                        shapes::refuse_unknown_keys(place, steps[*taken], &["holder", "get"])?;
                        shapes::text(place, steps[*taken], "get")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.consumed.extend(late);
                if !ids.is_empty() {
                    locked(&self.heard).late_gets.insert(holder.to_owned(), ids);
                }
                return Ok(());
            }
            let is_a_get_of_the_holder = step.get("holder").and_then(Json::as_text) == Some(holder)
                && step.contains_key("get");
            if !is_a_get_of_the_holder {
                return Ok(());
            }
            late.push(index);
        }
        Ok(())
    }

    fn first_definition(&self, holder: &str) -> Result<usize, DriverFailure> {
        locked(&self.heard)
            .resolved
            .get(holder)
            .into_iter()
            .flatten()
            .find_map(|(_, object)| {
                object
                    .downcast_ref::<ForeignObject>()
                    .map(ForeignObject::address)
            })
            .ok_or_else(|| {
                format!("{holder} resolved no definition before a step that calls through one")
            })
    }

    fn call(
        &mut self,
        holder: &str,
        id: &str,
        function: &str,
        text: &str,
    ) -> Result<(), DriverFailure> {
        let service = locked(&self.heard)
            .resolved_by(holder, id)
            .ok_or_else(|| format!("{holder} resolved no {id}"))?;
        let service = service
            .downcast_ref::<LibraryService>()
            .ok_or_else(|| format!("{holder} resolved no bytes service {id}"))?;
        match service.call(function, text.as_bytes()) {
            LibraryAnswer::Returned(bytes) => {
                self.kept_answers.insert(
                    (holder.to_owned(), id.to_owned()),
                    LibraryAnswer::Returned(bytes),
                );
                Ok(())
            }
            other => Err(format!(
                "{holder}'s call of {id} {function} answered {other:?}"
            )),
        }
    }

    fn unload(&mut self, world: &DriverWorld) -> Result<(), DriverFailure> {
        let Some(Ok(library)) = &self.load else {
            return Err(format!(
                "an unload step after a load that answered {:?}",
                self.load
            ));
        };
        match world.host.unload_library(library) {
            Ok(true) => Ok(()),
            other => Err(format!("the unload answered {other:?}")),
        }
    }

    fn release_holding(
        &self,
        world: &DriverWorld,
        holder: &str,
        id: &str,
    ) -> Result<(), DriverFailure> {
        let revocation = {
            let heard = locked(&self.heard);
            let object = heard
                .resolved_by(holder, id)
                .ok_or_else(|| format!("{holder} resolved no {id} to let go of"))?;
            heard
                .revocations
                .get(holder)
                .into_iter()
                .flatten()
                .rev()
                .find(|heard| heard.awaits_release && Arc::ptr_eq(&heard.object, &object))
                .cloned()
                .ok_or_else(|| {
                    format!("{holder} heard no revocation of {id} awaiting its release")
                })?
        };
        match world.context(holder)?.release_holding(&revocation) {
            Ok(true) => Ok(()),
            other => Err(format!(
                "{holder}'s release of its holding of {id} answered {other:?}"
            )),
        }
    }

    /// Quarantines a holder the way the host does it unasked: its activate fails, and the host quarantines it.
    fn quarantine(&self, holder: &str) -> Result<(), DriverFailure> {
        let settle = self
            .activates
            .get(holder)
            .and_then(|entry| locked(&entry.settle).take())
            .ok_or_else(|| {
                format!("the vector quarantines a plugin it does not hold in flight: {holder}")
            })?;
        settle.settle(Err(format!("the suite quarantines {holder}")));
        Ok(())
    }

    fn is_same_object(&self) -> bool {
        locked(&self.heard)
            .resolved
            .values()
            .flatten()
            .all(|(id, object)| {
                self.answered_addresses.get(id).is_none_or(|answered| {
                    object
                        .downcast_ref::<ForeignObject>()
                        .is_some_and(|foreign| foreign.address() == *answered)
                })
            })
    }

    fn reported(&self, world: &DriverWorld) -> Json {
        let (failures, logs) = self.reported_from;
        let sunk = world
            .failures()
            .into_iter()
            .skip(failures)
            .map(|failure| format!("{failure:?}"));
        let logged = world
            .logs()
            .into_iter()
            .skip(logs)
            .map(|record| format!("{record:?}"));
        texts_json(sunk.chain(logged))
    }

    pub(crate) fn observers<'a>(
        &'a self,
        world: &'a DriverWorld,
    ) -> BTreeMap<&'static str, Observer<'a>> {
        let mut observers: BTreeMap<&'static str, Observer<'a>> = BTreeMap::new();
        observers.insert(
            "loaded",
            Box::new(|_| Ok(Json::Bool(matches!(self.load, Some(Ok(_)))))),
        );
        observers.insert(
            "callersMistake",
            Box::new(|_| {
                Ok(Json::Bool(matches!(
                    self.load,
                    Some(Err(Answer::Mistake(_)))
                )))
            }),
        );
        observers.insert(
            "got",
            Box::new(|_| {
                Ok(Json::Array(
                    locked(&self.heard)
                        .got
                        .iter()
                        .map(|got| Json::Bool(*got))
                        .collect(),
                ))
            }),
        );
        observers.insert(
            "sameObject",
            Box::new(|_| Ok(Json::Bool(self.is_same_object()))),
        );
        observers.insert("echoed", Box::new(|_| Ok(texts_json(self.echoed.clone()))));
        observers.insert(
            "revoked",
            Box::new(|_| {
                let mut revoked = locked(&self.heard).revoked.clone();
                revoked.sort();
                Ok(pairs_json(revoked))
            }),
        );
        observers.insert(
            "unloads",
            Box::new(|_| {
                Ok(Json::Array(
                    self.unloads
                        .iter()
                        .map(|count| Json::Number(*count as f64))
                        .collect(),
                ))
            }),
        );
        observers.insert(
            "holdingsReleased",
            Box::new(|_| Ok(pairs_json(locked(&self.heard).holdings_released.clone()))),
        );
        observers.insert("reported", Box::new(move |_| Ok(self.reported(world))));
        observers
    }
}

fn pairs_json(pairs: Vec<(String, String)>) -> Json {
    Json::Array(
        pairs
            .into_iter()
            .map(|(plugin, id)| texts_json([plugin, id]))
            .collect(),
    )
}

fn release_holder(world: &DriverWorld, holder: &str) -> Result<(), DriverFailure> {
    match world.host.release(world.plugin(holder)?).value() {
        Some(Ok(true)) => Ok(()),
        other => Err(format!("the host's release of {holder} answered {other:?}")),
    }
}
