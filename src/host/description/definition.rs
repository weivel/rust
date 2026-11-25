//! One description bound once at the load, and every definition struct it describes bound through it: a library's
//! service, a definition an answer carried, or one its caller bound with no library behind it.

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::fmt;
use std::sync::{Arc, OnceLock};

use super::abi::{KIND_CONTEXT, RETURNS_NOTHING, RETURNS_STATUS};
use super::described::Description;
use super::enums::enums_of;
use super::holdings::{CarriedScope, Holdings, KeptAnswer, Mapping};
use super::invoke::ReleaseCall;
use super::layouts::{LayoutRefusal, Layouts};
use super::memory::{definition_size, definition_word};
use super::operations::{CallTarget, CarriedBinder, OperationPlan};
use super::reader;
use super::value::{DescribedAnswer, DescribedCallError, Inputs};
use crate::context::DescribedEnums;
use crate::core::{Answer, DescribedDefinition};

/// One description, bound: its layouts, one plan per operation, its enums and the bindings of the definitions it
/// nests, built once and shared by every definition it binds.
pub(crate) struct Binding {
    description: Description,
    layouts: Layouts,
    operations: Vec<OperationPlan>,
    releases: Vec<(String, usize)>,
    nested: BTreeMap<String, Arc<Binding>>,
    enums: DescribedEnums,
    blocking: Vec<String>,
}

impl Binding {
    /// Binds `description`, refusing one this platform's layout disagrees with or this binding cannot call.
    pub(crate) fn new(description: Description) -> Result<Arc<Binding>, LayoutRefusal> {
        let refuse = |detail: String| {
            format!(
                "the description of {} cannot be bound: {detail}",
                description.definition
            )
        };
        if size_of::<usize>() != size_of::<u64>() {
            return Err(refuse(
                "this binding passes every integer and pointer in a 64-bit word, and this platform's \
                 pointers are not 64 bits"
                    .to_owned(),
            ));
        }
        let layouts = Layouts::new(&description)?;
        let enums = enums_of(&description).map_err(refuse)?;
        let nested = description
            .definitions
            .iter()
            .map(|nested| Ok((nested.definition.clone(), Binding::new(nested.clone())?)))
            .collect::<Result<BTreeMap<_, _>, LayoutRefusal>>()?;
        let operations = description
            .operations
            .iter()
            .filter(|operation| operation.returns == RETURNS_STATUS)
            .map(|operation| OperationPlan::new(operation, &layouts).map_err(&refuse))
            .collect::<Result<Vec<_>, _>>()?;
        let releases = description
            .operations
            .iter()
            .filter(|operation| operation.returns == RETURNS_NOTHING)
            .map(|operation| (operation.name.clone(), operation.offset))
            .collect();
        let blocking = operations
            .iter()
            .filter(|plan| description.blocking.contains(&plan.name))
            .map(|plan| plan.name.clone())
            .collect();
        Ok(Arc::new(Binding {
            description,
            layouts,
            operations,
            releases,
            nested,
            enums,
            blocking,
        }))
    }

    /// Binds the definition's struct at `address` in `environment`: its context, every operation's function and every
    /// release read once from it, within the `size` it answered.
    fn bind(
        self: &Arc<Binding>,
        address: usize,
        environment: Environment,
    ) -> Result<BoundDefinition, String> {
        let definition = &self.description.definition;
        let Some(laid_out) = self.layouts.laid_out(Some(definition)) else {
            return Err(format!(
                "the description of {definition} describes no struct {definition}"
            ));
        };
        let answered_size = definition_size(address);
        let first_shape = laid_out.described.first_shape;
        if answered_size < first_shape {
            return Err(format!(
                "the library answered a {definition} of size {answered_size}, below its first shape of {first_shape}"
            ));
        }
        let context = laid_out
            .described
            .members
            .iter()
            .find(|member| member.kind == KIND_CONTEXT)
            .map_or(0, |member| {
                definition_word(address, answered_size, member.offset)
            });
        let functions = self
            .operations
            .iter()
            .map(|plan| definition_word(address, answered_size, plan.offset))
            .collect();
        let releases = self
            .releases
            .iter()
            .map(|(name, offset)| {
                let function = definition_word(address, answered_size, *offset);
                (name.clone(), ReleaseCall::new(function, context))
            })
            .collect();
        Ok(BoundDefinition {
            inner: Arc::new(BoundInner {
                binding: self.clone(),
                address,
                context,
                functions,
                releases,
                environment,
                keeper: OnceLock::new(),
            }),
        })
    }
}

