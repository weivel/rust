//! Every answer read back from the memory its provider filled, by the layouts the definition was bound with. The walk
//! keeps its own stack of structs and value nodes still to read rather than recursing, so the depth limit is reached on
//! any stack: each part is made where it is met, its children read into it later, and every part is placed into its
//! parent once the walk is done.

use std::collections::BTreeSet;
use std::mem::offset_of;
use std::sync::Arc;

use super::abi::{
    self, DEPTH_LIMIT, KIND_ARRAY, KIND_BYTES, KIND_DEFINITION, KIND_HANDLE, KIND_POINTER,
    KIND_STRUCT, KIND_TEXT, KIND_TEXTS, KIND_VALUE, ValueEntry, ValueNode,
};
use super::described::{Member, Parameter};
use super::layouts::{LaidOut, Layouts};
use super::memory::{bytes_at, counted_text, get, scalar_at, struct_of, text_at, texts_at};
use super::stack::Stack;
use super::value::{Members, Tree, Value};

/// What reading an answer makes of a handle and of a definition it carries.
pub(crate) trait Answering {
    /// The handle at `pointer`, of the opaque `type_name`, which the release named `release` frees.
    fn handle(&mut self, pointer: usize, type_name: &str, release: Option<&str>) -> Value;
    /// The definition at `pointer`, of the struct `type_name`, which a nested description describes.
    fn definition(&mut self, pointer: usize, type_name: &str) -> Result<Value, String>;
}

/// Reads answers back from the memory their provider filled.
pub(crate) struct Reader<'a> {
    pub(crate) layouts: &'a Layouts,
    pub(crate) answering: &'a mut dyn Answering,
}

/// Where a part read goes in its parent: a struct's member, or an item of an array, a list or a record.
enum Key {
    Member(Arc<str>),
    Item(usize),
}

/// One part of the answer, made when the walk met it, and where it goes.
struct Part {
    value: Value,
    parent: Option<(usize, Key)>,
}

/// A struct or a value node still to read into the part at `at`.
enum Visit<'layouts> {
    Struct {
        address: usize,
        laid_out: &'layouts LaidOut,
        depth: usize,
        at: usize,
    },
    Node {
        address: usize,
        depth: usize,
        at: usize,
    },
}

const PLACEHOLDER: Value = Value::U32(0);

struct Walk<'layouts, 'answering> {
    layouts: &'layouts Layouts,
    answering: &'answering mut dyn Answering,
    parts: Stack<Part>,
    visits: Stack<Visit<'layouts>>,
}

impl<'a> Reader<'a> {
    /// Reads the answer an answer parameter's block holds, or none for one left out.
    pub(crate) fn answer(
        &mut self,
        parameter: &Parameter,
        block: usize,
    ) -> Result<Option<Value>, String> {
        // SAFETY: the block is the answer block the binder handed the provider, filled on OK.
        unsafe {
            if let Some(scalar) = scalar_at(block, parameter.kind) {
                return Ok(Some(scalar));
            }
            let type_name = parameter.type_name.as_deref();
            match parameter.kind {
                KIND_POINTER => {
                    let laid_out = struct_of(self.layouts, type_name, &an_answer)?;
                    self.walk(members_of(laid_out), |at| Visit::Struct {
                        address: block,
                        laid_out,
                        depth: 0,
                        at,
                    })
                    .map(Some)
                }
                KIND_VALUE => {
                    let root = block + offset_of!(abi::ValueTree, root);
                    if get::<u32>(root) == abi::VALUE_NONE {
                        return Ok(None);
                    }
                    self.walk(Value::Tree(Tree::Null), |at| Visit::Node {
                        address: root,
                        depth: 0,
                        at,
                    })
                    .map(Some)
                }
                KIND_TEXT => Ok(Some(Value::Text(text_at(get(block))?))),
                kind => pointer_answer(
                    self.answering,
                    kind,
                    get(block),
                    type_name,
                    parameter.release.as_deref(),
                ),
            }
        }
    }

