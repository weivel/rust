//! The binder's memory: blocks a call's inputs and answers live in, and the reads and writes every walk over them is
//! built from. Writing is `write.rs`'s and reading `read.rs`'s.

use std::cell::RefCell;
use std::ffi::{CStr, c_char};

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

/// The memory of one call: every block and text its inputs are written into and the blocks its answers and refusal
/// are handed, cut from chunks that each stay at their address, and every handle it was handed, kept from being freed
/// until the call has returned. A call takes one spare arena of its thread and gives it back emptied, so a call
/// allocates no memory of its own once its thread has made one like it.
#[derive(Default)]
pub(crate) struct Arena {
    chunks: Vec<Block>,
    current: usize,
    /// Words used of the current chunk.
    used: usize,
    pub(super) handles: Vec<HandleUse>,
}

/// The words of the first chunk an arena cuts from: room for the inputs and answers of most calls.
const FIRST_CHUNK_WORDS: usize = 512;
/// The most words a spare arena keeps, so one large call does not hold its memory for the thread's life.
const MOST_SPARE_WORDS: usize = 8 * 1024;
/// The most spare arenas a thread keeps: one per call its thread can have running inside another.
const MOST_SPARE_ARENAS: usize = 4;

thread_local! {
    static SPARE_ARENAS: RefCell<Vec<Arena>> = const { RefCell::new(Vec::new()) };
}

impl Arena {
    /// A spare arena of this thread, or a new one.
    pub(crate) fn take() -> Arena {
        SPARE_ARENAS
            .try_with(|spare| spare.borrow_mut().pop())
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    /// Empties the arena, releasing every handle use it holds, and keeps it as a spare of this thread.
    pub(crate) fn give_back(mut self) {
        self.handles.clear();
        self.current = 0;
        self.used = 0;
        let mut kept = 0;
        self.chunks.retain(|chunk| {
            kept += chunk.words;
            kept <= MOST_SPARE_WORDS
        });
        let _ = SPARE_ARENAS.try_with(|spare| {
            let mut spare = spare.borrow_mut();
            if spare.len() < MOST_SPARE_ARENAS {
                spare.push(self);
            }
        });
    }

    /// A zeroed block of at least `bytes`, aligned for any member a description holds, which stays at its address
    /// until the arena is given back or dropped.
    pub(super) fn block(&mut self, bytes: usize) -> usize {
        let words = bytes.div_ceil(8).max(1);
        loop {
            if let Some(chunk) = self.chunks.get(self.current) {
                if chunk.words - self.used >= words {
                    let address = chunk.address() + self.used * size_of::<u64>();
                    self.used += words;
                    // SAFETY: the words lie inside the chunk, which the arena owns, and no other block holds them.
                    unsafe { std::ptr::write_bytes(address as *mut u64, 0, words) };
                    return address;
                }
                if self.current + 1 < self.chunks.len() {
                    self.current += 1;
                    self.used = 0;
                    continue;
                }
            }
            let last = self.chunks.last().map_or(0, |chunk| chunk.words);
            let size = words.max(FIRST_CHUNK_WORDS).max(last * 2);
            self.chunks.push(Block::zeroed(size * size_of::<u64>()));
            self.current = self.chunks.len() - 1;
            self.used = 0;
        }
    }

    /// `text` as a NUL-terminated C text in the arena, refusing one that holds a NUL.
    pub(super) fn text(&mut self, text: &str) -> Result<usize, String> {
        if text.as_bytes().contains(&0) {
            return Err(format!(
                "the text {text:?} holds a NUL, which no C text can"
            ));
        }
        let address = self.block(text.len() + 1);
        // SAFETY: the block holds the text and its NUL, which the zeroed block already ends in.
        unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), address as *mut u8, text.len()) };
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

/// The layout of the struct `type_name`; `place` spells out where it was met, only when the description lacks it.
pub(super) fn struct_of<'layouts>(
    layouts: &'layouts Layouts,
    type_name: Option<&str>,
    place: &dyn Fn() -> String,
) -> Result<&'layouts LaidOut, String> {
    layouts
        .laid_out(type_name)
        .ok_or_else(|| format!("{} reaches a struct its description lacks", place()))
}

/// The slot of a scalar kind, from its value or as its zero; none for any other kind. `place` spells out where the
/// value lies, only when its kind does not hold it.
pub(super) fn scalar_slot(
    kind: u32,
    value: Option<&Value>,
    place: &dyn Fn() -> String,
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
            return Err(wrong(&place(), kind, other));
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
