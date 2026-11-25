//! Every struct a description names, laid out by this platform's C rules from its members in order and held against
//! the description: a struct's size, and each member's offset and size, must be what the platform makes of them.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::c_void;

use super::abi::{
    KIND_ARRAY, KIND_BYTES, KIND_CONTEXT, KIND_DEFINITION, KIND_F64, KIND_HANDLE, KIND_I64,
    KIND_LAST, KIND_POINTER, KIND_SIZE, KIND_STORAGE, KIND_STRUCT, KIND_TEXT, KIND_TEXTS, KIND_U32,
    KIND_U64, KIND_VALUE, ValueNode,
};
use super::described::{Description, Member, Type};

/// Why a description cannot be bound on this platform: the caller's mistake.
pub(crate) type LayoutRefusal = String;

/// Where one field lies: its name, offset and size.
struct Field {
    name: String,
    offset: usize,
    size: usize,
    align: usize,
}

/// One struct, laid out and checked: its description, its alignment, which of its members a caller writes, and what
/// each call would otherwise look up by name.
#[derive(Debug)]
pub(crate) struct LaidOut {
    pub(crate) described: Type,
    pub(crate) align: usize,
    /// The members a caller writes and an answer carries, in the description's order.
    pub(crate) visible: Vec<Visible>,
    /// The size its leading `size` member is set to, if it has one.
    pub(crate) leading_size: Option<u32>,
}

/// One member a caller writes and an answer carries: its index among the struct's members, and the offset of the
/// member that counts it, if one does.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Visible {
    pub(crate) index: usize,
    pub(crate) count_offset: Option<usize>,
}

/// The layouts of every struct one description names, built once at the load.
#[derive(Debug)]
pub(crate) struct Layouts {
    types: BTreeMap<String, LaidOut>,
}

const POINTER_SIZE: usize = size_of::<*const c_void>();
const POINTER_ALIGN: usize = align_of::<*const c_void>();

fn round_up(offset: usize, align: usize) -> usize {
    offset.div_ceil(align) * align
}

fn is_pointer_sized(kind: u32) -> bool {
    matches!(
        kind,
        KIND_TEXT
            | KIND_TEXTS
            | KIND_BYTES
            | KIND_POINTER
            | KIND_ARRAY
            | KIND_HANDLE
            | KIND_CONTEXT
            | KIND_STORAGE
            | KIND_DEFINITION
    )
}

/// The size and alignment of a scalar kind, or none for any other.
pub(crate) fn scalar_layout(kind: u32) -> Option<(usize, usize)> {
    match kind {
        KIND_U32 => Some((size_of::<u32>(), align_of::<u32>())),
        KIND_U64 => Some((size_of::<u64>(), align_of::<u64>())),
        KIND_I64 => Some((size_of::<i64>(), align_of::<i64>())),
        KIND_F64 => Some((size_of::<f64>(), align_of::<f64>())),
        KIND_SIZE => Some((size_of::<usize>(), align_of::<usize>())),
        _ => None,
    }
}

struct Builder<'a> {
    description: &'a Description,
    built: BTreeMap<String, LaidOut>,
    building: BTreeSet<String>,
}

