//! One operation of a described definition: its parameter steps and its answer and refusal blocks planned once at
//! the bind, and each call made from them.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::abi::{
    KIND_CONTEXT, KIND_HANDLE, KIND_POINTER, KIND_VALUE, ROLE_ANSWER, ROLE_INPUT, ROLE_REFUSAL,
    ValueTree,
};
use super::definition::BoundDefinition;
use super::described::{Operation, Parameter};
use super::holdings::{CarriedScope, DescribedHandle, Holdings, KeptAnswer, KeptBlocks, Mapping};
use super::invoke::{self, MOST_PARAMETERS, ReleaseCall, Slot};
use super::layouts::{LayoutRefusal, Layouts};
use super::memory::{Arena, put};
use super::read::{Answering, Reader};
use super::value::{
    Answers, CallStatus, DescribedAnswer, DescribedCallError, Inputs, Members, Value,
};
use super::write::Writer;
use crate::core::{STATUS_NONE, STATUS_OK, STATUS_REFUSED};

/// How large a block an answer or a refusal is handed, and the size its leading `size` is set to, if it has one.
#[derive(Debug, Clone, Copy)]
struct BlockShape {
    bytes: usize,
    leading_size: Option<u32>,
}

/// What one parameter is handed, worked out at the bind.
#[derive(Debug, Clone, Copy)]
enum Step {
    Answer(usize),
    Refusal,
    Count,
    Input { count: Option<usize> },
}

/// One operation, planned once at the bind and shared by every definition its description binds.
#[derive(Debug)]
pub(crate) struct OperationPlan {
    pub(crate) name: String,
    pub(crate) offset: usize,
    parameters: Vec<Parameter>,
    /// One step per parameter after the context.
    steps: Vec<Step>,
    input_names: Vec<Arc<str>>,
    answers: Vec<(usize, BlockShape)>,
    refusal: Option<(usize, BlockShape)>,
}

/// What a call reaches beyond its plan: the definition's function and context, its releases, its binary and its
/// holdings, and how a definition its answer carries is bound.
pub(crate) struct CallTarget<'a> {
    pub(crate) layouts: &'a Layouts,
    pub(crate) function: usize,
    pub(crate) context: usize,
    pub(crate) releases: &'a BTreeMap<String, ReleaseCall>,
    pub(crate) mapping: &'a Mapping,
    pub(crate) holdings: &'a Arc<Holdings>,
    pub(crate) carrier: &'a dyn CarriedBinder,
}

/// Binds a definition an answer carries, from the description nested for it.
pub(crate) trait CarriedBinder {
    fn bind_carried(
        &self,
        type_name: &str,
        pointer: usize,
    ) -> Result<(BoundDefinition, Arc<CarriedScope>), String>;
}

fn shape_of(layouts: &Layouts, parameter: &Parameter) -> BlockShape {
    let laid_out = layouts.laid_out(parameter.type_name.as_deref());
    match (parameter.kind, laid_out) {
        (KIND_POINTER, Some(laid_out)) => BlockShape {
            bytes: laid_out.described.size,
            leading_size: laid_out.leading_size,
        },
        (KIND_VALUE, _) => BlockShape {
            bytes: size_of::<ValueTree>(),
            leading_size: Some(size_of::<ValueTree>() as u32),
        },
        _ => BlockShape {
            bytes: size_of::<u64>(),
            leading_size: None,
        },
    }
}

