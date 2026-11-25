//! The standard's `description.h` and `value.h`, declared by hand: every struct the binder reads a description
//! through and every value node it writes and reads, held to the standard's `layouts` program by a test.

use std::ffi::{c_char, c_void};

pub(crate) const KIND_U32: u32 = 1;
pub(crate) const KIND_U64: u32 = 2;
pub(crate) const KIND_I64: u32 = 3;
pub(crate) const KIND_F64: u32 = 4;
pub(crate) const KIND_SIZE: u32 = 5;
pub(crate) const KIND_TEXT: u32 = 6;
pub(crate) const KIND_TEXTS: u32 = 7;
pub(crate) const KIND_BYTES: u32 = 8;
pub(crate) const KIND_STRUCT: u32 = 9;
pub(crate) const KIND_POINTER: u32 = 10;
pub(crate) const KIND_ARRAY: u32 = 11;
pub(crate) const KIND_VALUE: u32 = 12;
pub(crate) const KIND_HANDLE: u32 = 13;
pub(crate) const KIND_CONTEXT: u32 = 14;
pub(crate) const KIND_STORAGE: u32 = 15;
pub(crate) const KIND_DEFINITION: u32 = 16;
/// The highest kind this binding knows; a description holding a higher one is refused whole.
pub(crate) const KIND_LAST: u32 = KIND_DEFINITION;

pub(crate) const ROLE_INPUT: u32 = 1;
pub(crate) const ROLE_ANSWER: u32 = 2;
pub(crate) const ROLE_REFUSAL: u32 = 3;

pub(crate) const RETURNS_STATUS: u32 = 1;
pub(crate) const RETURNS_NOTHING: u32 = 2;

pub(crate) const VALUE_NONE: u32 = 0;
pub(crate) const VALUE_NULL: u32 = 1;
pub(crate) const VALUE_BOOLEAN: u32 = 2;
pub(crate) const VALUE_NUMBER: u32 = 3;
pub(crate) const VALUE_TEXT: u32 = 4;
pub(crate) const VALUE_LIST: u32 = 5;
pub(crate) const VALUE_RECORD: u32 = 6;

/// `WEIVEL_STANDARD_VALUE_DEPTH_LIMIT`, which bounds a value tree and every walk over structs as well.
pub(crate) const DEPTH_LIMIT: usize = 1000;

/// `weivel_standard_described_member`.
#[repr(C)]
pub(crate) struct DescribedMember {
    pub(crate) name: *const c_char,
    pub(crate) offset: usize,
    pub(crate) size: usize,
    pub(crate) kind: u32,
    pub(crate) element: u32,
    pub(crate) type_name: *const c_char,
    pub(crate) count: *const c_char,
    pub(crate) release: *const c_char,
}

/// `weivel_standard_described_type`.
#[repr(C)]
pub(crate) struct DescribedType {
    pub(crate) name: *const c_char,
    pub(crate) size: usize,
    pub(crate) first_shape: usize,
    pub(crate) members: *const DescribedMember,
    pub(crate) member_count: usize,
}

/// `weivel_standard_described_enum_entry`.
#[repr(C)]
pub(crate) struct DescribedEnumEntry {
    pub(crate) name: *const c_char,
    pub(crate) number: u32,
}

/// `weivel_standard_described_enum`.
#[repr(C)]
pub(crate) struct DescribedEnum {
    pub(crate) name: *const c_char,
    pub(crate) entries: *const DescribedEnumEntry,
    pub(crate) entry_count: usize,
}

/// `weivel_standard_described_parameter`.
#[repr(C)]
pub(crate) struct DescribedParameter {
    pub(crate) name: *const c_char,
    pub(crate) kind: u32,
    pub(crate) role: u32,
    pub(crate) element: u32,
    pub(crate) type_name: *const c_char,
    pub(crate) count: *const c_char,
    pub(crate) release: *const c_char,
}

/// `weivel_standard_described_operation`.
#[repr(C)]
pub(crate) struct DescribedOperation {
    pub(crate) name: *const c_char,
    pub(crate) offset: usize,
    pub(crate) returns: u32,
    pub(crate) parameters: *const DescribedParameter,
    pub(crate) parameter_count: usize,
}

/// `weivel_standard_description`.
#[repr(C)]
pub(crate) struct Description {
    pub(crate) size: u32,
    pub(crate) definition: *const c_char,
    pub(crate) types: *const DescribedType,
    pub(crate) type_count: usize,
    pub(crate) enums: *const DescribedEnum,
    pub(crate) enum_count: usize,
    pub(crate) operations: *const DescribedOperation,
    pub(crate) operation_count: usize,
    pub(crate) blocking: *const *const c_char,
    pub(crate) blocking_count: usize,
    pub(crate) definitions: *const *const Description,
    pub(crate) definition_count: usize,
    pub(crate) answered_by: Option<unsafe extern "C" fn() -> *const c_void>,
}

/// `weivel_standard_value`, one node of a value tree.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ValueNode {
    pub(crate) kind: u32,
    pub(crate) boolean: u32,
    pub(crate) number: f64,
    pub(crate) text: *const c_char,
    pub(crate) length: usize,
    pub(crate) items: *const ValueNode,
    pub(crate) entries: *const ValueEntry,
    pub(crate) count: usize,
}

/// `weivel_standard_entry`, one member of a record.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ValueEntry {
    pub(crate) key: *const c_char,
    pub(crate) key_length: usize,
    pub(crate) value: ValueNode,
}

/// `weivel_standard_value_tree`, the answer a VALUE parameter is handed.
#[repr(C)]
pub(crate) struct ValueTree {
    pub(crate) size: u32,
    pub(crate) root: ValueNode,
    pub(crate) storage: *mut c_void,
}

impl ValueNode {
    /// A node of kind NONE, every field zero.
    pub(crate) const NONE: ValueNode = ValueNode {
        kind: VALUE_NONE,
        boolean: 0,
        number: 0.0,
        text: std::ptr::null(),
        length: 0,
        items: std::ptr::null(),
        entries: std::ptr::null(),
        count: 0,
    };
}
