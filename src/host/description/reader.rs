//! Reads a description the standard's check has passed into Rust values: every array it names is as long as its count
//! says, every name it requires is set, and its nested descriptions hold no cycle.

use std::ffi::{CStr, c_char};
use std::mem::offset_of;

use super::abi;
use super::described::{Description, Enum, Member, Operation, Parameter, Type};

/// The text at `pointer`, or none for NULL.
///
/// # Safety
/// `pointer` is NULL or a NUL-terminated text that lives across the call.
unsafe fn optional_text(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: as the caller promised.
    Some(
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// # Safety
/// As [`optional_text`]; the check refuses a NULL name, so one is read as empty only defensively.
unsafe fn text(pointer: *const c_char) -> String {
    // SAFETY: as the caller promised.
    unsafe { optional_text(pointer) }.unwrap_or_default()
}

/// The `count` items at `items`, or none for NULL.
///
/// # Safety
/// `items` is NULL or points at `count` initialised items that live across the call.
unsafe fn items<'a, T>(items: *const T, count: usize) -> &'a [T] {
    if items.is_null() || count == 0 {
        return &[];
    }
    // SAFETY: as the caller promised.
    unsafe { std::slice::from_raw_parts(items, count) }
}

fn member_of(member: &abi::DescribedMember) -> Member {
    // SAFETY: every text a checked description names is NUL-terminated and lives as long as its library.
    unsafe {
        Member {
            name: text(member.name),
            offset: member.offset,
            size: member.size,
            kind: member.kind,
            element: member.element,
            type_name: optional_text(member.type_name),
            count: optional_text(member.count),
            release: optional_text(member.release),
        }
    }
}

fn type_of(described: &abi::DescribedType) -> Type {
    // SAFETY: a checked description's arrays are as long as their counts and live as long as their library.
    let members = unsafe { items(described.members, described.member_count) };
    Type {
        // SAFETY: as for `member_of`.
        name: unsafe { text(described.name) },
        size: described.size,
        first_shape: described.first_shape,
        members: members.iter().map(member_of).collect(),
    }
}

fn enum_of(described: &abi::DescribedEnum) -> Enum {
    // SAFETY: as for `type_of`.
    let entries = unsafe { items(described.entries, described.entry_count) };
    Enum {
        // SAFETY: as for `member_of`.
        name: unsafe { text(described.name) },
        entries: entries
            .iter()
            // SAFETY: as for `member_of`.
            .map(|entry| (unsafe { text(entry.name) }, entry.number))
            .collect(),
    }
}

fn parameter_of(parameter: &abi::DescribedParameter) -> Parameter {
    // SAFETY: as for `member_of`.
    unsafe {
        Parameter {
            name: text(parameter.name),
            kind: parameter.kind,
            role: parameter.role,
            element: parameter.element,
            type_name: optional_text(parameter.type_name),
            count: optional_text(parameter.count),
            release: optional_text(parameter.release),
        }
    }
}

fn operation_of(operation: &abi::DescribedOperation) -> Operation {
    // SAFETY: as for `type_of`.
    let parameters = unsafe { items(operation.parameters, operation.parameter_count) };
    Operation {
        // SAFETY: as for `member_of`.
        name: unsafe { text(operation.name) },
        offset: operation.offset,
        returns: operation.returns,
        parameters: parameters.iter().map(parameter_of).collect(),
    }
}

/// Whether a description that answered `size` reaches the end of the field at `offset` of `field_size` bytes.
fn reaches(size: u32, offset: usize, field_size: usize) -> bool {
    offset + field_size <= size as usize
}

/// Reads the description at `address`. A field past the `size` it answered reads as none, so a description of an
/// earlier shape blocks nothing and nests nothing.
///
/// # Safety
/// `address` is a description `weivel_standard_description_check` passed, from a library still open.
pub(crate) unsafe fn read(address: usize) -> Description {
    // SAFETY: as the caller promised; the check proved it reaches its first shape, which ends at `operation_count`.
    let described = unsafe { &*(address as *const abi::Description) };
    let size = described.size;
    let pointer = size_of::<usize>();
    let blocking = if reaches(size, offset_of!(abi::Description, blocking_count), pointer) {
        // SAFETY: as for `type_of`.
        unsafe { items(described.blocking, described.blocking_count) }
    } else {
        &[]
    };
    let definitions = if reaches(
        size,
        offset_of!(abi::Description, definition_count),
        pointer,
    ) {
        // SAFETY: as for `type_of`.
        unsafe { items(described.definitions, described.definition_count) }
    } else {
        &[]
    };
    // SAFETY: as for `type_of`.
    let (types, enums, operations) = unsafe {
        (
            items(described.types, described.type_count),
            items(described.enums, described.enum_count),
            items(described.operations, described.operation_count),
        )
    };
    Description {
        // SAFETY: as for `member_of`.
        definition: unsafe { text(described.definition) },
        types: types.iter().map(type_of).collect(),
        enums: enums.iter().map(enum_of).collect(),
        operations: operations.iter().map(operation_of).collect(),
        // SAFETY: as for `member_of`.
        blocking: blocking.iter().map(|name| unsafe { text(*name) }).collect(),
        definitions: definitions
            .iter()
            // SAFETY: the check checked every nested description as a whole one and refused a cycle.
            .map(|nested| unsafe { read(*nested as usize) })
            .collect(),
    }
}
