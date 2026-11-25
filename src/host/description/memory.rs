//! The binder's memory: blocks a call's inputs and answers live in, and the reads and writes every walk over them is
//! built from. Writing is `write.rs`'s and reading `read.rs`'s.

use std::ffi::{CStr, CString, c_char};

use super::abi::{
    KIND_ARRAY, KIND_BYTES, KIND_DEFINITION, KIND_F64, KIND_HANDLE, KIND_I64, KIND_POINTER,
    KIND_SIZE, KIND_STRUCT, KIND_TEXT, KIND_TEXTS, KIND_U32, KIND_U64, KIND_VALUE,
};
use super::holdings::HandleUse;
use super::invoke::Slot;
use super::layouts::{LaidOut, Layouts};
use super::value::Value;

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
pub(super) unsafe fn put<T>(address: usize, value: T) {
    // SAFETY: as the caller promised.
    unsafe { std::ptr::write_unaligned(address as *mut T, value) }
}

/// # Safety
/// `address` holds a `T` the provider wrote, or the binder did.
pub(super) unsafe fn get<T: Copy>(address: usize) -> T {
    // SAFETY: as the caller promised.
    unsafe { std::ptr::read_unaligned(address as *const T) }
}

/// Every block and text one call's inputs are written into, freed when the call has answered.
#[derive(Default)]
pub(crate) struct Arena {
    blocks: Vec<Block>,
    texts: Vec<CString>,
    /// Every handle the call was handed, kept from being freed until the call has returned.
    pub(super) handles: Vec<HandleUse>,
}

impl Arena {
    pub(super) fn block(&mut self, bytes: usize) -> usize {
        let block = Block::zeroed(bytes);
        let address = block.address();
        self.blocks.push(block);
        address
    }

    pub(super) fn text(&mut self, text: &str) -> Result<usize, String> {
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

pub(super) fn wrong(place: &str, kind: u32, value: &Value) -> String {
    format!(
        "{place} holds a {}, and was handed {value:?}",
        kind_name(kind)
    )
}

pub(super) fn struct_of<'layouts>(
    layouts: &'layouts Layouts,
    type_name: Option<&str>,
    place: &str,
) -> Result<&'layouts LaidOut, String> {
    layouts
        .laid_out(type_name)
        .ok_or_else(|| format!("{place} reaches a struct its description lacks"))
}

/// The slot of a scalar kind, from its value or as its zero; none for any other kind.
pub(super) fn scalar_slot(
    kind: u32,
    value: Option<&Value>,
    place: &str,
) -> Result<Option<Slot>, String> {
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
pub(super) unsafe fn put_slot(address: usize, kind: u32, slot: Slot) -> Result<(), String> {
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

fn utf8(bytes: &[u8]) -> Result<String, String> {
    String::from_utf8(bytes.to_vec())
        .map_err(|_| format!("an answer holds the text {bytes:?}, which is not UTF-8"))
}

/// The text at `pointer`, or none for NULL; a text that is not UTF-8 is the provider's fault, never replaced.
///
/// # Safety
/// `pointer` is NULL or a NUL-terminated text that lives across the call.
pub(super) unsafe fn text_at(pointer: usize) -> Result<Option<String>, String> {
    if pointer == 0 {
        return Ok(None);
    }
    // SAFETY: as the caller promised.
    utf8(unsafe { CStr::from_ptr(pointer as *const c_char) }.to_bytes()).map(Some)
}

/// The `length` bytes at `pointer` as text, empty for NULL; bytes that are not UTF-8 are the provider's fault.
///
/// # Safety
/// `pointer` is NULL or `length` readable bytes.
pub(super) unsafe fn counted_text(pointer: usize, length: usize) -> Result<String, String> {
    if pointer == 0 {
        return Ok(String::new());
    }
    // SAFETY: as the caller promised.
    utf8(unsafe { std::slice::from_raw_parts(pointer as *const u8, length) })
}

/// The scalar of `kind` at `address`, or none for any other kind.
///
/// # Safety
/// `address` holds a value of the scalar `kind`.
pub(super) unsafe fn scalar_at(address: usize, kind: u32) -> Option<Value> {
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

/// # Safety
/// `pointer` is NULL or `count` text pointers.
pub(super) unsafe fn texts_at(pointer: usize, count: usize) -> Result<Vec<Option<String>>, String> {
    if pointer == 0 {
        return Ok(Vec::new());
    }
    (0..count)
        // SAFETY: as the caller promised.
        .map(|index| unsafe { text_at(get(pointer + index * size_of::<usize>())) })
        .collect()
}

/// # Safety
/// `pointer` is NULL or `count` bytes.
pub(super) unsafe fn bytes_at(pointer: usize, count: usize) -> Vec<u8> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_an_answer_holds_that_is_not_utf8_is_the_providers_fault() {
        let invalid = b"in\xffvalid\0";
        // SAFETY: both texts live across the reads.
        unsafe {
            assert!(text_at(invalid.as_ptr() as usize).is_err());
            assert!(counted_text(invalid.as_ptr() as usize, 7).is_err());
            assert_eq!(
                text_at(c"valid".as_ptr() as usize),
                Ok(Some("valid".to_owned()))
            );
        }
    }
}
