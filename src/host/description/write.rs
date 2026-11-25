//! A call's inputs written into memory of its own, laid out as the checked description says. The walk keeps its own
//! stack of structs and value nodes still to fill rather than recursing, so the depth limit is reached on any stack.
//! Where a value lies is kept as a chain of places and spelled out only when it is wrong, so a call that writes its
//! inputs whole formats nothing.

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
use super::stack::Stack;
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

/// One step of the way to a value: the input itself, or a member, an item or a key below the place before it.
#[derive(Clone, Copy)]
enum Step<'a> {
    Input {
        operation: &'a str,
        parameter: &'a str,
    },
    Member(&'a str),
    Item(usize),
    Key(&'a str),
}

/// Where a value lies: its last step and the place that step was taken from.
#[derive(Clone, Copy)]
struct Place<'a> {
    from: Option<usize>,
    step: Step<'a>,
}

/// A struct or a value node whose place is made and whose contents are still to be written.
enum Fill<'a> {
    Struct {
        address: usize,
        laid_out: &'a LaidOut,
        members: &'a Members,
        place: usize,
        depth: usize,
    },
    Node {
        address: usize,
        tree: &'a Tree,
        place: usize,
        depth: usize,
    },
}

/// Writes one call's inputs, by the layouts its definition was bound with, into the call's arena.
pub(crate) struct Writer<'a> {
    pub(crate) layouts: &'a Layouts,
    pub(crate) library: u64,
    pub(crate) arena: Arena,
    places: Stack<Place<'a>>,
    fills: Stack<Fill<'a>>,
}

static EMPTY: Members = Members::new();

