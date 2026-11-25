//! Reads a description the standard's check has passed into Rust values: every array it names is as long as its count
//! says, every name it requires is set, and its nested descriptions hold no cycle. Every name must be UTF-8, which
//! `description.h` requires and the check does not police, so a name that is not refuses the load.

use std::ffi::{CStr, c_char};
use std::mem::offset_of;

use super::abi;
use super::described::{Description, Enum, Member, Operation, Parameter, Type};

/// Why a description could not be read: the caller's mistake.
type Unreadable = String;

/// The text at `pointer`, or none for NULL.
///
/// # Safety
/// `pointer` is NULL or a NUL-terminated text that lives across the call.
unsafe fn optional_text(pointer: *const c_char) -> Result<Option<String>, Unreadable> {
    if pointer.is_null() {
        return Ok(None);
    }
    // SAFETY: as the caller promised.
    let text = unsafe { CStr::from_ptr(pointer) };
    text.to_str()
        .map(|text| Some(text.to_owned()))
        .map_err(|_| format!("the description names {text:?}, which is not UTF-8"))
}

/// # Safety
/// As [`optional_text`]; the check refuses a NULL name, so one is read as empty only defensively.
unsafe fn text(pointer: *const c_char) -> Result<String, Unreadable> {
    // SAFETY: as the caller promised.
    Ok(unsafe { optional_text(pointer) }?.unwrap_or_default())
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

fn member_of(member: &abi::DescribedMember) -> Result<Member, Unreadable> {
    // SAFETY: every text a checked description names is NUL-terminated and lives as long as its library.
    unsafe {
        Ok(Member {
            name: text(member.name)?.into(),
            offset: member.offset,
            size: member.size,
            kind: member.kind,
            element: member.element,
            type_name: optional_text(member.type_name)?,
            count: optional_text(member.count)?,
            release: optional_text(member.release)?,
        })
    }
}

fn type_of(described: &abi::DescribedType) -> Result<Type, Unreadable> {
    // SAFETY: a checked description's arrays are as long as their counts and live as long as their library.
    let members = unsafe { items(described.members, described.member_count) };
    Ok(Type {
        // SAFETY: as for `member_of`.
        name: unsafe { text(described.name) }?,
        size: described.size,
        first_shape: described.first_shape,
        members: members.iter().map(member_of).collect::<Result<_, _>>()?,
    })
}

fn enum_of(described: &abi::DescribedEnum) -> Result<Enum, Unreadable> {
    // SAFETY: as for `type_of`.
    let entries = unsafe { items(described.entries, described.entry_count) };
    Ok(Enum {
        // SAFETY: as for `member_of`.
        name: unsafe { text(described.name) }?,
        entries: entries
            .iter()
            // SAFETY: as for `member_of`.
            .map(|entry| Ok((unsafe { text(entry.name) }?, entry.number)))
            .collect::<Result<_, Unreadable>>()?,
    })
}

fn parameter_of(parameter: &abi::DescribedParameter) -> Result<Parameter, Unreadable> {
    // SAFETY: as for `member_of`.
    unsafe {
        Ok(Parameter {
            name: text(parameter.name)?.into(),
            kind: parameter.kind,
            role: parameter.role,
            element: parameter.element,
            type_name: optional_text(parameter.type_name)?,
            count: optional_text(parameter.count)?,
            release: optional_text(parameter.release)?,
        })
    }
}

fn operation_of(operation: &abi::DescribedOperation) -> Result<Operation, Unreadable> {
    // SAFETY: as for `type_of`.
    let parameters = unsafe { items(operation.parameters, operation.parameter_count) };
    Ok(Operation {
        // SAFETY: as for `member_of`.
        name: unsafe { text(operation.name) }?,
        offset: operation.offset,
        returns: operation.returns,
        parameters: parameters
            .iter()
            .map(parameter_of)
            .collect::<Result<_, _>>()?,
    })
}

/// Reads one field of the description at `described` through the raw pointer, never through a reference to the whole
/// struct, which may be shorter than this crate's declaration of it.
macro_rules! field {
    ($described:expr, $field:ident) => {
        // SAFETY: the caller of `read` promised the description reaches this field.
        unsafe { std::ptr::read_unaligned(&raw const (*$described).$field) }
    };
}

/// The description's array `items` of `count`, or none when the `size` it answered ends before the count.
///
/// # Safety
/// As [`read`]; `items` and `count` are offsets of a pointer and its count in the description.
unsafe fn optional_array<T>(
    described: *const abi::Description,
    size: u32,
    items_at: usize,
    count_at: usize,
) -> &'static [T] {
    if count_at + size_of::<usize>() > size as usize {
        return &[];
    }
    let base = described as usize;
    // SAFETY: both fields lie inside the `size` bytes the description answered.
    unsafe {
        let pointer = std::ptr::read_unaligned((base + items_at) as *const *const T);
        let count = std::ptr::read_unaligned((base + count_at) as *const usize);
        items(pointer, count)
    }
}

