//! The binder's memory walk: a call's inputs written into memory of its own, laid out as the checked description
//! says, and every answer read back from the memory its provider filled. Structs nest at most
//! `WEIVEL_STANDARD_VALUE_DEPTH_LIMIT` levels below the one a call takes or answers, so a cycle of pointers in a
//! provider's answer is refused rather than followed for ever.

use std::collections::BTreeSet;
use std::ffi::{CStr, CString, c_char};
use std::mem::offset_of;

use super::abi::{
    self, DEPTH_LIMIT, KIND_ARRAY, KIND_BYTES, KIND_CONTEXT, KIND_DEFINITION, KIND_F64,
    KIND_HANDLE, KIND_I64, KIND_POINTER, KIND_SIZE, KIND_STORAGE, KIND_STRUCT, KIND_TEXT,
    KIND_TEXTS, KIND_U32, KIND_U64, KIND_VALUE, ValueEntry, ValueNode,
};
use super::described::{Member, Parameter};
use super::invoke::Slot;
use super::layouts::{LaidOut, Layouts};
use super::value::{Members, Tree, Value};

/// Zeroed memory aligned for any member a description holds, which stays at its address until dropped.
pub(crate) struct Block {
    start: *mut u64,
    words: usize,
}

// SAFETY: a block is plain memory the binder owns; it is written only by the one call it was made for, and freed once.
unsafe impl Send for Block {}
// SAFETY: as above; a shared reference reads nothing.
unsafe impl Sync for Block {}

impl Block {
    pub(crate) fn zeroed(bytes: usize) -> Block {
        let words = vec![0u64; bytes.div_ceil(8).max(1)].into_boxed_slice();
        let count = words.len();
        Block {
            start: Box::into_raw(words).cast::<u64>(),
            words: count,
        }
    }

    pub(crate) fn address(&self) -> usize {
        self.start as usize
    }

    /// Empties the block, then sets its leading `size` when it has one.
    pub(crate) fn empty(&self, leading_size: Option<u32>) {
        // SAFETY: the block is `words` words the binder owns, and no call is using it.
        unsafe { std::ptr::write_bytes(self.start, 0, self.words) };
        if let Some(size) = leading_size {
            // SAFETY: as above; every block is at least one word.
            unsafe { put(self.address(), size) };
        }
    }
}

impl Drop for Block {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `Box::into_raw` of a slice of `words` words, and is freed once, here.
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(self.start, self.words)) });
    }
}

/// # Safety
/// `address` is writable for a `T` inside memory the binder owns.
unsafe fn put<T>(address: usize, value: T) {
    // SAFETY: as the caller promised.
    unsafe { std::ptr::write_unaligned(address as *mut T, value) }
}

/// # Safety
/// `address` holds a `T` the provider wrote, or the binder did.
unsafe fn get<T: Copy>(address: usize) -> T {
    // SAFETY: as the caller promised.
    unsafe { std::ptr::read_unaligned(address as *const T) }
}

/// Every block and text one call's inputs are written into, freed when the call has answered.
#[derive(Default)]
pub(crate) struct Arena {
    blocks: Vec<Block>,
    texts: Vec<CString>,
}

impl Arena {
    fn block(&mut self, bytes: usize) -> usize {
        let block = Block::zeroed(bytes);
        let address = block.address();
        self.blocks.push(block);
        address
    }

    fn text(&mut self, text: &str) -> Result<usize, String> {
        let text = CString::new(text)
            .map_err(|_| format!("the text {text:?} holds a NUL, which no C text can"))?;
        let address = text.as_ptr() as usize;
        self.texts.push(text);
        Ok(address)
    }
}

fn kind_name(kind: u32) -> &'static str {
    match kind {
        KIND_U32 => "u32",
        KIND_U64 => "u64",
        KIND_I64 => "i64",
        KIND_F64 => "f64",
        KIND_SIZE => "size",
        KIND_TEXT => "text",
        KIND_TEXTS => "texts",
        KIND_BYTES => "bytes",
        KIND_STRUCT => "struct",
        KIND_POINTER => "pointer",
        KIND_ARRAY => "array",
        KIND_VALUE => "value",
        KIND_HANDLE => "handle",
        KIND_DEFINITION => "definition",
        _ => "kind it cannot write",
    }
}

