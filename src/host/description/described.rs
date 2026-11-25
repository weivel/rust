//! A description as Rust values, read once at the load from the struct its library answered.

use super::abi::{KIND_CONTEXT, KIND_STORAGE, KIND_U32};

/// One member of a described struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Member {
    pub(crate) name: String,
    pub(crate) offset: usize,
    pub(crate) size: usize,
    pub(crate) kind: u32,
    pub(crate) element: u32,
    pub(crate) type_name: Option<String>,
    pub(crate) count: Option<String>,
    pub(crate) release: Option<String>,
}

/// One described struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Type {
    pub(crate) name: String,
    pub(crate) size: usize,
    pub(crate) first_shape: usize,
    pub(crate) members: Vec<Member>,
}

/// One described enum, its entries as its header numbers them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Enum {
    pub(crate) name: String,
    pub(crate) entries: Vec<(String, u32)>,
}

/// One parameter of an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Parameter {
    pub(crate) name: String,
    pub(crate) kind: u32,
    pub(crate) role: u32,
    pub(crate) element: u32,
    pub(crate) type_name: Option<String>,
    pub(crate) count: Option<String>,
    pub(crate) release: Option<String>,
}

/// One function pointer of the definition, an operation or a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Operation {
    pub(crate) name: String,
    pub(crate) offset: usize,
    pub(crate) returns: u32,
    pub(crate) parameters: Vec<Parameter>,
}

/// A whole description: its structs, enums and operations, the operations it marks blocking, and the description of
/// every definition it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Description {
    pub(crate) definition: String,
    pub(crate) types: Vec<Type>,
    pub(crate) enums: Vec<Enum>,
    pub(crate) operations: Vec<Operation>,
    pub(crate) blocking: Vec<String>,
    pub(crate) definitions: Vec<Description>,
}

impl Description {
    pub(crate) fn type_named(&self, name: &str) -> Option<&Type> {
        self.types.iter().find(|described| described.name == name)
    }
}

impl Member {
    /// A U32 called `size` at offset 0: the struct's own size, set by the binding and never answered.
    pub(crate) fn is_leading_size(&self) -> bool {
        self.kind == KIND_U32 && self.name == "size" && self.offset == 0
    }
}

impl Type {
    pub(crate) fn member_named(&self, name: &str) -> Option<&Member> {
        self.members.iter().find(|member| member.name == name)
    }

    fn counts(&self, member: &Member) -> bool {
        self.members
            .iter()
            .any(|other| other.count.as_deref() == Some(member.name.as_str()))
    }

    /// Whether a caller writes `member` and an answer carries it: all but a leading size, a count, the context and
    /// storage.
    pub(crate) fn is_visible(&self, member: &Member) -> bool {
        !member.is_leading_size()
            && !self.counts(member)
            && member.kind != KIND_CONTEXT
            && member.kind != KIND_STORAGE
    }

    pub(crate) fn leading_size(&self) -> Option<u32> {
        self.members
            .iter()
            .any(Member::is_leading_size)
            .then_some(self.size as u32)
    }
}