    fn walk(
        &mut self,
        root: Value,
        visit: impl FnOnce(usize) -> Visit<'a>,
    ) -> Result<Value, String> {
        let mut walk = Walk {
            layouts: self.layouts,
            answering: &mut *self.answering,
            parts: Stack::new(),
            visits: Stack::new(),
        };
        walk.parts.push(Part {
            value: root,
            parent: None,
        });
        walk.visits.push(visit(0));
        while let Some(visit) = walk.visits.pop() {
            // SAFETY: every visit is of memory the provider filled, reached through the answer's own layouts.
            unsafe {
                match visit {
                    Visit::Struct {
                        address,
                        laid_out,
                        depth,
                        at,
                    } => walk.read_struct(address, laid_out, depth, at)?,
                    Visit::Node { address, depth, at } => walk.read_node(address, depth, at)?,
                }
            }
        }
        Ok(walk.assemble())
    }

    /// The `detail` and `fix` texts of the refusal struct `laid_out` at `address`.
    pub(crate) fn refusal_texts(
        laid_out: &LaidOut,
        address: usize,
    ) -> Result<(Option<String>, Option<String>), String> {
        let text = |name: &str| match laid_out.described.member_named(name) {
            // SAFETY: the check proved every refusal's struct carries `detail` and `fix` TEXT members, and the block
            // is the refusal the binder handed the provider.
            Some(member) => unsafe { text_at(get(address + member.offset)) },
            None => Ok(None),
        };
        Ok((text("detail")?, text("fix")?))
    }
}

fn pointer_answer(
    answering: &mut dyn Answering,
    kind: u32,
    pointer: usize,
    type_name: Option<&str>,
    release: Option<&str>,
) -> Result<Option<Value>, String> {
    if pointer == 0 {
        return Ok(None);
    }
    let type_name = type_name.unwrap_or_default();
    match kind {
        KIND_HANDLE => Ok(Some(answering.handle(pointer, type_name, release))),
        KIND_DEFINITION => answering.definition(pointer, type_name).map(Some),
        other => Err(format!("an answer of kind {other}, which no binding reads")),
    }
}

fn an_answer() -> String {
    "an answer".to_owned()
}

/// A struct's members, with room for every member `laid_out` shows.
fn members_of(laid_out: &LaidOut) -> Value {
    Value::Struct(Members::with_capacity(laid_out.visible.len()))
}

/// Puts `child` where `key` names in `parent`.
fn place(parent: &mut Value, key: Key, child: Value) {
    match (parent, key, child) {
        (Value::Struct(members), Key::Member(name), child) => members.push(name, child),
        (Value::Array(items), Key::Item(index), child) => items[index] = child,
        (Value::Tree(Tree::List(items)), Key::Item(index), Value::Tree(tree)) => {
            items[index] = tree;
        }
        (Value::Tree(Tree::Record(entries)), Key::Item(index), Value::Tree(tree)) => {
            entries[index].1 = tree;
        }
        _ => unreachable!("a part is placed only into the parent it was made for"),
    }
}