fn wrong(place: &str, kind: u32, value: &Value) -> String {
    format!(
        "{place} holds a {}, and was handed {value:?}",
        kind_name(kind)
    )
}

/// What a pointer-sized input or member was written as: its address, and how many it counts.
struct Reached {
    address: usize,
    count: usize,
}

const NOTHING: Reached = Reached {
    address: 0,
    count: 0,
};

/// Writes one call's inputs, by the layouts its definition was bound with.
pub(crate) struct Writer<'a> {
    pub(crate) layouts: &'a Layouts,
    pub(crate) library: u64,
    pub(crate) arena: Arena,
}

impl Writer<'_> {
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
        let reached = self.reached(
            parameter.kind,
            parameter.element,
            parameter.type_name.as_deref(),
            value,
            place,
            0,
        )?;
        Ok((Slot::Word(reached.address as u64), reached.count))
    }

    fn reached(
        &mut self,
        kind: u32,
        element: u32,
        type_name: Option<&str>,
        value: Option<&Value>,
        place: &str,
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
            (KIND_POINTER, _) => self.pointed(element, type_name, value, place, depth),
            (KIND_ARRAY, Value::Array(items)) => {
                self.array(element, type_name, items, place, depth)
            }
            (KIND_HANDLE, Value::Handle(handle)) => Ok(Reached {
                address: handle
                    .pointer_for(self.library, type_name.unwrap_or_default())
                    .map_err(|why| format!("{place}: {why}"))?,
                count: 0,
            }),
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

    fn pointed(
        &mut self,
        element: u32,
        type_name: Option<&str>,
        value: &Value,
        place: &str,
        depth: usize,
    ) -> Result<Reached, String> {
        let bytes = self.layouts.element_size(element, type_name).unwrap_or(0);
        let address = self.arena.block(bytes);
        self.element(element, type_name, address, value, place, depth + 1)?;
        Ok(Reached { address, count: 0 })
    }

    fn array(
        &mut self,
        element: u32,
        type_name: Option<&str>,
        items: &[Value],
        place: &str,
        depth: usize,
    ) -> Result<Reached, String> {
        let stride = self.layouts.element_size(element, type_name).unwrap_or(0);
        let address = self.arena.block(stride * items.len());
        for (index, item) in items.iter().enumerate() {
            self.element(
                element,
                type_name,
                address + index * stride,
                item,
                &format!("{place}[{index}]"),
                depth + 1,
            )?;
        }
        Ok(Reached {
            address,
            count: items.len(),
        })
    }

    /// Writes one element a pointer or an array reaches at `address`, inside a block of the arena's.
    fn element(
        &mut self,
        element: u32,
        type_name: Option<&str>,
        address: usize,
        value: &Value,
        place: &str,
        depth: usize,
    ) -> Result<(), String> {
        match (element, value) {
            (KIND_STRUCT, Value::Struct(members)) => {
                let laid_out = struct_of(self.layouts, type_name, place)?;
                self.write_struct(address, laid_out, members, place, depth)
            }
            (KIND_VALUE, Value::Tree(tree)) => self.write_node(address, tree, place, depth),
            (scalar, value) => match scalar_slot(scalar, Some(value), place)? {
                // SAFETY: the element lies inside a block sized for its kind.
                Some(slot) => unsafe { put_slot(address, scalar, slot) },
                None => Err(wrong(place, scalar, value)),
            },
        }
    }

    /// Writes `members` as the struct `laid_out` at `address`, its leading size set and a member left out at its
    /// kind's zero.
    fn write_struct(
        &mut self,
        address: usize,
        laid_out: &LaidOut,
        members: &Members,
        place: &str,
        depth: usize,
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
            let count = self.write_member(
                address,
                member,
                members.get(&member.name),
                &member_place,
                depth,
            )?;
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
    fn write_member(
        &mut self,
        base: usize,
        member: &Member,
        value: Option<&Value>,
        place: &str,
        depth: usize,
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
                let empty = Members::new();
                let members = match value {
                    None => &empty,
                    Some(Value::Struct(members)) => members,
                    Some(other) => return Err(wrong(place, KIND_STRUCT, other)),
                };
                self.write_struct(address, laid_out, members, place, depth + 1)?;
                Ok(0)
            }
            KIND_VALUE => match value {
                None => Ok(0),
                Some(Value::Tree(tree)) => self.write_node(address, tree, place, 0).map(|_| 0),
                Some(other) => Err(wrong(place, KIND_VALUE, other)),
            },
            KIND_CONTEXT | KIND_STORAGE => Ok(0),
            kind => {
                let reached = self.reached(
                    kind,
                    member.element,
                    member.type_name.as_deref(),
                    value,
                    place,
                    depth,
                )?;
                // SAFETY: a pointer-sized member lies inside its struct's block.
                unsafe { put(address, reached.address) };
                Ok(reached.count)
            }
        }
    }

    /// Writes `tree` as the value node at `address`, every node and text below it in the arena.
    fn write_node(
        &mut self,
        address: usize,
        tree: &Tree,
        place: &str,
        depth: usize,
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
                node.items = self.nodes(items, place, depth)? as *const ValueNode;
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

    fn nodes(&mut self, items: &[Tree], place: &str, depth: usize) -> Result<usize, String> {
        let address = self.arena.block(items.len() * size_of::<ValueNode>());
        for (index, item) in items.iter().enumerate() {
            self.write_node(
                address + index * size_of::<ValueNode>(),
                item,
                &format!("{place}[{index}]"),
                depth + 1,
            )?;
        }
        Ok(address)
    }

    fn entries(
        &mut self,
        entries: &[(String, Tree)],
        place: &str,
        depth: usize,
    ) -> Result<usize, String> {
        let mut keys = BTreeSet::new();
        let address = self.arena.block(entries.len() * size_of::<ValueEntry>());
        for (index, (key, value)) in entries.iter().enumerate() {
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
            self.write_node(
                entry + offset_of!(ValueEntry, value),
                value,
                &format!("{place}.{key}"),
                depth + 1,
            )?;
        }
        Ok(address)
    }
}

