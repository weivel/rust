//! Reading what the core hands out into the crate's own values, before the struct carrying it is released.

use std::ffi::{CStr, c_char};

use super::abi;
use super::codes::RefusalCode;
use super::values::{Handle, Number, Refusal};

/// The text an event carries in place of one the core handed out that is not UTF-8.
pub(crate) const UNREADABLE_TEXT: &str =
    "the core handed out a text that is not UTF-8, so it is left out";

/// Reads texts and lists out of the core's memory, remembering whether anything had to be left out.
#[derive(Default)]
pub(crate) struct Reader {
    pub(crate) incomplete: bool,
}

impl Reader {
    /// The text at `pointer`, empty for NULL.
    ///
    /// # Safety
    /// `pointer` is NULL or a NUL-terminated string the core keeps alive for the call.
    pub(crate) unsafe fn text(&mut self, pointer: *const c_char) -> String {
        // SAFETY: as the caller promises.
        unsafe { self.optional_text(pointer) }.unwrap_or_default()
    }

    /// The text at `pointer`, `None` for NULL.
    ///
    /// # Safety
    /// As for [`Reader::text`].
    pub(crate) unsafe fn optional_text(&mut self, pointer: *const c_char) -> Option<String> {
        if pointer.is_null() {
            return None;
        }
        // SAFETY: as the caller promises.
        let bytes = unsafe { CStr::from_ptr(pointer) }.to_bytes();
        match std::str::from_utf8(bytes) {
            Ok(text) => Some(text.to_owned()),
            Err(_) => {
                self.incomplete = true;
                Some(UNREADABLE_TEXT.to_owned())
            }
        }
    }

    /// Every text of a list.
    ///
    /// # Safety
    /// `strings.items` holds `strings.count` valid texts, or is NULL with a count of 0.
    pub(crate) unsafe fn strings(&mut self, strings: abi::Strings) -> Vec<String> {
        // SAFETY: as the caller promises.
        let items = unsafe { slice(strings.items, strings.count) };
        // SAFETY: each item is a text the core keeps alive, as the caller promises.
        items
            .iter()
            .map(|item| unsafe { self.text(*item) })
            .collect()
    }

    /// Every number of a list, stepped through at the stride its first element declares.
    ///
    /// # Safety
    /// `numbers.items` holds `numbers.count` elements of the stride the first declares, or is NULL with a count of 0.
    pub(crate) unsafe fn numbers(&mut self, numbers: abi::Numbers) -> Vec<Number> {
        // SAFETY: as the caller promises.
        let elements: Vec<abi::Number> = unsafe { strided(numbers.items, numbers.count) };
        elements
            .into_iter()
            // SAFETY: each name is a text the core keeps alive, as the caller promises.
            .map(|number| Number {
                name: unsafe { self.text(number.name) },
                value: number.value,
            })
            .collect()
    }

    /// A refusal with what it is about.
    ///
    /// # Safety
    /// The refusal's texts and `subject` are NULL or texts the core keeps alive.
    pub(crate) unsafe fn refusal(
        &mut self,
        refusal: abi::Refusal,
        subject: *const c_char,
        has_fix: u32,
    ) -> Refusal {
        Refusal {
            code: RefusalCode(refusal.code),
            // SAFETY: as the caller promises, for each of the three texts.
            detail: unsafe { self.text(refusal.detail) },
            fix: unsafe { self.text(refusal.fix) },
            subject: unsafe { self.optional_text(subject) },
            has_fix: has_fix != 0,
        }
    }
}

/// The handles of a list.
///
/// # Safety
/// `items` holds `count` handles, or is NULL with a count of 0.
pub(crate) unsafe fn handles(items: *const u64, count: u32) -> Vec<Handle> {
    // SAFETY: as the caller promises.
    unsafe { slice(items, count) }
        .iter()
        .map(|handle| Handle(*handle))
        .collect()
}

/// The elements of an array that never grows, at the crate's own stride.
///
/// # Safety
/// `items` holds `count` elements of `T`, or is NULL with a count of 0.
pub(crate) unsafe fn slice<'a, T>(items: *const T, count: u32) -> &'a [T] {
    if items.is_null() || count == 0 {
        return &[];
    }
    // SAFETY: as the caller promises.
    unsafe { std::slice::from_raw_parts(items, count as usize) }
}

/// The elements of an array that may grow, each beginning with its `uint32_t size`, stepped through at the stride the
/// first declares. Only the bytes both the core and the crate know are read; a field the core's elements lack keeps
/// its zero default.
///
/// # Safety
/// `items` holds `count` elements of the stride the first declares, or is NULL with a count of 0, and `T` is a plain
/// struct of integers and raw pointers beginning with its size.
pub(crate) unsafe fn strided<T: Copy>(items: *const T, count: u32) -> Vec<T> {
    if items.is_null() || count == 0 {
        return Vec::new();
    }
    // SAFETY: the first element begins with its size, as the caller promises.
    let stride = unsafe { items.cast::<u32>().read_unaligned() } as usize;
    let known = stride.min(std::mem::size_of::<T>());
    (0..count as usize)
        .map(|index| {
            let mut element = abi::answer_struct::<T>();
            // SAFETY: element `index` lies `index * stride` bytes in and spans at least `known` bytes.
            unsafe {
                let source = items.cast::<u8>().add(index * stride);
                std::ptr::copy_nonoverlapping(source, (&raw mut element).cast::<u8>(), known);
            }
            element
        })
        .collect()
}
