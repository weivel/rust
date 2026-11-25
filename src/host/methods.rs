//! Methods by name: what a service object offers to be called by a name its caller reads at run time, and the actions
//! a plugin's entry offers, since Rust reflects no method of an object.

use std::collections::BTreeMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use super::description::Tree;
use crate::context::{ForeignObject, Object};
use crate::core::panic_text;

/// How a method ended: the value tree it answered, `None` for nothing, which is a value of kind NONE, or why it
/// failed, in English.
pub type MethodResult = Result<Option<Tree>, String>;

/// One method: handed a value tree, `None` for a value of kind NONE, and answering how it ended.
pub type Method = Arc<dyn Fn(Option<&Tree>) -> MethodResult + Send + Sync>;

/// Named methods, each taking and answering a value tree. A service that offers methods by name registers a
/// `Methods` as its object, so a host program's plan can name one (`weivel/host.h` of the standard), and a capability
/// implementation offering the operations of a contract is one: the operations it offers are its names. Cloning it
/// shares its methods.
#[derive(Clone, Default)]
pub struct Methods {
    by_name: BTreeMap<String, Method>,
}

impl fmt::Debug for Methods {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_set().entries(self.by_name.keys()).finish()
    }
}

impl Methods {
    /// No methods.
    pub fn new() -> Methods {
        Methods::default()
    }

    /// These methods and `method` under `name`, which replaces one of that name.
    pub fn with(
        mut self,
        name: impl Into<String>,
        method: impl Fn(Option<&Tree>) -> MethodResult + Send + Sync + 'static,
    ) -> Methods {
        self.insert(name, method);
        self
    }

    /// Adds `method` under `name`, replacing one of that name.
    pub fn insert(
        &mut self,
        name: impl Into<String>,
        method: impl Fn(Option<&Tree>) -> MethodResult + Send + Sync + 'static,
    ) {
        self.by_name.insert(name.into(), Arc::new(method));
    }

    /// The name of every method, in order.
    pub fn names(&self) -> Vec<String> {
        self.by_name.keys().cloned().collect()
    }

    /// Whether it offers a method `name`.
    pub fn offers(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Calls the method `name` with `arguments` and answers how it ended, a panic as a failure saying so; `None` when
    /// it offers no method of that name, so nothing was called.
    pub fn call(&self, name: &str, arguments: Option<&Tree>) -> Option<MethodResult> {
        let method = self.by_name.get(name)?;
        Some(
            catch_unwind(AssertUnwindSafe(|| method(arguments))).unwrap_or_else(|payload| {
                Err(format!(
                    "the method {name} panicked: {}",
                    panic_text(&*payload)
                ))
            }),
        )
    }
}

/// The methods `object` offers by name: its own when it is a [`Methods`], and those a plugin across the plugin seam
/// said its object offers when it is a [`ForeignObject`] that crossed with them; `None` for any other object.
pub fn methods_of(object: &Object) -> Option<Methods> {
    if let Some(methods) = object.downcast_ref::<Methods>() {
        return Some(methods.clone());
    }
    object
        .downcast_ref::<ForeignObject>()
        .and_then(|foreign| foreign.methods().cloned())
}