fn struct_of<'layouts>(
    layouts: &'layouts Layouts,
    type_name: Option<&str>,
    place: &str,
) -> Result<&'layouts LaidOut, String> {
    layouts
        .laid_out(type_name)
        .ok_or_else(|| format!("{place} reaches a struct its description lacks"))
}

/// The slot of a scalar kind, from its value or as its zero; none for any other kind.
fn scalar_slot(kind: u32, value: Option<&Value>, place: &str) -> Result<Option<Slot>, String> {
    let slot = match (kind, value) {
        (KIND_U32 | KIND_U64 | KIND_I64 | KIND_SIZE, None) => Slot::Word(0),
        (KIND_F64, None) => Slot::Float(0.0),
        (KIND_U32, Some(Value::U32(number))) => Slot::Word(u64::from(*number)),
        (KIND_U64, Some(Value::U64(number))) => Slot::Word(*number),
        (KIND_I64, Some(Value::I64(number))) => Slot::Word(*number as u64),
        (KIND_SIZE, Some(Value::Size(number))) => Slot::Word(*number as u64),
        (KIND_F64, Some(Value::F64(number))) => Slot::Float(*number),
        (KIND_U32 | KIND_U64 | KIND_I64 | KIND_SIZE | KIND_F64, Some(other)) => {
            return Err(wrong(place, kind, other));
        }
        _ => return Ok(None),
    };
    Ok(Some(slot))
}

/// # Safety
/// `address` is writable for the scalar `kind`.
unsafe fn put_slot(address: usize, kind: u32, slot: Slot) -> Result<(), String> {
    // SAFETY: as the caller promised.
    unsafe {
        match (kind, slot) {
            (KIND_U32, Slot::Word(word)) => put(address, word as u32),
            (KIND_F64, Slot::Float(float)) => put(address, float),
            (_, Slot::Word(word)) => put(address, word),
            (_, Slot::Float(_)) => return Err("a float slot for an integer kind".to_owned()),
        }
    }
    Ok(())
}

/// What reading an answer makes of a handle and of a definition it carries.
pub(crate) trait Answering {
    /// The handle at `pointer`, of the opaque `type_name`, which the release named `release` frees.
    fn handle(&mut self, pointer: usize, type_name: &str, release: Option<&str>) -> Value;
    /// The definition at `pointer`, of the struct `type_name`, which a nested description describes.
    fn definition(&mut self, pointer: usize, type_name: &str) -> Result<Value, String>;
}

