//! What the crate declares by hand of the standard's headers, described so a test can hold it to the C compiler: the
//! layout of every struct of `description.h` and `value.h` the description binder reads and writes, and of the
//! `weivel_standard_refusal` the description checks fill.

use std::mem::offset_of;

use super::description::StandardRefusal;
use crate::core::declarations::{FieldLayout, StructLayout, pointee_size};
use crate::host::description_abi as abi;

/// One struct's layout, each field named as the header names it, which a Rust field cannot always be.
macro_rules! named_layout {
    ($type:ty as $name:literal { $($field:ident as $printed:literal),* $(,)? }) => {
        StructLayout {
            name: $name,
            size: size_of::<$type>(),
            fields: vec![$(FieldLayout {
                name: $printed,
                offset: offset_of!($type, $field),
                // SAFETY: the projection only names a field's place and is never called.
                size: pointee_size(|value: *const $type| unsafe { &raw const (*value).$field }),
            }),*],
        }
    };
}

/// Every struct of the standard's headers the crate declares, as it declares it.
pub fn standard_struct_layouts() -> Vec<StructLayout> {
    let mut layouts = description_layouts();
    layouts.extend(value_layouts());
    layouts.push(named_layout!(StandardRefusal as "weivel_standard_refusal" {
        size as "size", code as "code", detail as "detail", fix as "fix", path as "path", storage as "storage",
    }));
    layouts
}

fn description_layouts() -> Vec<StructLayout> {
    vec![
        named_layout!(abi::DescribedMember as "weivel_standard_described_member" {
            name as "name", offset as "offset", size as "size", kind as "kind", element as "element",
            type_name as "type", count as "count", release as "release",
        }),
        named_layout!(abi::DescribedType as "weivel_standard_described_type" {
            name as "name", size as "size", first_shape as "first_shape", members as "members",
            member_count as "member_count",
        }),
        named_layout!(abi::DescribedEnumEntry as "weivel_standard_described_enum_entry" {
            name as "name", number as "number",
        }),
        named_layout!(abi::DescribedEnum as "weivel_standard_described_enum" {
            name as "name", entries as "entries", entry_count as "entry_count",
        }),
        named_layout!(abi::DescribedParameter as "weivel_standard_described_parameter" {
            name as "name", kind as "kind", role as "role", element as "element", type_name as "type",
            count as "count", release as "release",
        }),
        named_layout!(abi::DescribedOperation as "weivel_standard_described_operation" {
            name as "name", offset as "offset", returns as "returns", parameters as "parameters",
            parameter_count as "parameter_count",
        }),
        named_layout!(abi::Description as "weivel_standard_description" {
            size as "size", definition as "definition", types as "types", type_count as "type_count",
            enums as "enums", enum_count as "enum_count", operations as "operations",
            operation_count as "operation_count", blocking as "blocking", blocking_count as "blocking_count",
            definitions as "definitions", definition_count as "definition_count", answered_by as "answered_by",
        }),
    ]
}

fn value_layouts() -> Vec<StructLayout> {
    vec![
        named_layout!(abi::ValueNode as "weivel_standard_value" {
            kind as "kind", boolean as "boolean", number as "number", text as "text", length as "length",
            items as "items", entries as "entries", count as "count",
        }),
        named_layout!(abi::ValueEntry as "weivel_standard_entry" {
            key as "key", key_length as "key_length", value as "value",
        }),
        named_layout!(abi::ValueTree as "weivel_standard_value_tree" {
            size as "size", root as "root", storage as "storage",
        }),
    ]
}