/// What a bound definition shares with what it was got from: its library's binary, the holdings everything got
/// through it lives in, and, for a definition an answer carried, the scope that answer ends.
pub(crate) struct Environment {
    pub(crate) mapping: Mapping,
    pub(crate) holdings: Arc<Holdings>,
    pub(crate) scope: Option<Arc<CarriedScope>>,
}

struct BoundInner {
    binding: Arc<Binding>,
    address: usize,
    context: usize,
    functions: Vec<usize>,
    releases: BTreeMap<String, ReleaseCall>,
    environment: Environment,
    /// The answer that carried it, kept alive while it lives.
    keeper: OnceLock<Arc<KeptAnswer>>,
}

/// A C library's definition bound from its description at the load: one operation per function pointer of the
/// definition that answers a status, each called by name with [`Inputs`] keyed by parameter name, its answers
/// arriving as [`Value`](super::Value)s. Its releases are the binding's: every answer is released before its call
/// answers but one carrying a definition, and a handle through its own release.
///
/// It stands for the very struct its definition's symbol answered, at [`BoundDefinition::address`]. A definition an
/// answer carried is bound too, and lives, with everything got through it, as long as that answer: once the answer has
/// ended, a call through it is the caller's mistake. Every operation runs on the calling thread, those its
/// description marks blocking too, which [`BoundDefinition::blocking_operations`] names. Cloning it shares it.
#[derive(Clone)]
pub struct BoundDefinition {
    inner: Arc<BoundInner>,
}

impl fmt::Debug for BoundDefinition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "BoundDefinition({} at {:#x}, {:?})",
            self.name(),
            self.address(),
            self.operation_names()
        )
    }
}