impl OperationPlan {
    /// Plans `operation`, refusing one this binding cannot call.
    pub(crate) fn new(
        operation: &Operation,
        layouts: &Layouts,
    ) -> Result<OperationPlan, LayoutRefusal> {
        let parameters = &operation.parameters;
        if parameters.len() > MOST_PARAMETERS {
            return Err(format!(
                "its operation {} takes {} parameters, beyond the {MOST_PARAMETERS} this binding calls",
                operation.name,
                parameters.len()
            ));
        }
        if parameters
            .first()
            .is_none_or(|first| first.kind != KIND_CONTEXT)
        {
            return Err(format!(
                "its operation {} takes no context first",
                operation.name
            ));
        }
        let counted: Vec<&str> = parameters
            .iter()
            .filter_map(|parameter| parameter.count.as_deref())
            .collect();
        let index_of = |name: &str| {
            parameters
                .iter()
                .position(|parameter| &*parameter.name == name)
        };
        let mut plan = OperationPlan {
            name: operation.name.clone(),
            offset: operation.offset,
            parameters: parameters.clone(),
            steps: Vec::new(),
            input_names: Vec::new(),
            answers: Vec::new(),
            refusal: None,
        };
        for (index, parameter) in parameters.iter().enumerate().skip(1) {
            let step = match parameter.role {
                ROLE_ANSWER => {
                    plan.answers.push((index, shape_of(layouts, parameter)));
                    Step::Answer(plan.answers.len() - 1)
                }
                ROLE_REFUSAL => {
                    plan.refusal = Some((index, shape_of(layouts, parameter)));
                    Step::Refusal
                }
                ROLE_INPUT if counted.contains(&&*parameter.name) => Step::Count,
                _ => {
                    plan.input_names.push(parameter.name.clone());
                    Step::Input {
                        count: parameter
                            .count
                            .as_deref()
                            .and_then(index_of)
                            .map(|count| count - 1),
                    }
                }
            };
            plan.steps.push(step);
        }
        Ok(plan)
    }

    /// The names of its answer parameters, in the order it takes them.
    pub(crate) fn answer_names(&self) -> Vec<&str> {
        self.answers
            .iter()
            .map(|(index, _)| &*self.parameters[*index].name)
            .collect()
    }

    /// The parameters a caller writes, by name.
    pub(crate) fn input(&self, name: &str) -> Option<&Parameter> {
        self.input_names
            .iter()
            .any(|input| &**input == name)
            .then(|| {
                self.parameters
                    .iter()
                    .find(|parameter| &*parameter.name == name)
            })
            .flatten()
    }

    fn mistake(&self, detail: String) -> DescribedCallError {
        DescribedCallError::mistake(&self.name, detail)
    }

    /// Calls the operation with `inputs` through `target`: every input written into memory of the call's own, the
    /// one native call made, every answer copied out and released, but one carrying a definition, which lives until
    /// its own release.
    pub(crate) fn call(
        &self,
        target: &CallTarget<'_>,
        inputs: &Inputs,
    ) -> Result<DescribedAnswer, DescribedCallError> {
        if target.function == 0 {
            return Err(self.mistake(format!(
                "its provider does not offer {}: the struct its library answered ends before it, or holds NULL there",
                self.name
            )));
        }
        if let Some(unknown) = inputs
            .keys()
            .find(|name| !self.input_names.iter().any(|input| **input == ***name))
        {
            return Err(self.mistake(format!(
                "{} has no input {unknown} a caller writes; it takes {}",
                self.name,
                self.input_names.join(", ")
            )));
        }
        let mut arena = Arena::take();
        let blocks = self.blocks(&mut arena);
        let mut writer = Writer::new(target.layouts, target.holdings.library(), arena);
        let slots = match self.slots(&mut writer, &blocks, inputs) {
            Ok(slots) => slots,
            Err(detail) => {
                writer.arena.give_back();
                return Err(self.mistake(detail));
            }
        };
        let arena = writer.arena;
        let Some(mapped) = target.mapping.hold() else {
            arena.give_back();
            if target.mapping.carrier_ended() {
                return Err(self.mistake(
                    "the definition was carried by an answer that has ended, and lives only as long as that answer"
                        .to_owned(),
                ));
            }
            return Err(DescribedCallError {
                operation: self.name.clone(),
                status: CallStatus::Unloaded,
                detail: Some("the library's binary has closed".to_owned()),
                fix: None,
            });
        };
        // SAFETY: the function is this operation's, read from its definition's struct, its binary held mapped across
        // the call; the slots follow the parameters' kinds, and every pointer among them lies in the call's arena or
        // its blocks, which live past the call.
        let status =
            unsafe { invoke::call_operation(target.function, target.context, slots.as_slice()) };
        let outcome = self.outcome(target, status, blocks, arena);
        drop(mapped);
        outcome
    }