/// Reads answers back from the memory their provider filled, by the layouts the definition was bound with.
pub(crate) struct Reader<'a> {
    pub(crate) layouts: &'a Layouts,
    pub(crate) answering: &'a mut dyn Answering,
}

/// The text at `pointer`, or none for NULL.
///
/// # Safety
/// `pointer` is NULL or a NUL-terminated text that lives across the call.
unsafe fn text_at(pointer: usize) -> Option<String> {
    if pointer == 0 {
        return None;
    }
    // SAFETY: as the caller promised.
    Some(
        unsafe { CStr::from_ptr(pointer as *const c_char) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// The `length` bytes at `pointer` as text; none for NULL.
///
/// # Safety
/// `pointer` is NULL or `length` readable bytes.
unsafe fn counted_text(pointer: usize, length: usize) -> String {
    if pointer == 0 {
        return String::new();
    }
    // SAFETY: as the caller promised.
    let bytes = unsafe { std::slice::from_raw_parts(pointer as *const u8, length) };
    String::from_utf8_lossy(bytes).into_owned()
}

/// The scalar of `kind` at `address`, or none for any other kind.
///
/// # Safety
/// `address` holds a value of the scalar `kind`.
unsafe fn scalar_at(address: usize, kind: u32) -> Option<Value> {
    // SAFETY: as the caller promised.
    unsafe {
        match kind {
            KIND_U32 => Some(Value::U32(get(address))),
            KIND_U64 => Some(Value::U64(get(address))),
            KIND_I64 => Some(Value::I64(get(address))),
            KIND_F64 => Some(Value::F64(get(address))),
            KIND_SIZE => Some(Value::Size(get(address))),
            _ => None,
        }
    }
}

impl Reader<'_> {
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
            match parameter.kind {
                KIND_POINTER => {
                    let laid_out =
                        struct_of(self.layouts, parameter.type_name.as_deref(), "an answer")?;
                    Ok(Some(Value::Struct(self.read_struct(block, laid_out, 0)?)))
                }
                KIND_VALUE => {
                    let root = block + offset_of!(abi::ValueTree, root);
                    Ok(self.tree(root, 0)?.map(Value::Tree))
                }
                KIND_TEXT => Ok(Some(Value::Text(text_at(get(block))))),
                kind => self.pointer_answer(
                    kind,
                    get(block),
                    parameter.type_name.as_deref(),
                    parameter.release.as_deref(),
                ),
            }
        }
    }

    fn pointer_answer(
        &mut self,
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
            KIND_HANDLE => Ok(Some(self.answering.handle(pointer, type_name, release))),
            KIND_DEFINITION => self.answering.definition(pointer, type_name).map(Some),
            other => Err(format!("an answer of kind {other}, which no binding reads")),
        }
    }

    /// Reads the struct `laid_out` at `address`: every visible member, but a NULL pointer, a NULL handle, a NULL
    /// definition and a value of kind NONE.
    ///
    /// # Safety
    /// `address` holds the struct, as its provider filled it.
    unsafe fn read_struct(
        &mut self,
        address: usize,
        laid_out: &LaidOut,
        depth: usize,
    ) -> Result<Members, String> {
        if depth > DEPTH_LIMIT {
            return Err(format!(
                "an answer nests structs deeper than {DEPTH_LIMIT} levels"
            ));
        }
        let described = &laid_out.described;
        let mut members = Members::new();
        for index in &laid_out.visible {
            let member = &described.members[*index];
            let count = member
                .count
                .as_deref()
                .and_then(|name| described.member_named(name))
                // SAFETY: a count is a SIZE member of the same struct.
                .map_or(0, |counted_by| unsafe {
                    get::<usize>(address + counted_by.offset)
                });
            // SAFETY: as the caller promised.
            if let Some(value) =
                unsafe { self.member(address + member.offset, member, count, depth)? }
            {
                members.insert(member.name.clone(), value);
            }
        }
        Ok(members)
    }

    /// # Safety
    /// `address` holds the member, as its provider filled it, and `count` is the member counting it.
    unsafe fn member(
        &mut self,
        address: usize,
        member: &Member,
        count: usize,
        depth: usize,
    ) -> Result<Option<Value>, String> {
        // SAFETY: as the caller promised.
        unsafe {
            if let Some(scalar) = scalar_at(address, member.kind) {
                return Ok(Some(scalar));
            }
            let type_name = member.type_name.as_deref();
            match member.kind {
                KIND_TEXT => Ok(Some(Value::Text(text_at(get(address))))),
                KIND_TEXTS => Ok(Some(Value::Texts(texts_at(get(address), count)))),
                KIND_BYTES => Ok(Some(Value::Bytes(bytes_at(get(address), count)))),
                KIND_STRUCT => {
                    let laid_out = struct_of(self.layouts, type_name, "an answer")?;
                    Ok(Some(Value::Struct(self.read_struct(
                        address,
                        laid_out,
                        depth + 1,
                    )?)))
                }
                KIND_VALUE => Ok(self.tree(address, 0)?.map(Value::Tree)),
                KIND_POINTER => self.pointed(member.element, type_name, get(address), depth),
                KIND_ARRAY => self
                    .array(member.element, type_name, get(address), count, depth)
                    .map(Some),
                kind => {
                    self.pointer_answer(kind, get(address), type_name, member.release.as_deref())
                }
            }
        }
    }

    /// # Safety
    /// `pointer` is NULL or what a POINTER of `element` reaches.
    unsafe fn pointed(
        &mut self,
        element: u32,
        type_name: Option<&str>,
        pointer: usize,
        depth: usize,
    ) -> Result<Option<Value>, String> {
        if pointer == 0 {
            return Ok(None);
        }
        // SAFETY: as the caller promised.
        unsafe { self.element(element, type_name, pointer, depth + 1) }
    }

    /// # Safety
    /// `pointer` is NULL or `count` elements of `element`.
    unsafe fn array(
        &mut self,
        element: u32,
        type_name: Option<&str>,
        pointer: usize,
        count: usize,
        depth: usize,
    ) -> Result<Value, String> {
        if pointer == 0 {
            return Ok(Value::Array(Vec::new()));
        }
        let stride = self.layouts.element_size(element, type_name).unwrap_or(0);
        let mut items = Vec::with_capacity(count);
        for index in 0..count {
            // SAFETY: as the caller promised.
            let item =
                unsafe { self.element(element, type_name, pointer + index * stride, depth + 1)? };
            items.push(
                item.ok_or_else(|| "an array of values holds a node of kind NONE".to_owned())?,
            );
        }
        Ok(Value::Array(items))
    }

    /// # Safety
    /// `address` holds one element of `element`.
    unsafe fn element(
        &mut self,
        element: u32,
        type_name: Option<&str>,
        address: usize,
        depth: usize,
    ) -> Result<Option<Value>, String> {
        // SAFETY: as the caller promised.
        unsafe {
            match element {
                KIND_STRUCT => {
                    let laid_out = struct_of(self.layouts, type_name, "an answer")?;
                    Ok(Some(Value::Struct(
                        self.read_struct(address, laid_out, depth)?,
                    )))
                }
                KIND_VALUE => Ok(self.tree(address, 0)?.map(Value::Tree)),
                scalar => Ok(scalar_at(address, scalar)),
            }
        }
    }

    /// Reads the value node at `address`, none for one of kind NONE.
    ///
    /// # Safety
    /// `address` holds a value node its provider built.
    unsafe fn tree(&mut self, address: usize, depth: usize) -> Result<Option<Tree>, String> {
        if depth > DEPTH_LIMIT {
            return Err(format!(
                "an answer nests a value deeper than {DEPTH_LIMIT} levels"
            ));
        }
        // SAFETY: as the caller promised.
        let node: ValueNode = unsafe { get(address) };
        let tree = match node.kind {
            abi::VALUE_NONE => return Ok(None),
            abi::VALUE_NULL => Tree::Null,
            abi::VALUE_BOOLEAN => Tree::Boolean(node.boolean != 0),
            abi::VALUE_NUMBER => Tree::Number(node.number),
            // SAFETY: a text node holds `length` bytes at `text`.
            abi::VALUE_TEXT => Tree::Text(unsafe { counted_text(node.text as usize, node.length) }),
            // SAFETY: a list node holds `count` nodes at `items`.
            abi::VALUE_LIST => Tree::List(unsafe { self.items(&node, depth)? }),
            // SAFETY: a record node holds `count` entries at `entries`.
            abi::VALUE_RECORD => Tree::Record(unsafe { self.record(&node, depth)? }),
            other => {
                return Err(format!(
                    "an answer holds a value of kind {other}, which no binding knows"
                ));
            }
        };
        Ok(Some(tree))
    }

    /// # Safety
    /// `node` is a list node its provider built.
    unsafe fn items(&mut self, node: &ValueNode, depth: usize) -> Result<Vec<Tree>, String> {
        let mut items = Vec::with_capacity(node.count);
        for index in 0..node.count {
            let address = node.items as usize + index * size_of::<ValueNode>();
            // SAFETY: as the caller promised.
            let item = unsafe { self.tree(address, depth + 1)? };
            items.push(item.ok_or_else(|| "a list holds an item of kind NONE".to_owned())?);
        }
        Ok(items)
    }

    /// # Safety
    /// `node` is a record node its provider built.
    unsafe fn record(
        &mut self,
        node: &ValueNode,
        depth: usize,
    ) -> Result<Vec<(String, Tree)>, String> {
        let mut keys = BTreeSet::new();
        let mut entries = Vec::with_capacity(node.count);
        for index in 0..node.count {
            // SAFETY: as the caller promised.
            let entry: ValueEntry =
                unsafe { get(node.entries as usize + index * size_of::<ValueEntry>()) };
            // SAFETY: a key holds `key_length` bytes.
            let key = unsafe { counted_text(entry.key as usize, entry.key_length) };
            if !keys.insert(key.clone()) {
                return Err(format!("an answer's record holds the key {key} twice"));
            }
            let value_address = node.entries as usize
                + index * size_of::<ValueEntry>()
                + offset_of!(ValueEntry, value);
            // SAFETY: as the caller promised.
            let value = unsafe { self.tree(value_address, depth + 1)? };
            let value = value.ok_or_else(|| format!("a record's {key} is of kind NONE"))?;
            entries.push((key, value));
        }
        Ok(entries)
    }

    /// The `detail` and `fix` texts of the refusal struct `laid_out` at `address`.
    pub(crate) fn refusal_texts(
        laid_out: &LaidOut,
        address: usize,
    ) -> (Option<String>, Option<String>) {
        let text = |name: &str| {
            laid_out
                .described
                .member_named(name)
                // SAFETY: the check proved every refusal's struct carries `detail` and `fix` TEXT members, and the
                // block is the refusal the binder handed the provider.
                .and_then(|member| unsafe { text_at(get(address + member.offset)) })
        };
        (text("detail"), text("fix"))
    }
}