/// Reads the description at `address`. A field past the `size` it answered reads as none, so a description of an
/// earlier shape blocks nothing and nests nothing; `answered_by` is never read here. Answers why not for a name that
/// is not UTF-8.
///
/// # Safety
/// `address` is a description `weivel_standard_description_check` passed, from a library still open, so it reaches
/// its first shape, which ends at `operation_count`.
pub(crate) unsafe fn read(address: usize) -> Result<Description, Unreadable> {
    let described = address as *const abi::Description;
    let size: u32 = field!(described, size);
    let (types, type_count) = (field!(described, types), field!(described, type_count));
    let (enums, enum_count) = (field!(described, enums), field!(described, enum_count));
    let (operations, operation_count) = (
        field!(described, operations),
        field!(described, operation_count),
    );
    let definition = field!(described, definition);
    // SAFETY: as the caller promised.
    let (types, enums, operations, blocking, definitions) = unsafe {
        (
            items(types, type_count),
            items(enums, enum_count),
            items(operations, operation_count),
            optional_array::<*const c_char>(
                described,
                size,
                offset_of!(abi::Description, blocking),
                offset_of!(abi::Description, blocking_count),
            ),
            optional_array::<*const abi::Description>(
                described,
                size,
                offset_of!(abi::Description, definitions),
                offset_of!(abi::Description, definition_count),
            ),
        )
    };
    Ok(Description {
        address,
        // SAFETY: as for `member_of`.
        definition: unsafe { text(definition) }?,
        types: types.iter().map(type_of).collect::<Result<_, _>>()?,
        enums: enums.iter().map(enum_of).collect::<Result<_, _>>()?,
        operations: operations
            .iter()
            .map(operation_of)
            .collect::<Result<_, _>>()?,
        blocking: blocking
            .iter()
            // SAFETY: as for `member_of`.
            .map(|name| unsafe { text(*name) })
            .collect::<Result<_, _>>()?,
        definitions: definitions
            .iter()
            // SAFETY: the check checked every nested description as a whole one and refused a cycle.
            .map(|nested| unsafe { read(*nested as usize) })
            .collect::<Result<_, _>>()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A description of no types, enums or operations whose `size` ends at its first shape, its later fields set to
    /// what no read may follow.
    fn first_shape_only(definition: &'static [u8]) -> abi::Description {
        abi::Description {
            size: offset_of!(abi::Description, blocking) as u32,
            definition: definition.as_ptr().cast(),
            types: std::ptr::null(),
            type_count: 0,
            enums: std::ptr::null(),
            enum_count: 0,
            operations: std::ptr::null(),
            operation_count: 0,
            blocking: std::ptr::dangling(),
            blocking_count: usize::MAX,
            definitions: std::ptr::dangling(),
            definition_count: usize::MAX,
            answered_by: None,
        }
    }

    #[test]
    fn a_description_ending_at_its_first_shape_blocks_nothing_and_nests_nothing() {
        let described = first_shape_only(b"example\0");
        // SAFETY: the description lives across the read, and nothing past its `size` is read.
        let read = unsafe { read(&raw const described as usize) }.expect("the description reads");
        assert_eq!(read.definition, "example");
        assert!(read.blocking.is_empty() && read.definitions.is_empty());
    }

    #[test]
    fn a_name_that_is_not_utf8_refuses_the_description() {
        let described = first_shape_only(b"ex\xffample\0");
        // SAFETY: as above.
        let read = unsafe { read(&raw const described as usize) };
        assert!(read.is_err(), "{read:?}");
    }
}