impl Builder<'_> {
    fn refuse(&self, detail: String) -> LayoutRefusal {
        format!(
            "the description of {} cannot be bound: {detail}",
            self.description.definition
        )
    }

    /// The size and alignment of a struct, laying it out first when it is not yet.
    fn struct_layout(&mut self, name: &str) -> Result<(usize, usize), LayoutRefusal> {
        if let Some(laid_out) = self.built.get(name) {
            return Ok((laid_out.described.size, laid_out.align));
        }
        let described = self
            .description
            .type_named(name)
            .ok_or_else(|| self.refuse(format!("it embeds {name}, which it does not describe")))?
            .clone();
        if !self.building.insert(name.to_owned()) {
            return Err(self.refuse(format!("{name} holds itself by value")));
        }
        let fields = self.fields_of(&described)?;
        self.building.remove(name);
        let align = self.check(&described, &fields)?;
        let visible = (0..described.members.len())
            .filter(|index| described.is_visible(&described.members[*index]))
            .map(|index| Visible {
                index,
                count_offset: described.members[index]
                    .count
                    .as_deref()
                    .and_then(|name| described.member_named(name))
                    .map(|counted_by| counted_by.offset),
            })
            .collect();
        let leading_size = described.leading_size();
        let size = described.size;
        self.built.insert(
            name.to_owned(),
            LaidOut {
                described,
                align,
                visible,
                leading_size,
            },
        );
        Ok((size, align))
    }

    fn member_layout(&mut self, member: &Member) -> Result<(usize, usize), LayoutRefusal> {
        if member.kind == 0 || member.kind > KIND_LAST {
            return Err(self.refuse(format!(
                "its member {} is of kind {}, which this binding does not know",
                member.name, member.kind
            )));
        }
        if let Some(layout) = scalar_layout(member.kind) {
            return Ok(layout);
        }
        if is_pointer_sized(member.kind) {
            return Ok((POINTER_SIZE, POINTER_ALIGN));
        }
        if member.kind == KIND_VALUE {
            return Ok((size_of::<ValueNode>(), align_of::<ValueNode>()));
        }
        let name = member.type_name.clone().ok_or_else(|| {
            self.refuse(format!(
                "its member {} holds a struct and names none",
                member.name
            ))
        })?;
        self.struct_layout(&name)
    }

    /// Every field of the struct in declaration order: a definition's operations, which it does not list as members,
    /// among them by offset.
    fn fields_of(&mut self, described: &Type) -> Result<Vec<Field>, LayoutRefusal> {
        let mut fields = Vec::new();
        for member in &described.members {
            let (size, align) = self.member_layout(member)?;
            fields.push(Field {
                name: member.name.to_string(),
                offset: member.offset,
                size,
                align,
            });
        }
        if described.name == self.description.definition {
            fields.extend(self.description.operations.iter().map(|operation| Field {
                name: operation.name.clone(),
                offset: operation.offset,
                size: POINTER_SIZE,
                align: POINTER_ALIGN,
            }));
            fields.sort_by_key(|field| field.offset);
        }
        Ok(fields)
    }

    /// Lays `fields` out in order and compares each with what the description states, answering the alignment.
    fn check(&self, described: &Type, fields: &[Field]) -> Result<usize, LayoutRefusal> {
        let mut end = 0;
        let mut align = 1;
        for field in fields {
            let offset = round_up(end, field.align);
            if offset != field.offset {
                return Err(self.refuse(format!(
                    "{}.{} is described at offset {}, where this platform lays it out at offset {offset}",
                    described.name, field.name, field.offset
                )));
            }
            end = offset + field.size;
            align = align.max(field.align);
        }
        let size = round_up(end, align);
        if size != described.size {
            return Err(self.refuse(format!(
                "{} is described as {} bytes, where this platform lays it out in {size}",
                described.name, described.size
            )));
        }
        Ok(align)
    }

    fn check_member_sizes(&self) -> Result<(), LayoutRefusal> {
        for laid_out in self.built.values() {
            for member in &laid_out.described.members {
                let size = self.sized(member);
                if size != member.size {
                    return Err(self.refuse(format!(
                        "{}.{} is described as {} bytes, where this platform holds it in {size}",
                        laid_out.described.name, member.name, member.size
                    )));
                }
            }
        }
        Ok(())
    }

    fn sized(&self, member: &Member) -> usize {
        if let Some((size, _)) = scalar_layout(member.kind) {
            return size;
        }
        if member.kind == KIND_VALUE {
            return size_of::<ValueNode>();
        }
        if member.kind == KIND_STRUCT {
            let name = member.type_name.as_deref().unwrap_or_default();
            return self
                .built
                .get(name)
                .map_or(0, |laid_out| laid_out.described.size);
        }
        POINTER_SIZE
    }
}

impl Layouts {
    /// Lays out every struct `description` names and checks it, and every element a pointer or an array reaches.
    pub(crate) fn new(description: &Description) -> Result<Layouts, LayoutRefusal> {
        let mut builder = Builder {
            description,
            built: BTreeMap::new(),
            building: BTreeSet::new(),
        };
        for described in &description.types {
            builder.struct_layout(&described.name)?;
        }
        builder.check_member_sizes()?;
        let layouts = Layouts {
            types: builder.built,
        };
        for described in &description.types {
            for member in &described.members {
                layouts.refuse_unreachable_element(
                    member.kind,
                    member.element,
                    member.type_name.as_deref(),
                    &format!("its member {}.{}", described.name, member.name),
                )?;
            }
        }
        for operation in &description.operations {
            for parameter in &operation.parameters {
                layouts.refuse_unreachable_element(
                    parameter.kind,
                    parameter.element,
                    parameter.type_name.as_deref(),
                    &format!("its operation {}'s {}", operation.name, parameter.name),
                )?;
            }
        }
        Ok(layouts)
    }

    /// The struct named `name`; the check proved every name a description uses names one, so none is only a defect.
    pub(crate) fn laid_out(&self, name: Option<&str>) -> Option<&LaidOut> {
        self.types.get(name?)
    }

    /// How many bytes one element of an array of `element` takes, `type_name` naming a struct element.
    pub(crate) fn element_size(&self, element: u32, type_name: Option<&str>) -> Option<usize> {
        match element {
            KIND_STRUCT => self
                .laid_out(type_name)
                .map(|laid_out| laid_out.described.size),
            KIND_VALUE => Some(size_of::<ValueNode>()),
            scalar => scalar_layout(scalar).map(|(size, _)| size),
        }
    }

    fn refuse_unreachable_element(
        &self,
        kind: u32,
        element: u32,
        type_name: Option<&str>,
        place: &str,
    ) -> Result<(), LayoutRefusal> {
        if kind != KIND_POINTER && kind != KIND_ARRAY {
            return Ok(());
        }
        let reachable = match (kind, element) {
            (KIND_POINTER, KIND_STRUCT) | (KIND_ARRAY, KIND_STRUCT) => {
                self.laid_out(type_name).is_some()
            }
            (KIND_POINTER, KIND_VALUE) | (KIND_ARRAY, KIND_VALUE) => true,
            (KIND_ARRAY, scalar) => scalar_layout(scalar).is_some(),
            _ => false,
        };
        if reachable {
            Ok(())
        } else {
            Err(format!(
                "{place} reaches an element of kind {element}, which this binding cannot hold there"
            ))
        }
    }
}