/// # Safety
/// `pointer` is NULL or `count` text pointers.
unsafe fn texts_at(pointer: usize, count: usize) -> Vec<Option<String>> {
    if pointer == 0 {
        return Vec::new();
    }
    (0..count)
        // SAFETY: as the caller promised.
        .map(|index| unsafe { text_at(get(pointer + index * size_of::<usize>())) })
        .collect()
}

/// # Safety
/// `pointer` is NULL or `count` bytes.
unsafe fn bytes_at(pointer: usize, count: usize) -> Vec<u8> {
    if pointer == 0 {
        return Vec::new();
    }
    // SAFETY: as the caller promised.
    unsafe { std::slice::from_raw_parts(pointer as *const u8, count) }.to_vec()
}

/// The `size` the definition's struct at `address` begins with.
///
/// Its caller hands only a definition's struct its library answered, while the binary is mapped.
pub(crate) fn definition_size(address: usize) -> usize {
    // SAFETY: every definition's struct begins with its `uint32_t size`, which the host read already at the load.
    unsafe { get::<u32>(address) as usize }
}

/// The pointer-sized word at `offset` of the definition's struct at `address`, or 0 where the `answered_size` bytes
/// its library answered end before the word does.
///
/// Its caller hands only a definition's struct its library answered, while the binary is mapped, and an offset its
/// checked description gives.
pub(crate) fn definition_word(address: usize, answered_size: usize, offset: usize) -> usize {
    if offset + size_of::<usize>() > answered_size {
        return 0;
    }
    // SAFETY: the word lies inside the `answered_size` bytes the definition's struct holds.
    unsafe { get::<usize>(address + offset) }
}
