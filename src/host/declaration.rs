//! A C library's declaration as a value, and what of one is the caller's mistake before anything is opened.

use std::collections::HashSet;

/// How a library function is called, as the standard's `library.h` names the two kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionKind {
    /// Opaque bytes in and opaque bytes out, `WEIVEL_STANDARD_FUNCTION_BYTES`.
    Bytes,
    /// One service's whole definition, `WEIVEL_STANDARD_FUNCTION_DEFINITION`.
    Definition,
}

/// One declared function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryFunction {
    /// The name a plugin calls it by.
    pub name: String,
    /// The exported C symbol that implements it.
    pub symbol: String,
    /// How it is called.
    pub kind: FunctionKind,
    /// The facts of the calling process it reads, as `weivel_standard_context_fact` bits; 0 for none.
    pub context: u32,
}

/// One declared service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryServiceDeclaration {
    /// The service id plugins request.
    pub id: String,
    /// Its functions: bytes functions, or one definition.
    pub functions: Vec<LibraryFunction>,
    /// The symbol answering its definition described, for a definition's service that declares one.
    pub description: Option<String>,
}

/// A description the library publishes for plugins to implement under a capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryContract {
    /// The capability id a plugin implementing the contract provides.
    pub capability: String,
    /// The symbol answering the description.
    pub description: String,
}

/// A C library's declaration, as the standard reads it or the caller builds it. It names no path: the caller hands
/// the library's path beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryDeclaration {
    /// The name plugins import it by.
    pub import_name: String,
    /// Its version.
    pub version: String,
    /// The symbol that frees what its bytes functions answer; `None` when every service is a definition.
    pub release: Option<String>,
    /// The services its functions offer.
    pub services: Vec<LibraryServiceDeclaration>,
    /// The contracts it publishes; the host keeps them on the declaration and binds nothing from them.
    pub contracts: Vec<LibraryContract>,
}

impl LibraryServiceDeclaration {
    /// Whether the service declares a definition, its one function.
    pub fn is_a_definition(&self) -> bool {
        self.functions
            .first()
            .is_some_and(|function| function.kind == FunctionKind::Definition)
    }
}

/// What of `declaration` is the caller's mistake, found before anything is opened: a declaration of neither a service
/// nor a contract or a service of no function, which the standard's reader refuses too, so a declaration the caller
/// builds cannot load what a read one never could; an id listed twice; a definition beside another function or
/// reading context facts; a description beside bytes functions or naming no symbol; a function name a service lists
/// twice; or a bytes function in a library with no release symbol.
pub(super) fn mistake_in(declaration: &LibraryDeclaration) -> Option<String> {
    if declaration.services.is_empty() && declaration.contracts.is_empty() {
        return Some(format!(
            "the library {} declares neither a service nor a contract",
            declaration.import_name
        ));
    }
    let mut ids = HashSet::new();
    for service in &declaration.services {
        if !ids.insert(service.id.as_str()) {
            return Some(format!(
                "the declaration lists the service id {} twice",
                service.id
            ));
        }
        if let Some(mistake) = mistake_in_service(service) {
            return Some(mistake);
        }
        if declaration.release.is_none() && !service.is_a_definition() {
            return Some(format!(
                "the service {} declares a bytes function, and the library {} declares no release symbol to free \
                 what it answers",
                service.id, declaration.import_name
            ));
        }
    }
    None
}

fn mistake_in_service(service: &LibraryServiceDeclaration) -> Option<String> {
    let id = &service.id;
    if service.functions.is_empty() {
        return Some(format!("the service {id} declares no function"));
    }
    match &service.description {
        Some(symbol) if symbol.is_empty() => {
            return Some(format!(
                "the service {id} declares an empty description symbol"
            ));
        }
        Some(_) if !service.is_a_definition() => {
            return Some(format!(
                "the service {id} declares a description beside bytes functions; only a definition is described"
            ));
        }
        _ => {}
    }
    let mut names = HashSet::new();
    for function in &service.functions {
        if function.kind == FunctionKind::Definition && service.functions.len() > 1 {
            return Some(format!(
                "the service {id} declares the definition {} beside another function; a definition is its \
                 service's one function",
                function.name
            ));
        }
        if function.kind == FunctionKind::Definition && function.context != 0 {
            return Some(format!(
                "the definition {} of the service {id} reads context facts; a definition's struct carries its \
                 own context",
                function.name
            ));
        }
        if !names.insert(function.name.as_str()) {
            return Some(format!(
                "the service {id} lists the function {} twice",
                function.name
            ));
        }
    }
    None
}