impl<'layouts> Walk<'layouts, '_> {
    /// Makes a part placed later where `key` names in the part at `parent`, answering where it is.
    fn part(&mut self, value: Value, parent: usize, key: Key) -> usize {
        self.parts.push(Part {
            value,
            parent: Some((parent, key)),
        });
        self.parts.len() - 1
    }

    /// Places every part into its parent, the last made first, so a part holds its own parts before it is placed.
    fn assemble(mut self) -> Value {
        while self.parts.len() > 1 {
            let part = self.parts.pop().expect("a part is left");
            if let Some((parent, key)) = part.parent {
                place(&mut self.parts[parent].value, key, part.value);
            }
        }
        self.parts.pop().map_or(PLACEHOLDER, |root| root.value)
    }

    fn set_member(&mut self, at: usize, name: &Arc<str>, value: Value) {
        if let Value::Struct(members) = &mut self.parts[at].value {
            members.push(name.clone(), value);
        }
    }

    /// # Safety
    /// `address` holds the struct `laid_out`, as its provider filled it.
    unsafe fn read_struct(
        &mut self,
        address: usize,
        laid_out: &'layouts LaidOut,
        depth: usize,
        at: usize,
    ) -> Result<(), String> {
        if depth > DEPTH_LIMIT {
            return Err(format!(
                "an answer nests structs deeper than {DEPTH_LIMIT} levels"
            ));
        }
        let described = &laid_out.described;
        for visible in &laid_out.visible {
            let member = &described.members[visible.index];
            let count = visible
                .count_offset
                // SAFETY: a count is a SIZE member of the same struct.
                .map_or(0, |offset| unsafe { get::<usize>(address + offset) });
            // SAFETY: as the caller promised.
            unsafe { self.member(address + member.offset, member, count, depth, at)? };
        }
        Ok(())
    }

    /// Reads one member into the struct at `at`, a struct or a node it reaches by a visit of its own.
    ///
    /// # Safety
    /// `address` holds the member, as its provider filled it, and `count` is the member counting it.
    unsafe fn member(
        &mut self,
        address: usize,
        member: &Member,
        count: usize,
        depth: usize,
        at: usize,
    ) -> Result<(), String> {
        // SAFETY: as the caller promised.
        unsafe {
            if let Some(scalar) = scalar_at(address, member.kind) {
                self.set_member(at, &member.name, scalar);
                return Ok(());
            }
            let type_name = member.type_name.as_deref();
            let key = || Key::Member(member.name.clone());
            let value = match member.kind {
                KIND_TEXT => Value::Text(text_at(get(address))?),
                KIND_TEXTS => Value::Texts(texts_at(get(address), count)?),
                KIND_BYTES => Value::Bytes(bytes_at(get(address), count)),
                KIND_STRUCT => {
                    let laid_out = struct_of(self.layouts, type_name, &an_answer)?;
                    let part = self.part(members_of(laid_out), at, key());
                    self.visits.push(Visit::Struct {
                        address,
                        laid_out,
                        depth: depth + 1,
                        at: part,
                    });
                    return Ok(());
                }
                KIND_VALUE => {
                    self.node_part(address, 0, at, key());
                    return Ok(());
                }
                KIND_POINTER => {
                    self.pointed(
                        member.element,
                        type_name,
                        get(address),
                        depth + 1,
                        at,
                        key(),
                    )?;
                    return Ok(());
                }
                KIND_ARRAY => {
                    let pointer = get(address);
                    let array = self.part(Value::Array(Vec::new()), at, key());
                    self.array(member.element, type_name, pointer, count, depth + 1, array)?;
                    return Ok(());
                }
                kind => {
                    let pointer = get(address);
                    let release = member.release.as_deref();
                    match pointer_answer(self.answering, kind, pointer, type_name, release)? {
                        Some(value) => value,
                        None => return Ok(()),
                    }
                }
            };
            self.set_member(at, &member.name, value);
        }
        Ok(())
    }

    /// A node that is not of kind NONE, read later into a part where `key` names in `parent`.
    ///
    /// # Safety
    /// `address` holds a value node its provider built.
    unsafe fn node_part(&mut self, address: usize, depth: usize, parent: usize, key: Key) -> bool {
        // SAFETY: as the caller promised.
        if unsafe { get::<u32>(address) } == abi::VALUE_NONE {
            return false;
        }
        let part = self.part(Value::Tree(Tree::Null), parent, key);
        self.visits.push(Visit::Node {
            address,
            depth,
            at: part,
        });
        true
    }

    /// # Safety
    /// `pointer` is NULL or what a POINTER of `element` reaches.
    unsafe fn pointed(
        &mut self,
        element: u32,
        type_name: Option<&str>,
        pointer: usize,
        depth: usize,
        parent: usize,
        key: Key,
    ) -> Result<(), String> {
        if pointer == 0 {
            return Ok(());
        }
        if element == KIND_VALUE {
            // SAFETY: as the caller promised.
            unsafe { self.node_part(pointer, 0, parent, key) };
            return Ok(());
        }
        let laid_out = struct_of(self.layouts, type_name, &an_answer)?;
        let part = self.part(members_of(laid_out), parent, key);
        self.visits.push(Visit::Struct {
            address: pointer,
            laid_out,
            depth,
            at: part,
        });
        Ok(())
    }

    /// Reads `count` elements at `pointer` into the array part at `array`.
    ///
    /// # Safety
    /// `pointer` is NULL or `count` elements of `element`.
    unsafe fn array(
        &mut self,
        element: u32,
        type_name: Option<&str>,
        pointer: usize,
        count: usize,
        depth: usize,
        array: usize,
    ) -> Result<(), String> {
        if pointer == 0 {
            return Ok(());
        }
        let stride = self.layouts.element_size(element, type_name).unwrap_or(0);
        let mut items = Vec::with_capacity(count);
        for index in 0..count {
            let address = pointer + index * stride;
            // SAFETY: as the caller promised.
            let item = match element {
                KIND_STRUCT => {
                    let laid_out = struct_of(self.layouts, type_name, &an_answer)?;
                    let part = self.part(members_of(laid_out), array, Key::Item(index));
                    self.visits.push(Visit::Struct {
                        address,
                        laid_out,
                        depth,
                        at: part,
                    });
                    PLACEHOLDER
                }
                KIND_VALUE => {
                    if !unsafe { self.node_part(address, 0, array, Key::Item(index)) } {
                        return Err("an array of values holds a node of kind NONE".to_owned());
                    }
                    PLACEHOLDER
                }
                scalar => unsafe { scalar_at(address, scalar) }.unwrap_or(PLACEHOLDER),
            };
            items.push(item);
        }
        self.parts[array].value = Value::Array(items);
        Ok(())
    }

    /// Reads the value node at `address` into the part at `at`, each node below it by a visit of its own.
    ///
    /// # Safety
    /// `address` holds a value node its provider built.
    unsafe fn read_node(&mut self, address: usize, depth: usize, at: usize) -> Result<(), String> {
        if depth > DEPTH_LIMIT {
            return Err(format!(
                "an answer nests a value deeper than {DEPTH_LIMIT} levels"
            ));
        }
        // SAFETY: as the caller promised.
        let node: ValueNode = unsafe { get(address) };
        let tree = match node.kind {
            abi::VALUE_NULL => Tree::Null,
            abi::VALUE_BOOLEAN => Tree::Boolean(node.boolean != 0),
            abi::VALUE_NUMBER => Tree::Number(node.number),
            // SAFETY: a text node holds `length` bytes at `text`.
            abi::VALUE_TEXT => {
                Tree::Text(unsafe { counted_text(node.text as usize, node.length) }?)
            }
            // SAFETY: a list node holds `count` nodes at `items`.
            abi::VALUE_LIST => Tree::List(unsafe { self.items(&node, depth, at)? }),
            // SAFETY: a record node holds `count` entries at `entries`.
            abi::VALUE_RECORD => Tree::Record(unsafe { self.record(&node, depth, at)? }),
            other => {
                return Err(format!(
                    "an answer holds a value of kind {other} where a node is required"
                ));
            }
        };
        self.parts[at].value = Value::Tree(tree);
        Ok(())
    }

    /// # Safety
    /// `node` is a list node its provider built.
    unsafe fn items(
        &mut self,
        node: &ValueNode,
        depth: usize,
        at: usize,
    ) -> Result<Vec<Tree>, String> {
        for index in 0..node.count {
            let address = node.items as usize + index * size_of::<ValueNode>();
            // SAFETY: as the caller promised.
            if !unsafe { self.node_part(address, depth + 1, at, Key::Item(index)) } {
                return Err("a list holds an item of kind NONE".to_owned());
            }
        }
        Ok(vec![Tree::Null; node.count])
    }

    /// # Safety
    /// `node` is a record node its provider built.
    unsafe fn record(
        &mut self,
        node: &ValueNode,
        depth: usize,
        at: usize,
    ) -> Result<Vec<(String, Tree)>, String> {
        let mut keys = BTreeSet::new();
        let mut entries = Vec::with_capacity(node.count);
        for index in 0..node.count {
            let entry_address = node.entries as usize + index * size_of::<ValueEntry>();
            // SAFETY: as the caller promised.
            let entry: ValueEntry = unsafe { get(entry_address) };
            // SAFETY: a key holds `key_length` bytes.
            let key = unsafe { counted_text(entry.key as usize, entry.key_length) }?;
            if !keys.insert(key.clone()) {
                return Err(format!("an answer's record holds the key {key} twice"));
            }
            let value_address = entry_address + offset_of!(ValueEntry, value);
            // SAFETY: as the caller promised.
            if !unsafe { self.node_part(value_address, depth + 1, at, Key::Item(index)) } {
                return Err(format!("a record's {key} is of kind NONE"));
            }
            entries.push((key, Tree::Null));
        }
        Ok(entries)
    }
}
