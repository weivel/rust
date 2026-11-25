//! A binding of the standard's `described` mirror written by hand for its one operation `reflect`, the control the
//! `described-call` gate measures the generic binding against.
//!
//! The fixture's structs are declared here alone, from `test/described/described.h`, `weivel/value.h` and
//! `weivel/services.h`, for 64-bit platforms, as the binding refuses any other.

use std::ffi::{CString, c_char, c_void};
use std::path::Path;

use super::bare::text_at;
use crate::core::binary::SharedLibrary;

#[repr(C)]
#[derive(Clone, Copy)]
struct HandValueNode {
    kind: u32,
    boolean: u32,
    number: f64,
    text: *const c_char,
    length: usize,
    items: *const HandValueNode,
    entries: *const HandEntry,
    count: usize,
}

#[repr(C)]
struct HandEntry {
    key: *const c_char,
    key_length: usize,
    value: HandValueNode,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct HandPointNode {
    label: *const c_char,
    x: i64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct HandSampleNode {
    size: u32,
    shade: u32,
    tier: u32,
    flags: u32,
    big: u64,
    delta: i64,
    ratio: f64,
    label: *const c_char,
    tags: *const *const c_char,
    tag_count: usize,
    bytes: *const u8,
    length: usize,
    origin: HandPointNode,
    best: *const HandPointNode,
    points: *const HandPointNode,
    point_count: usize,
    numbers: *const u32,
    number_count: usize,
    held: HandValueNode,
    extra: *const HandValueNode,
    values: *const HandValueNode,
    value_count: usize,
}

#[repr(C)]
struct HandReflection {
    size: u32,
    sample: HandSampleNode,
    storage: *mut c_void,
}

#[repr(C)]
struct HandRefusal {
    size: u32,
    detail: *const c_char,
    fix: *const c_char,
    storage: *mut c_void,
}

type Reflect = unsafe extern "C" fn(
    *mut c_void,
    *const HandSampleNode,
    *mut HandReflection,
    *mut HandRefusal,
) -> i32;
type Release = unsafe extern "C" fn(*mut c_void, *mut c_void);
type AddressSymbol = unsafe extern "C" fn() -> *const c_void;

/// The mirror's head up to `release_refusal`, as a binding written for one operation declares it.
#[repr(C)]
struct HandMirrorHead {
    size: u32,
    context: *mut c_void,
    reflect: Reflect,
    tree: *const c_void,
    find: *const c_void,
    measure: *const c_void,
    negate: *const c_void,
    halve: *const c_void,
    invert: *const c_void,
    swap_shade: *const c_void,
    grow: *const c_void,
    weigh: *const c_void,
    count_releases: *const c_void,
    release_reflection: Release,
    release_tree: *const c_void,
    release_refusal: Release,
}

const NULL_NODE: HandValueNode = HandValueNode {
    kind: 0,
    boolean: 0,
    number: 0.0,
    text: std::ptr::null(),
    length: 0,
    items: std::ptr::null(),
    entries: std::ptr::null(),
    count: 0,
};

const NULL_POINT: HandPointNode = HandPointNode {
    label: std::ptr::null(),
    x: 0,
    y: 0.0,
};

/// A value tree node copied out into Rust, as a hand binding answers one.
#[derive(Debug, Clone, PartialEq)]
pub enum HandValue {
    /// Kind NONE.
    None,
    /// Kind NULL.
    Null,
    /// A boolean.
    Boolean(bool),
    /// A number.
    Number(f64),
    /// A text.
    Text(String),
    /// A list.
    List(Vec<HandValue>),
    /// A record.
    Record(Vec<(String, HandValue)>),
}

/// A point copied out into Rust.
#[derive(Debug, Clone, PartialEq)]
pub struct HandPoint {
    /// Its label.
    pub label: Option<String>,
    /// Its x.
    pub x: i64,
    /// Its y.
    pub y: f64,
}

/// Every member of a reflected sample, copied out into Rust values.
#[derive(Debug, Clone, PartialEq)]
pub struct HandSample {
    /// The member of that name.
    pub shade: u32,
    /// The member of that name.
    pub tier: u32,
    /// The member of that name.
    pub flags: u32,
    /// The member of that name.
    pub big: u64,
    /// The member of that name.
    pub delta: i64,
    /// The member of that name.
    pub ratio: f64,
    /// The member of that name.
    pub label: Option<String>,
    /// The member of that name.
    pub tags: Vec<Option<String>>,
    /// The member of that name.
    pub bytes: Vec<u8>,
    /// The member of that name.
    pub origin: HandPoint,
    /// The member of that name.
    pub best: Option<HandPoint>,
    /// The member of that name.
    pub points: Vec<HandPoint>,
    /// The member of that name.
    pub numbers: Vec<u32>,
    /// The member of that name.
    pub held: HandValue,
    /// The member of that name.
    pub extra: Option<HandValue>,
    /// The member of that name.
    pub values: Vec<HandValue>,
}

/// A slice of `count` elements at `pointer`, empty for NULL.
///
/// # Safety
/// `pointer` is NULL or points at `count` elements that live as long as the answer.
unsafe fn elements<'a, T>(pointer: *const T, count: usize) -> &'a [T] {
    if pointer.is_null() || count == 0 {
        return &[];
    }
    // SAFETY: as the caller promised.
    unsafe { std::slice::from_raw_parts(pointer, count) }
}

/// # Safety
/// `text` is NULL or points at `length` bytes that live across the call.
unsafe fn counted_text(text: *const c_char, length: usize) -> String {
    // SAFETY: as the caller promised.
    let bytes = unsafe { elements(text.cast::<u8>(), length) };
    String::from_utf8_lossy(bytes).into_owned()
}

/// # Safety
/// `node` is a node the mirror answered, every pointer under it valid while its reflection is unreleased.
unsafe fn copied_value(node: &HandValueNode) -> HandValue {
    // SAFETY: every read below is of the answer, as the caller promised.
    unsafe {
        match node.kind {
            1 => HandValue::Null,
            2 => HandValue::Boolean(node.boolean != 0),
            3 => HandValue::Number(node.number),
            4 => HandValue::Text(counted_text(node.text, node.length)),
            5 => HandValue::List(
                elements(node.items, node.count)
                    .iter()
                    .map(|item| copied_value(item))
                    .collect(),
            ),
            6 => HandValue::Record(
                elements(node.entries, node.count)
                    .iter()
                    .map(|entry| {
                        (
                            counted_text(entry.key, entry.key_length),
                            copied_value(&entry.value),
                        )
                    })
                    .collect(),
            ),
            _ => HandValue::None,
        }
    }
}

/// # Safety
/// As for [`copied_value`].
unsafe fn copied_point(point: &HandPointNode) -> HandPoint {
    HandPoint {
        // SAFETY: as the caller promised.
        label: unsafe { text_at(point.label) },
        x: point.x,
        y: point.y,
    }
}

/// # Safety
/// `sample` is the sample of a reflection the mirror answered and has not released.
unsafe fn copied_sample(sample: &HandSampleNode) -> HandSample {
    // SAFETY: every read below is of the answer, as the caller promised.
    unsafe {
        HandSample {
            shade: sample.shade,
            tier: sample.tier,
            flags: sample.flags,
            big: sample.big,
            delta: sample.delta,
            ratio: sample.ratio,
            label: text_at(sample.label),
            tags: elements(sample.tags, sample.tag_count)
                .iter()
                .map(|tag| text_at(*tag))
                .collect(),
            bytes: elements(sample.bytes, sample.length).to_vec(),
            origin: copied_point(&sample.origin),
            best: sample.best.as_ref().map(|best| copied_point(best)),
            points: elements(sample.points, sample.point_count)
                .iter()
                .map(|point| copied_point(point))
                .collect(),
            numbers: elements(sample.numbers, sample.number_count).to_vec(),
            held: copied_value(&sample.held),
            extra: sample.extra.as_ref().map(|extra| copied_value(extra)),
            values: elements(sample.values, sample.value_count)
                .iter()
                .map(|value| copied_value(value))
                .collect(),
        }
    }
}

/// The mirror's `reflect` bound by hand on a load of its own: the sample it is handed, a label and a counted array of texts, written into
/// a struct declared here, and one answer and one refusal block kept across calls, as a binding written by hand for
/// one operation keeps them.
pub(crate) struct HandMirror {
    context: *mut c_void,
    reflect: Reflect,
    release_reflection: Release,
    release_refusal: Release,
    _label: CString,
    _tags: Vec<CString>,
    tag_pointers: Vec<*const c_char>,
    label_pointer: *const c_char,
    reflection: HandReflection,
    refusal: HandRefusal,
    _library: SharedLibrary,
}

impl HandMirror {
    /// Opens the described library at `path` through the platform's loader, reads the mirror
    /// `described_mirror_definition` answers and prepares the sample `label` and `tags`; `None` for a path that is no
    /// such library, a mirror too small to carry `release_refusal`, or a text holding a NUL.
    pub(crate) fn open(path: &Path, label: &str, tags: &[&str]) -> Option<HandMirror> {
        let library = SharedLibrary::open(path)?;
        let symbol = library.symbol(c"described_mirror_definition")?;
        // SAFETY: a definition symbol is `const void *f(void)`, as `library.h` declares it.
        let address =
            unsafe { std::mem::transmute::<*mut c_void, AddressSymbol>(symbol)() } as usize;
        if address == 0 {
            return None;
        }
        // SAFETY: the address is what the symbol answered, from the binary kept open below; the struct begins with
        // its `size`, read before anything past it.
        let size = unsafe { std::ptr::read(address as *const u32) } as usize;
        if size < size_of::<HandMirrorHead>() {
            return None;
        }
        // SAFETY: the struct is at least the head long and lives while the binary is open.
        let head = unsafe { &*(address as *const HandMirrorHead) };
        let label = CString::new(label).ok()?;
        let tags: Vec<CString> = tags
            .iter()
            .map(|tag| CString::new(*tag).ok())
            .collect::<Option<_>>()?;
        Some(HandMirror {
            context: head.context,
            reflect: head.reflect,
            release_reflection: head.release_reflection,
            release_refusal: head.release_refusal,
            tag_pointers: tags.iter().map(|tag| tag.as_ptr()).collect(),
            label_pointer: label.as_ptr(),
            _label: label,
            _tags: tags,
            reflection: HandReflection {
                size: size_of::<HandReflection>() as u32,
                sample: empty_sample(),
                storage: std::ptr::null_mut(),
            },
            refusal: HandRefusal {
                size: size_of::<HandRefusal>() as u32,
                detail: std::ptr::null(),
                fix: std::ptr::null(),
                storage: std::ptr::null_mut(),
            },
            _library: library,
        })
    }

    fn sample(&self) -> HandSampleNode {
        HandSampleNode {
            label: self.label_pointer,
            tags: self.tag_pointers.as_ptr(),
            tag_count: self.tag_pointers.len(),
            ..empty_sample()
        }
    }

    /// Calls `reflect` with the sample, copies every member of its answer out, and releases the answer and the
    /// refusal, answering the copy or the status it answered instead of OK.
    pub(crate) fn reflect(&mut self) -> Result<HandSample, i32> {
        let sample = self.sample();
        // SAFETY: the sample and the texts under it live across the call, and the answer and refusal blocks are this
        // mirror's own, empty as the release leaves them.
        let status = unsafe {
            (self.reflect)(
                self.context,
                &sample,
                &mut self.reflection,
                &mut self.refusal,
            )
        };
        let answered = if status == 0 {
            // SAFETY: on OK the mirror filled the answer, which lives until its release below.
            Ok(unsafe { copied_sample(&self.reflection.sample) })
        } else {
            Err(status)
        };
        // SAFETY: both blocks are this mirror's own; each release empties its block and does nothing to one already
        // empty, as `described.h` defines them.
        unsafe {
            (self.release_reflection)(self.context, (&raw mut self.reflection).cast());
            (self.release_refusal)(self.context, (&raw mut self.refusal).cast());
        }
        answered
    }
}

fn empty_sample() -> HandSampleNode {
    HandSampleNode {
        size: size_of::<HandSampleNode>() as u32,
        shade: 0,
        tier: 0,
        flags: 0,
        big: 0,
        delta: 0,
        ratio: 0.0,
        label: std::ptr::null(),
        tags: std::ptr::null(),
        tag_count: 0,
        bytes: std::ptr::null(),
        length: 0,
        origin: NULL_POINT,
        best: std::ptr::null(),
        points: std::ptr::null(),
        point_count: 0,
        numbers: std::ptr::null(),
        number_count: 0,
        held: NULL_NODE,
        extra: std::ptr::null(),
        values: std::ptr::null(),
        value_count: 0,
    }
}
