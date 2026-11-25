//! Drives a `description` vector: the standard's `described` library loaded through the host on the standard's
//! description check, each holder a plugin opened with its activate left in flight, and every call made through the
//! [`BoundDefinition`] the holder resolved, or one an answer carried and a step kept. A vector's JSON is translated
//! into [`Value`]s by the kinds the description gives each, and every answer back, as the kit's
//! `docs/vectors/layer/description.md` writes them.
//!
//! The mirror whose releases a `readReleases` step counts is found as the holder's resolved definition that has a
//! `count_releases` operation, since a vector does not name the service.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::echo;
use super::library::declaration_of;
use super::observations::Observer;
use super::shapes::{self, DriverFailure, Members, texts_json};
use super::world::{DriverWorld, InFlight, manifest};
use crate::context::{ForeignObject, Object};
use crate::core::{Answer, EdgeSource, STATUS_MALFORMED, STATUS_OUT_OF_MEMORY, STATUS_REFUSED};
use crate::host::description_abi::{
    KIND_ARRAY as ARRAY, KIND_BYTES as BYTES, KIND_F64 as F64, KIND_HANDLE as HANDLE,
    KIND_I64 as I64, KIND_POINTER as POINTER, KIND_SIZE as SIZE, KIND_STRUCT as STRUCT,
    KIND_TEXT as TEXT, KIND_TEXTS as TEXTS, KIND_U32 as U32, KIND_U64 as U64, KIND_VALUE as VALUE,
};
use crate::host::{
    Answers, BoundDefinition, CallStatus, DescribedAnswer, DescribedCallError, DescribedHandle,
    Inputs, Layouts, LibraryDeclaration, LoadedLibrary, Tree, Value,
};
use crate::standard::Json;

const COUNT_RELEASES: &str = "count_releases";

const STEPS: &[&str] = &[
    "load",
    "get",
    "call",
    "release",
    "readReleases",
    "unload",
    "blocking",
];

/// A definition an answer carried, kept under a step's name with the answer that carried it.
struct KeptDefinition {
    definition: BoundDefinition,
    carrier: Answers,
}

pub(crate) struct DescriptionRun {
    described_binary: PathBuf,
    declaration: LibraryDeclaration,
    load: Option<Result<LoadedLibrary, Answer>>,
    answered_addresses: HashMap<String, usize>,
    resolved: BTreeMap<String, Vec<(String, Object)>>,
    handles: HashMap<String, DescribedHandle>,
    definitions: HashMap<String, KeptDefinition>,
    got: Vec<bool>,
    calls: Vec<Json>,
    blocking: Vec<Json>,
    release_reads: Vec<BTreeMap<String, u64>>,
    reported_from: (usize, usize),
}

pub(crate) fn drive(
    given: &Members,
    world: &mut DriverWorld,
    described_binary: &Path,
) -> Result<DescriptionRun, DriverFailure> {
    let place = "the vector's given";
    shapes::refuse_unknown_keys(place, given, &["declaration", "holders", "steps"])?;
    let mut run = DescriptionRun {
        described_binary: described_binary.to_owned(),
        declaration: declaration_of(place, shapes::object_at(place, given, "declaration")?)?,
        load: None,
        answered_addresses: HashMap::new(),
        resolved: BTreeMap::new(),
        handles: HashMap::new(),
        definitions: HashMap::new(),
        got: Vec::new(),
        calls: Vec::new(),
        blocking: Vec::new(),
        release_reads: Vec::new(),
        reported_from: (0, 0),
    };
    run.open(&shapes::texts(place, given, "holders")?, world)?;
    for (index, step) in shapes::objects(place, given, "steps")?
        .into_iter()
        .enumerate()
    {
        run.step(&format!("{place}'s step {index}"), step, world)?;
    }
    Ok(run)
}

impl DescriptionRun {
    fn open(&mut self, holders: &[String], world: &mut DriverWorld) -> Result<(), DriverFailure> {
        let ids: Vec<String> = self
            .declaration
            .services
            .iter()
            .map(|service| service.id.clone())
            .collect();
        for holder in holders {
            world.register(manifest(holder, &[], &ids, &[], &[]), Arc::new(InFlight))?;
        }
        world.open_in_flight(holders, EdgeSource::CAPABILITIES)?;
        self.reported_from = world.reported_marks();
        Ok(())
    }