impl<'a> Writer<'a> {
    pub(crate) fn new(layouts: &'a Layouts, library: u64, arena: Arena) -> Writer<'a> {
        Writer {
            layouts,
            library,
            arena,
            places: Stack::new(),
            fills: Stack::new(),
        }
    }

    /// The place `place` spelled out, as a mistake names it.
    fn spelled(&self, place: usize) -> String {
        let mut steps = Vec::new();
        let mut next = Some(place);
        while let Some(index) = next {
            steps.push(self.places[index].step);
            next = self.places[index].from;
        }
        let mut spelled = String::new();
        for step in steps.iter().rev() {
            match step {
                Step::Input {
                    operation,
                    parameter,
                } => spelled.push_str(&format!("{operation}'s {parameter}")),
                Step::Member(name) | Step::Key(name) => spelled.push_str(&format!(".{name}")),
                Step::Item(index) => spelled.push_str(&format!("[{index}]")),
            }
        }
        spelled
    }

    fn place(&mut self, from: Option<usize>, step: Step<'a>) -> usize {
        self.places.push(Place { from, step });
        self.places.len() - 1
    }

    /// The slot an input parameter of `operation` is passed in and, for one a SIZE parameter counts, its count; a
    /// left-out input is its kind's zero.
    pub(crate) fn input(
        &mut self,
        operation: &'a str,
        parameter: &'a Parameter,
        value: Option<&'a Value>,
    ) -> Result<(Slot, usize), String> {
        let spelled_input = || format!("{operation}'s {}", parameter.name);
        if let Some(slot) = scalar_slot(parameter.kind, value, &spelled_input)? {
            return Ok((slot, 0));
        }
        let root = self.place(
            None,
            Step::Input {
                operation,
                parameter: &parameter.name,
            },
        );
        let reached = self.reached(
            parameter.kind,
            parameter.element,
            parameter.type_name.as_deref(),
            value,
            root,
            0,
        )?;
        while let Some(fill) = self.fills.pop() {
            match fill {
                Fill::Struct {
                    address,
                    laid_out,
                    members,
                    place,
                    depth,
                } => self.write_struct(address, laid_out, members, place, depth)?,
                Fill::Node {
                    address,
                    tree,
                    place,
                    depth,
                } => self.write_node(address, tree, place, depth)?,
            }
        }
        Ok((Slot::Word(reached.address as u64), reached.count))
    }

    fn reached(
        &mut self,
        kind: u32,
        element: u32,
        type_name: Option<&'a str>,
        value: Option<&'a Value>,
        place: usize,
        depth: usize,
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
                self.element(element, type_name, address, value, place, depth)?;
                Ok(Reached { address, count: 0 })
            }
            (KIND_ARRAY, Value::Array(items)) => {
                self.array(element, type_name, items, place, depth)
            }
            (KIND_HANDLE, Value::Handle(handle)) => {
                let (address, using) = handle
                    .use_for(self.library, type_name.unwrap_or_default())
                    .map_err(|why| format!("{}: {why}", self.spelled(place)))?;
                self.arena.handles.push(using);
                Ok(Reached { address, count: 0 })
            }
            (KIND_DEFINITION, _) => Err(format!(
                "{} is a definition, which is no input",
                self.spelled(place)
            )),
            _ => Err(wrong(&self.spelled(place), kind, value)),
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

    fn array(
        &mut self,
        element: u32,
        type_name: Option<&'a str>,
        items: &'a [Value],
        place: usize,
        depth: usize,
    ) -> Result<Reached, String> {
        let stride = self.layouts.element_size(element, type_name).unwrap_or(0);
        let address = self.arena.block(stride * items.len());
        for (index, item) in items.iter().enumerate() {
            let item_place = self.place(Some(place), Step::Item(index));
            let at = address + index * stride;
            self.element(element, type_name, at, item, item_place, depth)?;
        }
        Ok(Reached {
            address,
            count: items.len(),
        })
    }

    /// Writes one element a pointer or an array reaches at `address`, a struct or a node by a fill of its own.
    fn element(
        &mut self,
        element: u32,
        type_name: Option<&'a str>,
        address: usize,
        value: &'a Value,
        place: usize,
        depth: usize,
    ) -> Result<(), String> {
        match (element, value) {
            (KIND_STRUCT, Value::Struct(members)) => {
                let laid_out = struct_of(self.layouts, type_name, &|| self.spelled(place))?;
                self.fills.push(Fill::Struct {
                    address,
                    laid_out,
                    members,
                    place,
                    depth,
                });
                Ok(())
            }
            (KIND_VALUE, Value::Tree(tree)) => {
                self.fills.push(Fill::Node {
                    address,
                    tree,
                    place,
                    depth: 0,
                });
                Ok(())
            }
            (KIND_VALUE, Value::NoValue) => Ok(()),
            (scalar, value) => match scalar_slot(scalar, Some(value), &|| self.spelled(place))? {
                // SAFETY: the element lies inside a block sized for its kind.
                Some(slot) => unsafe { put_slot(address, scalar, slot) },
                None => Err(wrong(&self.spelled(place), scalar, value)),
            },
        }
    }

    /// Writes `members` as the struct `laid_out` at `address`, its leading size set and a member left out at its
    /// kind's zero.
    fn write_struct(
        &mut self,
        address: usize,
        laid_out: &'a LaidOut,
        members: &'a Members,
        place: usize,
        depth: usize,
    ) -> Result<(), String> {
        if depth > DEPTH_LIMIT {
            return Err(format!(
                "{} nests structs deeper than {DEPTH_LIMIT} levels",
                self.spelled(place)
            ));
        }
        let described = &laid_out.described;
        let is_visible = |name: &str| {
            laid_out
                .visible
                .iter()
                .any(|visible| &*described.members[visible.index].name == name)
        };
        if let Some(unknown) = members.keys().find(|name| !is_visible(name)) {
            return Err(format!(
                "{} has no member {unknown} a caller writes",
                self.spelled(place)
            ));
        }
        if let Some(size) = laid_out.leading_size {
            // SAFETY: a struct with a leading size holds it at offset 0, inside its block.
            unsafe { put(address, size) };
        }
        for visible in &laid_out.visible {
            let member = &described.members[visible.index];
            let value = members.get(&member.name);
            let count = self.write_member(address, member, value, place, depth)?;
            if let Some(offset) = visible.count_offset {
                // SAFETY: a count is a SIZE member of the same struct.
                unsafe { put(address + offset, count) };
            }
        }
        Ok(())
    }

    /// Writes one member of the struct at `base`, which lies at `place`, answering how many it counts.
    fn write_member(
        &mut self,
        base: usize,
        member: &'a Member,
        value: Option<&'a Value>,
        place: usize,
        depth: usize,
    ) -> Result<usize, String> {
        let address = base + member.offset;
        let spelled_member = || format!("{}.{}", self.spelled(place), member.name);
        if let Some(slot) = scalar_slot(member.kind, value, &spelled_member)? {
            // SAFETY: the member lies inside its struct's block, sized for its kind.
            unsafe { put_slot(address, member.kind, slot)? };
            return Ok(0);
        }
        match member.kind {
            KIND_CONTEXT | KIND_STORAGE => return Ok(0),
            KIND_VALUE if matches!(value, None | Some(Value::NoValue)) => return Ok(0),
            _ => {}
        }
        let member_place = self.place(Some(place), Step::Member(&member.name));
        match member.kind {
            KIND_STRUCT => {
                let type_name = member.type_name.as_deref();
                let laid_out = struct_of(self.layouts, type_name, &|| self.spelled(member_place))?;
                let members = match value {
                    None => &EMPTY,
                    Some(Value::Struct(members)) => members,
                    Some(other) => {
                        return Err(wrong(&self.spelled(member_place), KIND_STRUCT, other));
                    }
                };
                self.fills.push(Fill::Struct {
                    address,
                    laid_out,
                    members,
                    place: member_place,
                    depth: depth + 1,
                });
                Ok(0)
            }
            KIND_VALUE => match value {
                Some(Value::Tree(tree)) => {
                    self.fills.push(Fill::Node {
                        address,
                        tree,
                        place: member_place,
                        depth: 0,
                    });
                    Ok(0)
                }
                Some(other) => Err(wrong(&self.spelled(member_place), KIND_VALUE, other)),
                None => Ok(0),
            },
            kind => {
                let type_name = member.type_name.as_deref();
                let reached = self.reached(
                    kind,
                    member.element,
                    type_name,
                    value,
                    member_place,
                    depth + 1,
                )?;
                // SAFETY: a pointer-sized member lies inside its struct's block.
                unsafe { put(address, reached.address) };
                Ok(reached.count)
            }
        }
    }

    /// Writes `tree` as the value node at `address`, each node below it by a fill of its own.
    fn write_node(
        &mut self,
        address: usize,
        tree: &'a Tree,
        place: usize,
        depth: usize,
    ) -> Result<(), String> {
        if depth > DEPTH_LIMIT {
            return Err(format!(
                "{} nests a value deeper than {DEPTH_LIMIT} levels",
                self.spelled(place)
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
                    "{} holds {number}, and a value's number is finite",
                    self.spelled(place)
                ));
            }
            Tree::Text(text) => {
                node.kind = abi::VALUE_TEXT;
                node.text = self.arena.text(text)? as *const c_char;
                node.length = text.len();
            }
            Tree::List(items) => {
                node.kind = abi::VALUE_LIST;
                node.items = self.nodes(items, place, depth) as *const ValueNode;
                node.count = items.len();
            }
            Tree::Record(entries) => {
                node.kind = abi::VALUE_RECORD;
                node.entries = self.entries(entries, place, depth)? as *const ValueEntry;
                node.count = entries.len();
            }
        }
        // SAFETY: a value node's place lies inside a block sized for it.
        unsafe { put(address, node) };
        Ok(())
    }