    /// The answer blocks and the refusal block, cut from the call's arena, each empty with its leading size set.
    fn blocks(&self, arena: &mut Arena) -> CallBlocks {
        let mut block = |shape: &BlockShape| {
            let address = arena.block(shape.bytes);
            if let Some(size) = shape.leading_size {
                // SAFETY: every block is at least one word, and the arena zeroed it.
                unsafe { put(address, size) };
            }
            address
        };
        let mut answers = [0; MOST_PARAMETERS];
        for (answer, (_, shape)) in self.answers.iter().enumerate() {
            answers[answer] = block(shape);
        }
        CallBlocks {
            answers,
            refusal: self.refusal.as_ref().map(|(_, shape)| block(shape)),
        }
    }

    fn slots<'a>(
        &'a self,
        writer: &mut Writer<'a>,
        blocks: &CallBlocks,
        inputs: &'a Inputs,
    ) -> Result<Slots, String> {
        let mut slots = Slots {
            slots: [Slot::Word(0); MOST_PARAMETERS],
            count: self.steps.len(),
        };
        for (position, step) in self.steps.iter().enumerate() {
            let parameter = &self.parameters[position + 1];
            match step {
                Step::Answer(answer) => {
                    slots.slots[position] = Slot::Word(blocks.answers[*answer] as u64)
                }
                Step::Refusal => {
                    slots.slots[position] = Slot::Word(blocks.refusal.unwrap_or(0) as u64)
                }
                Step::Count => {}
                Step::Input { count } => {
                    let (slot, counted) =
                        writer.input(&self.name, parameter, inputs.get(&*parameter.name))?;
                    slots.slots[position] = slot;
                    if let Some(count) = count {
                        slots.slots[*count] = Slot::Word(counted as u64);
                    }
                }
            }
        }
        Ok(slots)
    }

    fn outcome(
        &self,
        target: &CallTarget<'_>,
        status: i32,
        blocks: CallBlocks,
        arena: Arena,
    ) -> Result<DescribedAnswer, DescribedCallError> {
        match status {
            STATUS_OK => {}
            STATUS_NONE => {
                return self.finish(target, &blocks, arena, Ok(DescribedAnswer::None));
            }
            _ => {
                let failure = self.failure(target, status, &blocks);
                return self.finish(target, &blocks, arena, Err(failure));
            }
        }
        let mut carried = Carried {
            target,
            handles: Vec::new(),
            definitions: Vec::new(),
        };
        let values = match self.read_answers(target, &blocks, &mut carried) {
            Ok(values) => values,
            Err(fault) => {
                carried.release_all();
                return self.finish(
                    target,
                    &blocks,
                    arena,
                    Err(DescribedCallError::fault(&self.name, fault)),
                );
            }
        };
        let only = match self.answers.as_slice() {
            [(index, _)] => Some(self.parameters[*index].name.clone()),
            _ => None,
        };
        if carried.definitions.is_empty() {
            let answers = Answers {
                values,
                only,
                kept: None,
            };
            return self.finish(
                target,
                &blocks,
                arena,
                Ok(DescribedAnswer::Answered(answers)),
            );
        }
        self.release_refusal(target, &blocks);
        let mut releases = Vec::new();
        self.each_answer_release(target, &blocks, |pointer, release| {
            releases.push((pointer, release))
        });
        let mut arena = arena;
        arena.handles.clear();
        let kept = KeptBlocks {
            releases,
            arena,
            mapping: target.mapping.clone(),
        };
        let kept = Some(carried.keep(kept));
        Ok(DescribedAnswer::Answered(Answers { values, only, kept }))
    }

    /// Releases the refusal and every answer through the releases their description names, gives the arena back,
    /// then answers `answer`.
    fn finish(
        &self,
        target: &CallTarget<'_>,
        blocks: &CallBlocks,
        arena: Arena,
        answer: Result<DescribedAnswer, DescribedCallError>,
    ) -> Result<DescribedAnswer, DescribedCallError> {
        self.release_refusal(target, blocks);
        self.each_answer_release(target, blocks, |pointer, release| release.free(pointer));
        arena.give_back();
        answer
    }

    fn read_answers(
        &self,
        target: &CallTarget<'_>,
        blocks: &CallBlocks,
        carried: &mut Carried<'_>,
    ) -> Result<Members, String> {
        let mut values = Members::with_capacity(self.answers.len());
        let mut reader = Reader {
            layouts: target.layouts,
            answering: carried,
        };
        for (answer, (index, _)) in self.answers.iter().enumerate() {
            let parameter = &self.parameters[*index];
            if let Some(value) = reader.answer(parameter, blocks.answers[answer])? {
                values.push(parameter.name.clone(), value);
            }
        }
        Ok(values)
    }

    /// Hands `each` every answer block a release frees and that release, in the order the operation takes them.
    fn each_answer_release(
        &self,
        target: &CallTarget<'_>,
        blocks: &CallBlocks,
        mut each: impl FnMut(usize, ReleaseCall),
    ) {
        for (answer, (index, _)) in self.answers.iter().enumerate() {
            let parameter = &self.parameters[*index];
            if parameter.kind == KIND_HANDLE {
                continue;
            }
            let release = parameter
                .release
                .as_deref()
                .and_then(|name| target.releases.get(name));
            if let Some(release) = release {
                each(blocks.answers[answer], *release);
            }
        }
    }

    fn release_refusal(&self, target: &CallTarget<'_>, blocks: &CallBlocks) {
        let Some((index, _)) = &self.refusal else {
            return;
        };
        let release = self.parameters[*index]
            .release
            .as_deref()
            .and_then(|name| target.releases.get(name));
        if let (Some(release), Some(block)) = (release, blocks.refusal) {
            release.free(block);
        }
    }

    fn failure(
        &self,
        target: &CallTarget<'_>,
        status: i32,
        blocks: &CallBlocks,
    ) -> DescribedCallError {
        let refused = self
            .refusal
            .as_ref()
            .zip(blocks.refusal)
            .filter(|_| status == STATUS_REFUSED);
        let texts = refused
            .and_then(|((index, _), block)| {
                let laid_out = target
                    .layouts
                    .laid_out(self.parameters[*index].type_name.as_deref())?;
                Some(Reader::refusal_texts(laid_out, block))
            })
            .unwrap_or(Ok((None, None)));
        let (detail, fix) = match texts {
            Ok(texts) => texts,
            Err(fault) => return DescribedCallError::fault(&self.name, fault),
        };
        DescribedCallError {
            operation: self.name.clone(),
            status: CallStatus::Returned(status),
            detail,
            fix,
        }
    }
}