    fn step(
        &mut self,
        place: &str,
        step: &Members,
        world: &DriverWorld,
    ) -> Result<(), DriverFailure> {
        match shapes::the_one_named(place, step, STEPS)? {
            "load" => {
                shapes::refuse_unknown_keys(place, step, &["load"])?;
                shapes::require_true(place, step, "load")?;
                self.load_library(world)
            }
            "unload" => {
                shapes::refuse_unknown_keys(place, step, &["unload"])?;
                shapes::require_true(place, step, "unload")?;
                self.unload(world)
            }
            "get" => {
                shapes::refuse_unknown_keys(place, step, &["holder", "get"])?;
                let holder = shapes::text(place, step, "holder")?;
                self.get(world, &holder, &shapes::text(place, step, "get")?)
            }
            "call" => {
                shapes::refuse_unknown_keys(
                    place,
                    step,
                    &["holder", "call", "operation", "with", "keep"],
                )?;
                let keep = match step.get("keep") {
                    Some(_) => Some(shapes::text(place, step, "keep")?),
                    None => None,
                };
                let definition = self.bound(
                    &shapes::text(place, step, "holder")?,
                    &shapes::text(place, step, "call")?,
                )?;
                let called = self.call(
                    place,
                    &definition,
                    &shapes::text(place, step, "operation")?,
                    shapes::object_at(place, step, "with")?,
                    keep,
                )?;
                self.calls.push(called);
                Ok(())
            }
            "release" => {
                shapes::refuse_unknown_keys(place, step, &["holder", "release"])?;
                shapes::text(place, step, "holder")?;
                self.release(&shapes::text(place, step, "release")?)
            }
            "blocking" => {
                shapes::refuse_unknown_keys(place, step, &["holder", "blocking"])?;
                let definition = self.bound(
                    &shapes::text(place, step, "holder")?,
                    &shapes::text(place, step, "blocking")?,
                )?;
                self.blocking
                    .push(texts_json(definition.blocking_operations().iter().cloned()));
                Ok(())
            }
            _ => {
                shapes::refuse_unknown_keys(place, step, &["holder", "readReleases"])?;
                shapes::require_true(place, step, "readReleases")?;
                let counts = self.read_releases(&shapes::text(place, step, "holder")?)?;
                self.release_reads.push(counts);
                Ok(())
            }
        }
    }

    fn load_library(&mut self, world: &DriverWorld) -> Result<(), DriverFailure> {
        let load = world
            .host
            .load_library(&self.described_binary, &self.declaration);
        if load.is_ok() {
            let definitions: Vec<_> = self
                .declaration
                .services
                .iter()
                .filter(|service| service.is_a_definition())
                .collect();
            let symbols: Vec<String> = definitions
                .iter()
                .map(|service| service.functions[0].symbol.clone())
                .collect();
            let addresses = echo::answered_addresses(&self.described_binary, &symbols)?;
            for (service, address) in definitions.iter().zip(addresses) {
                self.answered_addresses.insert(service.id.clone(), address);
            }
        }
        self.load = Some(load);
        Ok(())
    }