    fn nodes(&mut self, items: &'a [Tree], place: usize, depth: usize) -> usize {
        let address = self.arena.block(items.len() * size_of::<ValueNode>());
        for (index, tree) in items.iter().enumerate() {
            let item_place = self.place(Some(place), Step::Item(index));
            self.fills.push(Fill::Node {
                address: address + index * size_of::<ValueNode>(),
                tree,
                place: item_place,
                depth: depth + 1,
            });
        }
        address
    }

    fn entries(
        &mut self,
        entries: &'a [(String, Tree)],
        place: usize,
        depth: usize,
    ) -> Result<usize, String> {
        let mut keys = BTreeSet::new();
        let address = self.arena.block(entries.len() * size_of::<ValueEntry>());
        for (index, (key, tree)) in entries.iter().enumerate() {
            if !keys.insert(key.as_str()) {
                return Err(format!("{} holds the key {key} twice", self.spelled(place)));
            }
            let entry = address + index * size_of::<ValueEntry>();
            let key_address = self.arena.text(key)?;
            // SAFETY: each entry lies inside the block sized for every entry.
            unsafe {
                put(entry + offset_of!(ValueEntry, key), key_address);
                put(entry + offset_of!(ValueEntry, key_length), key.len());
            }
            let entry_place = self.place(Some(place), Step::Key(key));
            self.fills.push(Fill::Node {
                address: entry + offset_of!(ValueEntry, value),
                tree,
                place: entry_place,
                depth: depth + 1,
            });
        }
        Ok(address)
    }
}
