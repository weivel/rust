//! A call's inputs written into memory of its own, laid out as the checked description says. The walk keeps its own
//! stack of structs and value nodes still to fill rather than recursing, so the depth limit is reached on any stack.

use std::collections::BTreeSet;
use std::ffi::c_char;
use std::mem::offset_of;

use super::abi::{
    self, DEPTH_LIMIT, KIND_ARRAY, KIND_BYTES, KIND_CONTEXT, KIND_DEFINITION, KIND_HANDLE,
    KIND_POINTER, KIND_STORAGE, KIND_STRUCT, KIND_TEXT, KIND_TEXTS, KIND_VALUE, ValueEntry,
    ValueNode,
};
use super::described::{Member, Parameter};
use super::invoke::Slot;
use super::layouts::{LaidOut, Layouts};
use super::memory::{Arena, put, put_slot, scalar_slot, struct_of, wrong};
use super::value::{Members, Tree, Value};

/// What a pointer-sized input or member was written as: its address, and how many it counts.
struct Reached {
    address: usize,
    count: usize,
}

const NOTHING: Reached = Reached {
    address: 0,
    count: 0,
};

/// A struct or a value node whose place is made and whose contents are still to be written.
enum Fill<'layouts, 'values> {
    Struct {
        address: usize,
        laid_out: &'layouts LaidOut,
        members: &'values Members,
        place: String,
        depth: usize,
    },
    Node {
        address: usize,
        tree: &'values Tree,
        place: String,
        depth: usize,
    },
}

type Fills<'layouts, 'values> = Vec<Fill<'layouts, 'values>>;

/// Writes one call's inputs, by the layouts its definition was bound with.
pub(crate) struct Writer<'a> {
    pub(crate) layouts: &'a Layouts,
    pub(crate) library: u64,
    pub(crate) arena: Arena,
}

static EMPTY: Members = Members::new();

impl<'a> Writer<'a> {
    /// The slot an input parameter is passed in and, for one a SIZE parameter counts, its count; a left-out input is
    /// its kind's zero.
    pub(crate) fn input(
        &mut self,
        parameter: &Parameter,
        value: Option<&Value>,
        place: &str,
    ) -> Result<(Slot, usize), String> {
        if let Some(slot) = scalar_slot(parameter.kind, value, place)? {
            return Ok((slot, 0));
        }
        let mut fills = Fills::new();
        let reached = self.reached(
            parameter.kind,
            parameter.element,
            parameter.type_name.as_deref(),
            value,
            place,
            0,
            &mut fills,
        )?;
        while let Some(fill) = fills.pop() {
            match fill {
                Fill::Struct {
                    address,
                    laid_out,
                    members,
                    place,
                    depth,
                } => self.write_struct(address, laid_out, members, &place, depth, &mut fills)?,
                Fill::Node {
                    address,
                    tree,
                    place,
                    depth,
                } => self.write_node(address, tree, &place, depth, &mut fills)?,
            }
        }
        Ok((Slot::Word(reached.address as u64), reached.count))
    }