    fn unload(&self, world: &DriverWorld) -> Result<(), DriverFailure> {
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

    fn get(&mut self, world: &DriverWorld, holder: &str, id: &str) -> Result<(), DriverFailure> {
        let answer = world.context(holder)?.get(id);
        self.got.push(answer.is_ok());
        if let Ok(object) = answer {
            let held = self.resolved.entry(holder.to_owned()).or_default();
            held.retain(|(kept, _)| kept != id);
            held.push((id.to_owned(), object));
        }
        Ok(())
    }

    fn resolved_definition(&self, holder: &str, id: &str) -> Option<BoundDefinition> {
        self.resolved
            .get(holder)?
            .iter()
            .find(|(kept, _)| kept == id)
            .and_then(|(_, object)| object.downcast_ref::<BoundDefinition>().cloned())
    }

    /// The bound definition the holder resolved for the service `id`, or else the one kept under the name, so a kept
    /// name never hides a service.
    fn bound(&self, holder: &str, id: &str) -> Result<BoundDefinition, DriverFailure> {
        self.resolved_definition(holder, id)
            .or_else(|| self.definitions.get(id).map(|kept| kept.definition.clone()))
            .ok_or_else(|| {
                format!("{holder} resolved no bound definition {id} and keeps none by that name")
            })
    }

    /// Releases the handle kept under the name, or ends the answer that carried the definition kept under it.
    fn release(&mut self, name: &str) -> Result<(), DriverFailure> {
        if let Some(kept) = self.definitions.get(name) {
            kept.carrier.release();
            return Ok(());
        }
        self.handles
            .get(name)
            .map(DescribedHandle::release)
            .ok_or_else(|| format!("a release step names {name}, which no step kept"))
    }

    fn read_releases(&self, holder: &str) -> Result<BTreeMap<String, u64>, DriverFailure> {
        let mirror = self
            .resolved
            .get(holder)
            .into_iter()
            .flatten()
            .filter_map(|(_, object)| object.downcast_ref::<BoundDefinition>())
            .find(|definition| definition.operation_names().contains(&COUNT_RELEASES))
            .ok_or_else(|| {
                format!("{holder} resolved nothing with a {COUNT_RELEASES} operation")
            })?;
        let answered = mirror.call(COUNT_RELEASES, &Inputs::new());
        let Ok(DescribedAnswer::Answered(answers)) = &answered else {
            return Err(format!("{COUNT_RELEASES} answered {answered:?}"));
        };
        let Some(Value::Struct(counts)) = answers.value() else {
            return Err(format!("{COUNT_RELEASES} answered {answers:?}"));
        };
        counts
            .iter()
            .map(|(name, count)| match count {
                Value::U64(count) => Ok((name.to_owned(), *count)),
                other => Err(format!("{COUNT_RELEASES} counted {name} as {other:?}")),
            })
            .collect()
    }

    fn call(
        &mut self,
        place: &str,
        definition: &BoundDefinition,
        operation: &str,
        with: &Members,
        keep: Option<String>,
    ) -> Result<Json, DriverFailure> {
        let inputs = self.inputs(place, definition, operation, with)?;
        let answered = definition.call(operation, &inputs);
        let answers = match answered {
            Ok(DescribedAnswer::Answered(answers)) => answers,
            Ok(DescribedAnswer::None) => return Ok(status_json("none")),
            Err(failure) => return failure_json(&failure),
        };
        let answer_count = definition
            .plan(operation)
            .map_or(0, |plan| plan.answer_names().len());
        if answer_count != 1 {
            return Err(format!(
                "{place}: a call vector reads one answer, and {operation} has {answer_count}"
            ));
        }
        let mut kept = Kept {
            name: keep,
            handle: None,
            definition: None,
        };
        let answer = answers.value().map(|value| kept.json(value)).transpose()?;
        self.keep(kept, answers)?;
        let mut call = BTreeMap::from([("status".to_owned(), Json::Text("ok".to_owned()))]);
        if let Some(answer) = answer {
            call.insert("answer".to_owned(), answer);
        }
        Ok(Json::Object(call))
    }

    fn keep(&mut self, kept: Kept, answers: Answers) -> Result<(), DriverFailure> {
        let Some(name) = kept.name else {
            return Ok(());
        };
        if let Some(handle) = kept.handle {
            self.handles.insert(name.clone(), handle);
        }
        if let Some(definition) = kept.definition {
            self.definitions.insert(
                name,
                KeptDefinition {
                    definition,
                    carrier: answers,
                },
            );
        }
        Ok(())
    }

    fn inputs(
        &self,
        place: &str,
        definition: &BoundDefinition,
        operation: &str,
        with: &Members,
    ) -> Result<Inputs, DriverFailure> {
        let plan = definition.plan(operation);
        let translate = Translate {
            layouts: definition.layouts(),
            handles: &self.handles,
        };
        with.iter()
            .map(|(name, json)| {
                let value = match plan.and_then(|plan| plan.input(name)) {
                    Some(parameter) => translate.value(
                        parameter.kind,
                        parameter.element,
                        parameter.type_name.as_deref(),
                        json,
                        &format!("{place}'s {name}"),
                    )?,
                    None => unwritable(),
                };
                Ok((name.clone(), value))
            })
            .collect()
    }

    fn is_same_object(&self) -> bool {
        self.resolved.values().flatten().all(|(id, object)| {
            self.answered_addresses.get(id).is_none_or(|answered| {
                let address = object
                    .downcast_ref::<BoundDefinition>()
                    .map(BoundDefinition::address)
                    .or_else(|| {
                        object
                            .downcast_ref::<ForeignObject>()
                            .map(ForeignObject::address)
                    });
                address == Some(*answered)
            })
        })
    }

    /// Each read of the counts after the first, minus the first, per kind, as decimal strings.
    fn released(&self) -> Json {
        let Some(first) = self.release_reads.first() else {
            return Json::Array(Vec::new());
        };
        let differences = self.release_reads[1..]
            .iter()
            .map(|later| {
                Json::Object(
                    first
                        .iter()
                        .map(|(name, before)| {
                            let after = later.get(name).copied().unwrap_or(*before);
                            (
                                camel_case(name),
                                Json::Text(after.wrapping_sub(*before).to_string()),
                            )
                        })
                        .collect(),
                )
            })
            .collect();
        Json::Array(differences)
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
                    self.got.iter().map(|got| Json::Bool(*got)).collect(),
                ))
            }),
        );
        observers.insert(
            "sameObject",
            Box::new(|_| Ok(Json::Bool(self.is_same_object()))),
        );
        observers.insert("calls", Box::new(|_| Ok(Json::Array(self.calls.clone()))));
        observers.insert(
            "blocking",
            Box::new(|_| Ok(Json::Array(self.blocking.clone()))),
        );
        observers.insert("released", Box::new(|_| Ok(self.released())));
        observers.insert(
            "reported",
            Box::new(move |_| Ok(world.reported_since(self.reported_from))),
        );
        observers
    }
}