impl PartialEq for BoundDefinition {
    fn eq(&self, other: &BoundDefinition) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl BoundDefinition {
    /// Binds a definition its caller got some other way than from a library's service, as one a library serves is
    /// bound. No library stands behind it, so nothing is released for its caller at an unload: every handle it
    /// answers, and every answer carrying a definition, lives until its own release or the drop of its last part.
    ///
    /// Answers the caller's mistake for a description this platform's layout disagrees with or this binding cannot
    /// bind, or a definition whose struct ends before its first shape.
    ///
    /// # Safety
    /// `definition` is the definition's struct and `description` its `const weivel_standard_description *`, which
    /// `weivel_standard_description_check` has passed; both outlive the bound definition and everything got through
    /// it. The description is read once here and never checked again.
    pub unsafe fn bind(
        definition: *const c_void,
        description: *const c_void,
    ) -> Result<BoundDefinition, Answer> {
        // SAFETY: as the caller promised.
        let read = unsafe { reader::read(description as usize) };
        let environment = Environment {
            mapping: Mapping::Unowned,
            holdings: Holdings::of_a_library(),
            scope: None,
        };
        read.and_then(Binding::new)
            .and_then(|binding| binding.bind(definition as usize, environment))
            .map_err(Answer::Mistake)
    }

    /// Binds a definition the host across the plugin seam bound from the same description, its binary held mapped
    /// through `mapping` across each call and release, and everything got through it among `holdings`, which the
    /// host's unload of that library releases while the binary is still mapped.
    ///
    /// # Safety
    /// `definition` and `description` are what the host's library answered, which the standard's check passed, and
    /// the binary is mapped for the length of this call.
    pub(crate) unsafe fn bind_remote(
        definition: usize,
        description: usize,
        mapping: Mapping,
        holdings: Arc<Holdings>,
    ) -> Result<BoundDefinition, Answer> {
        // SAFETY: as the caller promised.
        let read = unsafe { reader::read(description) };
        let environment = Environment {
            mapping,
            holdings,
            scope: None,
        };
        read.and_then(Binding::new)
            .and_then(|binding| binding.bind(definition, environment))
            .map_err(Answer::Mistake)
    }

    /// The struct its definition is, as its description names it.
    pub fn name(&self) -> &str {
        &self.inner.binding.description.definition
    }

    /// The address its definition's symbol answered, which a host registers a definition with no description as.
    pub fn address(&self) -> usize {
        self.inner.address
    }

    /// Every operation's name, in the struct's order, those its provider does not offer included.
    pub fn operation_names(&self) -> Vec<&str> {
        self.inner
            .binding
            .operations
            .iter()
            .map(|plan| plan.name.as_str())
            .collect()
    }

    /// The operations its description marks blocking, which may do I/O or wait, in the struct's order; this binding
    /// calls them on the calling thread as it calls every other, so a caller that must not wait hands such a call to
    /// a thread of its own.
    pub fn blocking_operations(&self) -> &[String] {
        &self.inner.binding.blocking
    }

    /// The enums its description lists, with `weivel_status` beside them.
    pub fn enums(&self) -> &DescribedEnums {
        &self.inner.binding.enums
    }

    /// Calls the operation `operation` with `inputs`, each keyed by its parameter's name, a left-out input at its
    /// kind's zero.
    ///
    /// Answers [`DescribedAnswer::Answered`] on OK and [`DescribedAnswer::None`] on NONE; any other status as a
    /// [`DescribedCallError`] carrying it, with the provider's `detail` and `fix` for REFUSED. The caller's mistake,
    /// calling nothing, for an operation the definition has none of or its provider does not offer, an input it does
    /// not take, a value its kind does not hold, a released handle, or a definition whose answer has ended;
    /// [`CallStatus::Unloaded`](super::CallStatus::Unloaded) once the library's binary has closed.
    pub fn call(
        &self,
        operation: &str,
        inputs: &Inputs,
    ) -> Result<DescribedAnswer, DescribedCallError> {
        let binding = &self.inner.binding;
        let Some(index) = binding
            .operations
            .iter()
            .position(|plan| plan.name == operation)
        else {
            return Err(DescribedCallError::mistake(
                operation,
                format!(
                    "{} has no operation {operation}; it has {}",
                    self.name(),
                    self.operation_names().join(", ")
                ),
            ));
        };
        let environment = &self.inner.environment;
        let _running = match &environment.scope {
            Some(scope) => Some(scope.enter().ok_or_else(|| {
                DescribedCallError::mistake(
                    operation,
                    format!(
                        "{} was carried by an answer that has ended, and lives only as long as that answer",
                        self.name()
                    ),
                )
            })?),
            None => None,
        };
        let target = CallTarget {
            layouts: &binding.layouts,
            function: self.inner.functions[index],
            context: self.inner.context,
            releases: &self.inner.releases,
            mapping: &environment.mapping,
            holdings: &environment.holdings,
            carrier: &*self.inner,
        };
        binding.operations[index].call(&target, inputs)
    }

    /// The plan of `operation`, for the suite translating a vector's values by the kinds it takes.
    pub(crate) fn plan(&self, operation: &str) -> Option<&OperationPlan> {
        self.inner
            .binding
            .operations
            .iter()
            .find(|plan| plan.name == operation)
    }

    pub(crate) fn layouts(&self) -> &Layouts {
        &self.inner.binding.layouts
    }

    /// Ends a carried definition whose answer was never handed to its caller.
    pub(crate) fn end_carried(&self) {
        if let Some(scope) = &self.inner.environment.scope {
            scope.end(Box::new(|| {}));
        }
    }

    /// Ties a carried definition to the answer that carried it, which then lives at least as long.
    pub(crate) fn kept_by(&self, kept: Arc<KeptAnswer>) {
        let _ = self.inner.keeper.set(kept);
    }
}

impl CarriedBinder for BoundInner {
    fn bind_carried(
        &self,
        type_name: &str,
        pointer: usize,
    ) -> Result<(BoundDefinition, Arc<CarriedScope>), String> {
        let nested = self.binding.nested.get(type_name).ok_or_else(|| {
            format!(
                "an answer carries a {type_name}, which the description of {} nests no description of",
                self.binding.description.definition
            )
        })?;
        let scope = CarriedScope::within(&self.environment.holdings);
        let environment = Environment {
            mapping: self.environment.mapping.clone(),
            holdings: scope.holdings.clone(),
            scope: Some(scope.clone()),
        };
        let bound = nested.bind(pointer, environment)?;
        Ok((bound, scope))
    }
}

/// Binds the definition a library's service declared a description for, among the library's `holdings`.
///
/// Its caller hands only a pair its library answered, whose description the host's check passed, while the binary is
/// mapped.
pub(crate) fn bind_described(
    described: &DescribedDefinition,
    mapping: Mapping,
    holdings: Arc<Holdings>,
) -> Result<BoundDefinition, String> {
    // SAFETY: as the caller promised: the check passed the description, and the library is open.
    let read = unsafe { reader::read(described.description_address()) }?;
    let environment = Environment {
        mapping,
        holdings,
        scope: None,
    };
    Binding::new(read)?.bind(described.definition_address(), environment)
}