    #[allow(clippy::too_many_arguments)]
    fn reached<'values>(
        &mut self,
        kind: u32,
        element: u32,
        type_name: Option<&str>,
        value: Option<&'values Value>,
        place: &str,
        depth: usize,
        fills: &mut Fills<'a, 'values>,
    ) -> Result<Reached, String> {
        let Some(value) = value else {
            return Ok(NOTHING);
        };
        match (kind, value) {
            (KIND_TEXT, Value::Text(None)) => Ok(NOTHING),
            (KIND_TEXT, Value::Text(Some(text))) => Ok(Reached {
                address: self.arena.text(text)?,
                count: 0,
            }),
            (KIND_TEXTS, Value::Texts(texts)) => self.texts(texts),
            (KIND_BYTES, Value::Bytes(bytes)) => Ok(self.bytes(bytes)),
            (KIND_POINTER, _) => {
                let bytes = self.layouts.element_size(element, type_name).unwrap_or(0);
                let address = self.arena.block(bytes);
                self.element(
                    element,
                    type_name,
                    address,
                    value,
                    place.to_owned(),
                    depth,
                    fills,
                )?;
                Ok(Reached { address, count: 0 })
            }
            (KIND_ARRAY, Value::Array(items)) => {
                self.array(element, type_name, items, place, depth, fills)
            }
            (KIND_HANDLE, Value::Handle(handle)) => {
                let (address, using) = handle
                    .use_for(self.library, type_name.unwrap_or_default())
                    .map_err(|why| format!("{place}: {why}"))?;
                self.arena.handles.push(using);
                Ok(Reached { address, count: 0 })
            }
            (KIND_DEFINITION, _) => Err(format!("{place} is a definition, which is no input")),
            _ => Err(wrong(place, kind, value)),
        }
    }

    fn texts(&mut self, texts: &[Option<String>]) -> Result<Reached, String> {
        let address = self.arena.block(texts.len() * size_of::<usize>());
        for (index, text) in texts.iter().enumerate() {
            let pointer = match text {
                Some(text) => self.arena.text(text)?,
                None => 0,
            };
            // SAFETY: the block holds one pointer per text.
            unsafe { put(address + index * size_of::<usize>(), pointer) };
        }
        Ok(Reached {
            address,
            count: texts.len(),
        })
    }

    fn bytes(&mut self, bytes: &[u8]) -> Reached {
        let address = self.arena.block(bytes.len());
        // SAFETY: the block holds at least `bytes.len()` bytes and does not overlap the slice.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), address as *mut u8, bytes.len()) };
        Reached {
            address,
            count: bytes.len(),
        }
    }

    fn array<'values>(
        &mut self,
        element: u32,
        type_name: Option<&str>,
        items: &'values [Value],
        place: &str,
        depth: usize,
        fills: &mut Fills<'a, 'values>,
    ) -> Result<Reached, String> {
        let stride = self.layouts.element_size(element, type_name).unwrap_or(0);
        let address = self.arena.block(stride * items.len());
        for (index, item) in items.iter().enumerate() {
            let item_place = format!("{place}[{index}]");
            let at = address + index * stride;
            self.element(element, type_name, at, item, item_place, depth, fills)?;
        }
        Ok(Reached {
            address,
            count: items.len(),
        })
    }

    /// Writes one element a pointer or an array reaches at `address`, a struct or a node by a fill of its own.
    #[allow(clippy::too_many_arguments)]
    fn element<'values>(
        &mut self,
        element: u32,
        type_name: Option<&str>,
        address: usize,
        value: &'values Value,
        place: String,
        depth: usize,
        fills: &mut Fills<'a, 'values>,
    ) -> Result<(), String> {
        match (element, value) {
            (KIND_STRUCT, Value::Struct(members)) => {
                let laid_out = struct_of(self.layouts, type_name, &place)?;
                fills.push(Fill::Struct {
                    address,
                    laid_out,
                    members,
                    place,
                    depth,
                });
                Ok(())
            }
            (KIND_VALUE, Value::Tree(tree)) => {
                fills.push(Fill::Node {
                    address,
                    tree,
                    place,
                    depth: 0,
                });
                Ok(())
            }
            (scalar, value) => match scalar_slot(scalar, Some(value), &place)? {
                // SAFETY: the element lies inside a block sized for its kind.
                Some(slot) => unsafe { put_slot(address, scalar, slot) },
                None => Err(wrong(&place, scalar, value)),
            },
        }
    }

    /// Writes `members` as the struct `laid_out` at `address`, its leading size set and a member left out at its
    /// kind's zero.
    fn write_struct<'values>(
        &mut self,
        address: usize,
        laid_out: &'a LaidOut,
        members: &'values Members,
        place: &str,
        depth: usize,
        fills: &mut Fills<'a, 'values>,
    ) -> Result<(), String> {
        if depth > DEPTH_LIMIT {
            return Err(format!(
                "{place} nests structs deeper than {DEPTH_LIMIT} levels"
            ));
        }
        let described = &laid_out.described;
        if let Some(unknown) = members.keys().find(|name| {
            described
                .member_named(name)
                .is_none_or(|member| !described.is_visible(member))
        }) {
            return Err(format!("{place} has no member {unknown} a caller writes"));
        }
        if let Some(size) = described.leading_size() {
            // SAFETY: a struct with a leading size holds it at offset 0, inside its block.
            unsafe { put(address, size) };
        }
        for index in &laid_out.visible {
            let member = &described.members[*index];
            let member_place = format!("{place}.{}", member.name);
            let value = members.get(&member.name);
            let count = self.write_member(address, member, value, &member_place, depth, fills)?;
            if let Some(counted_by) = member
                .count
                .as_deref()
                .and_then(|name| described.member_named(name))
            {
                // SAFETY: a count is a SIZE member of the same struct.
                unsafe { put(address + counted_by.offset, count) };
            }
        }
        Ok(())
    }

    /// Writes one member of the struct at `base`, answering how many it counts.
    fn write_member<'values>(
        &mut self,
        base: usize,
        member: &Member,
        value: Option<&'values Value>,
        place: &str,
        depth: usize,
        fills: &mut Fills<'a, 'values>,
    ) -> Result<usize, String> {
        let address = base + member.offset;
        if let Some(slot) = scalar_slot(member.kind, value, place)? {
            // SAFETY: the member lies inside its struct's block, sized for its kind.
            unsafe { put_slot(address, member.kind, slot)? };
            return Ok(0);
        }
        match member.kind {
            KIND_STRUCT => {
                let laid_out = struct_of(self.layouts, member.type_name.as_deref(), place)?;
                let members = match value {
                    None => &EMPTY,
                    Some(Value::Struct(members)) => members,
                    Some(other) => return Err(wrong(place, KIND_STRUCT, other)),
                };
                fills.push(Fill::Struct {
                    address,
                    laid_out,
                    members,
                    place: place.to_owned(),
                    depth: depth + 1,
                });
                Ok(0)
            }
            KIND_VALUE => match value {
                None => Ok(0),
                Some(Value::Tree(tree)) => {
                    fills.push(Fill::Node {
                        address,
                        tree,
                        place: place.to_owned(),
                        depth: 0,
                    });
                    Ok(0)
                }
                Some(other) => Err(wrong(place, KIND_VALUE, other)),
            },
            KIND_CONTEXT | KIND_STORAGE => Ok(0),
            kind => {
                let type_name = member.type_name.as_deref();
                let reached = self.reached(
                    kind,
                    member.element,
                    type_name,
                    value,
                    place,
                    depth + 1,
                    fills,
                )?;
                // SAFETY: a pointer-sized member lies inside its struct's block.
                unsafe { put(address, reached.address) };
                Ok(reached.count)
            }
        }
    }

    /// Writes `tree` as the value node at `address`, each node below it by a fill of its own.
    fn write_node<'values>(
        &mut self,
        address: usize,
        tree: &'values Tree,
        place: &str,
        depth: usize,
        fills: &mut Fills<'a, 'values>,
    ) -> Result<(), String> {
        if depth > DEPTH_LIMIT {
            return Err(format!(
                "{place} nests a value deeper than {DEPTH_LIMIT} levels"
            ));
        }
        let mut node = ValueNode::NONE;
        match tree {
            Tree::Null => node.kind = abi::VALUE_NULL,
            Tree::Boolean(boolean) => {
                node.kind = abi::VALUE_BOOLEAN;
                node.boolean = u32::from(*boolean);
            }
            Tree::Number(number) if number.is_finite() => {
                node.kind = abi::VALUE_NUMBER;
                node.number = *number;
            }
            Tree::Number(number) => {
                return Err(format!(
                    "{place} holds {number}, and a value's number is finite"
                ));
            }
            Tree::Text(text) => {
                node.kind = abi::VALUE_TEXT;
                node.text = self.arena.text(text)? as *const c_char;
                node.length = text.len();
            }
            Tree::List(items) => {
                node.kind = abi::VALUE_LIST;
                node.items = self.nodes(items, place, depth, fills) as *const ValueNode;
                node.count = items.len();
            }
            Tree::Record(entries) => {
                node.kind = abi::VALUE_RECORD;
                node.entries = self.entries(entries, place, depth, fills)? as *const ValueEntry;
                node.count = entries.len();
            }
        }
        // SAFETY: a value node's place lies inside a block sized for it.
        unsafe { put(address, node) };
        Ok(())
    }

    fn nodes<'values>(
        &mut self,
        items: &'values [Tree],
        place: &str,
        depth: usize,
        fills: &mut Fills<'a, 'values>,
    ) -> usize {
        let address = self.arena.block(items.len() * size_of::<ValueNode>());
        for (index, tree) in items.iter().enumerate() {
            fills.push(Fill::Node {
                address: address + index * size_of::<ValueNode>(),
                tree,
                place: format!("{place}[{index}]"),
                depth: depth + 1,
            });
        }
        address
    }

    fn entries<'values>(
        &mut self,
        entries: &'values [(String, Tree)],
        place: &str,
        depth: usize,
        fills: &mut Fills<'a, 'values>,
    ) -> Result<usize, String> {
        let mut keys = BTreeSet::new();
        let address = self.arena.block(entries.len() * size_of::<ValueEntry>());
        for (index, (key, tree)) in entries.iter().enumerate() {
            if !keys.insert(key.as_str()) {
                return Err(format!("{place} holds the key {key} twice"));
            }
            let entry = address + index * size_of::<ValueEntry>();
            let key_address = self.arena.text(key)?;
            // SAFETY: each entry lies inside the block sized for every entry.
            unsafe {
                put(entry + offset_of!(ValueEntry, key), key_address);
                put(entry + offset_of!(ValueEntry, key_length), key.len());
            }
            fills.push(Fill::Node {
                address: entry + offset_of!(ValueEntry, value),
                tree,
                place: format!("{place}.{key}"),
                depth: depth + 1,
            });
        }
        Ok(address)
    }
}