/// The answer blocks and the refusal block one call hands its provider, each in the call's arena.
struct CallBlocks {
    answers: [usize; MOST_PARAMETERS],
    refusal: Option<usize>,
}

/// The slots of one call, one per parameter after the context.
struct Slots {
    slots: [Slot; MOST_PARAMETERS],
    count: usize,
}

impl Slots {
    fn as_slice(&self) -> &[Slot] {
        &self.slots[..self.count]
    }
}

/// What one answer carries beyond its copy: every handle wrapped and every definition bound while it was read.
struct Carried<'a> {
    target: &'a CallTarget<'a>,
    handles: Vec<DescribedHandle>,
    definitions: Vec<(BoundDefinition, Arc<CarriedScope>)>,
}

impl Answering for Carried<'_> {
    fn handle(&mut self, pointer: usize, type_name: &str, release: Option<&str>) -> Value {
        let release = release
            .and_then(|name| self.target.releases.get(name))
            .copied()
            .unwrap_or(ReleaseCall::new(0, 0));
        let handle = DescribedHandle::hold(
            self.target.holdings,
            pointer,
            type_name,
            release,
            self.target.mapping.clone(),
        );
        self.handles.push(handle.clone());
        Value::Handle(handle)
    }

    fn definition(&mut self, pointer: usize, type_name: &str) -> Result<Value, String> {
        let (definition, scope) = self.target.carrier.bind_carried(type_name, pointer)?;
        self.definitions.push((definition.clone(), scope));
        Ok(Value::Definition(definition))
    }
}

impl Carried<'_> {
    /// Releases every handle and ends every definition, as an answer never handed to its caller must.
    fn release_all(&self) {
        for handle in &self.handles {
            handle.release();
        }
        for (definition, _) in &self.definitions {
            definition.end_carried();
        }
    }

    /// Keeps an answer that carried a definition, its blocks freed at its end, and ties each definition to it.
    fn keep(self, blocks: KeptBlocks) -> Arc<KeptAnswer> {
        let scopes = self
            .definitions
            .iter()
            .map(|(_, scope)| scope.clone())
            .collect();
        let kept = KeptAnswer::keep(self.target.holdings, scopes, blocks);
        for (definition, _) in &self.definitions {
            definition.kept_by(kept.clone());
        }
        kept
    }
}