/// A value the binding refuses whatever it is, for a name the operation or struct does not let a caller write.
fn unwritable() -> Value {
    Value::Text(None)
}

fn status_json(status: &str) -> Json {
    Json::Object(BTreeMap::from([(
        "status".to_owned(),
        Json::Text(status.to_owned()),
    )]))
}

/// A call that answered neither OK nor NONE, the status as the kit writes it: the core's name, lowercase without its
/// prefix.
fn failure_json(failure: &DescribedCallError) -> Result<Json, DriverFailure> {
    let status = match failure.status {
        CallStatus::CallersMistake => {
            return Ok(Json::Object(BTreeMap::from([(
                "callersMistake".to_owned(),
                Json::Bool(true),
            )])));
        }
        CallStatus::Returned(STATUS_MALFORMED) => "malformed",
        CallStatus::Returned(STATUS_OUT_OF_MEMORY) => "out_of_memory",
        CallStatus::Returned(STATUS_REFUSED) => {
            let text = |text: &Option<String>| text.clone().map_or(Json::Null, Json::Text);
            return Ok(Json::Object(BTreeMap::from([
                ("status".to_owned(), Json::Text("refused".to_owned())),
                ("detail".to_owned(), text(&failure.detail)),
                ("fix".to_owned(), text(&failure.fix)),
            ])));
        }
        _ => {
            return Err(format!(
                "a call through a held definition answered {failure}"
            ));
        }
    };
    Ok(status_json(status))
}

/// A member's C name as the kit writes its count, `nested_refusals` as `nestedRefusals`.
fn camel_case(name: &str) -> String {
    let mut camel = String::with_capacity(name.len());
    let mut upper = false;
    for character in name.chars() {
        if character == '_' {
            upper = true;
        } else if upper {
            camel.extend(character.to_uppercase());
            upper = false;
        } else {
            camel.push(character);
        }
    }
    camel
}

/// The handle or definition one answer hands back, kept under the step's `keep`.
struct Kept {
    name: Option<String>,
    handle: Option<DescribedHandle>,
    definition: Option<BoundDefinition>,
}

impl Kept {
    fn name(&self, what: &str) -> Result<String, DriverFailure> {
        self.name
            .clone()
            .ok_or_else(|| format!("a call answered {what} and its step names no keep"))
    }

    /// The value as the kit writes it, a handle or a definition kept under the step's name.
    fn json(&mut self, value: &Value) -> Result<Json, DriverFailure> {
        Ok(match value {
            Value::U32(number) => Json::Number(f64::from(*number)),
            Value::U64(number) => Json::Text(number.to_string()),
            Value::I64(number) => Json::Text(number.to_string()),
            Value::Size(number) => Json::Text(number.to_string()),
            Value::F64(number) => Json::Number(*number),
            Value::Text(text) => text.clone().map_or(Json::Null, Json::Text),
            Value::Texts(texts) => Json::Array(
                texts
                    .iter()
                    .map(|text| text.clone().map_or(Json::Null, Json::Text))
                    .collect(),
            ),
            Value::Bytes(bytes) => Json::Array(
                bytes
                    .iter()
                    .map(|byte| Json::Number(f64::from(*byte)))
                    .collect(),
            ),
            Value::Array(items) => Json::Array(
                items
                    .iter()
                    .map(|item| self.json(item))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Struct(members) => Json::Object(
                members
                    .iter()
                    .map(|(name, member)| Ok((name.to_owned(), self.json(member)?)))
                    .collect::<Result<_, DriverFailure>>()?,
            ),
            Value::Tree(tree) => tree_json(tree),
            Value::NoValue => Json::Null,
            Value::Handle(handle) => {
                let name = self.name("a handle")?;
                self.handle = Some(handle.clone());
                Json::Object(BTreeMap::from([("handle".to_owned(), Json::Text(name))]))
            }
            Value::Definition(definition) => {
                let name = self.name("a definition")?;
                self.definition = Some(definition.clone());
                Json::Object(BTreeMap::from([(
                    "definition".to_owned(),
                    Json::Text(name),
                )]))
            }
        })
    }
}

fn tree_json(tree: &Tree) -> Json {
    match tree {
        Tree::Null => Json::Null,
        Tree::Boolean(boolean) => Json::Bool(*boolean),
        Tree::Number(number) => Json::Number(*number),
        Tree::Text(text) => Json::Text(text.clone()),
        Tree::List(items) => Json::Array(items.iter().map(tree_json).collect()),
        Tree::Record(entries) => Json::Object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), tree_json(value)))
                .collect(),
        ),
    }
}

fn json_tree(json: &Json) -> Tree {
    match json {
        Json::Null => Tree::Null,
        Json::Bool(boolean) => Tree::Boolean(*boolean),
        Json::Number(number) => Tree::Number(*number),
        Json::Text(text) => Tree::Text(text.clone()),
        Json::Array(items) => Tree::List(items.iter().map(json_tree).collect()),
        Json::Object(members) => Tree::Record(
            members
                .iter()
                .map(|(key, value)| (key.clone(), json_tree(value)))
                .collect(),
        ),
    }
}

/// Translates a vector's JSON into the values a bound definition takes, by the kinds its description gives each: a
/// 64-bit integer from a decimal string, bytes from an array of numbers, a handle from `{handle: name}`.
struct Translate<'a> {
    layouts: &'a Layouts,
    handles: &'a HashMap<String, DescribedHandle>,
}

impl Translate<'_> {
    fn value(
        &self,
        kind: u32,
        element: u32,
        type_name: Option<&str>,
        json: &Json,
        place: &str,
    ) -> Result<Value, DriverFailure> {
        let unread =
            || format!("{place} holds {json:?}, which the driver cannot read as kind {kind}");
        Ok(match kind {
            U32 => Value::U32(
                json.as_number()
                    .filter(|number| number.fract() == 0.0)
                    .ok_or_else(unread)? as u32,
            ),
            U64 => Value::U64(
                json.as_text()
                    .and_then(|text| text.parse().ok())
                    .ok_or_else(unread)?,
            ),
            I64 => Value::I64(
                json.as_text()
                    .and_then(|text| text.parse().ok())
                    .ok_or_else(unread)?,
            ),
            SIZE => Value::Size(
                json.as_text()
                    .and_then(|text| text.parse().ok())
                    .ok_or_else(unread)?,
            ),
            F64 => Value::F64(json.as_number().ok_or_else(unread)?),
            TEXT => Value::Text(json.as_text().map(str::to_owned)),
            TEXTS => Value::Texts(
                json.as_array()
                    .ok_or_else(unread)?
                    .iter()
                    .map(|text| text.as_text().map(str::to_owned))
                    .collect(),
            ),
            BYTES => Value::Bytes(
                json.as_array()
                    .ok_or_else(unread)?
                    .iter()
                    .map(|byte| byte.as_number().map(|byte| byte as u8).ok_or_else(unread))
                    .collect::<Result<_, _>>()?,
            ),
            STRUCT => self.structure(type_name, json, place)?,
            POINTER => self.value(element, 0, type_name, json, place)?,
            ARRAY => Value::Array(
                json.as_array()
                    .ok_or_else(unread)?
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        self.value(element, 0, type_name, item, &format!("{place}[{index}]"))
                    })
                    .collect::<Result<_, _>>()?,
            ),
            VALUE => Value::Tree(json_tree(json)),
            HANDLE => {
                let name = json
                    .member("handle")
                    .and_then(Json::as_text)
                    .ok_or_else(unread)?;
                Value::Handle(self.handles.get(name).cloned().ok_or_else(|| {
                    format!("{place} names the handle {name}, which no step kept")
                })?)
            }
            _ => return Err(unread()),
        })
    }

    fn structure(
        &self,
        type_name: Option<&str>,
        json: &Json,
        place: &str,
    ) -> Result<Value, DriverFailure> {
        let described = &self
            .layouts
            .laid_out(type_name)
            .ok_or_else(|| format!("{place} reaches a struct its description lacks"))?
            .described;
        let members = json
            .as_object()
            .ok_or_else(|| format!("{place} is not an object"))?;
        members
            .iter()
            .map(|(name, member)| {
                let value = match described.member_named(name) {
                    Some(found) => self.value(
                        found.kind,
                        found.element,
                        found.type_name.as_deref(),
                        member,
                        &format!("{place}.{name}"),
                    )?,
                    None => unwritable(),
                };
                Ok((name.clone(), value))
            })
            .collect::<Result<_, DriverFailure>>()
            .map(Value::Struct)
    }
}
