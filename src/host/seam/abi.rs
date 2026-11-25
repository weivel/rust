//! The plugin seam's C tables. Each is led by its `size`, as the core's structs are: a side refuses a table below its
//! first shape, the fields it was first published with, reads a field appended later only where the other side's
//! `size` reaches past it, and never reads past that `size`. A table a side receives is copied by value, so nothing it
//! points into outlives the call that handed it, and every text and array is lent for the length of one call.

use std::ffi::c_void;
use std::mem::offset_of;

use crate::core::{
    Answer, FailureKind, Manifest, Number, Refusal, RefusalCode, RefusalStatus, ServiceOffer,
    ServiceWant,
};

/// A text lent for the length of one call: UTF-8, not NUL-terminated, of any length.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SeamText {
    pub(super) pointer: *const u8,
    pub(super) length: usize,
}

/// A named number, lent as its text is.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SeamNumber {
    pub(super) name: SeamText,
    pub(super) value: f64,
}

/// A refusal of the core's, its texts lent.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SeamRefusal {
    pub(super) code: u32,
    pub(super) has_subject: u32,
    pub(super) has_fix: u32,
    pub(super) subject: SeamText,
    pub(super) detail: SeamText,
    pub(super) fix: SeamText,
}

/// What a call across the seam answered, handed to the caller's [`SeamAnswer`] for the length of that hand-over:
/// `kind` 0 for success with `number` and the lists, and otherwise which [`Answer`] it was.
#[repr(C)]
pub struct SeamAnswered {
    pub(super) size: u32,
    pub(super) kind: u32,
    pub(super) status: u32,
    pub(super) has_refusal: u32,
    pub(super) refusal: SeamRefusal,
    pub(super) detail: SeamText,
    pub(super) number: u64,
    pub(super) texts: *const SeamText,
    pub(super) text_count: usize,
    pub(super) addresses: *const usize,
    pub(super) address_count: usize,
    /// Appended: the description of the definition at `number`, and of each of `addresses`, 0 for an object no
    /// description bound, so the other side binds the same definition from it.
    pub(super) description: usize,
    pub(super) descriptions: *const usize,
    /// Appended: the host's id of the library the definition at `number`, and each of `addresses`, lies in.
    pub(super) mapping: u64,
    pub(super) mappings: *const u64,
    /// Appended: bytes the call answered, a value tree in the seam's own encoding where a method answered one.
    pub(super) bytes: *const u8,
    pub(super) byte_count: usize,
}

/// Where the callee hands what it answered: `take` is called once, before the call returns, with `data`.
#[repr(C)]
pub struct SeamAnswer {
    pub(super) size: u32,
    pub(super) data: *mut c_void,
    pub(super) take: unsafe extern "C" fn(*mut c_void, *const SeamAnswered),
}

/// How long a want may wait, each limit present when its flag is 1.
#[repr(C)]
pub struct SeamWantLimits {
    pub(super) size: u32,
    pub(super) has_timeout: u32,
    pub(super) has_budget: u32,
    pub(super) has_poll: u32,
    pub(super) timeout_millis: i64,
    pub(super) budget_millis: i64,
    pub(super) poll_millis: i64,
}

/// A service a manifest provides.
#[repr(C)]
pub struct SeamOffer {
    pub(super) id: SeamText,
    pub(super) attributes: *const SeamNumber,
    pub(super) attribute_count: usize,
}

/// A service a manifest consumes; each optional flag is 0 for absent, 1 for false and 2 for true.
#[repr(C)]
pub struct SeamWant {
    pub(super) id: SeamText,
    pub(super) has_provider: u32,
    pub(super) optional: u32,
    pub(super) degradable: u32,
    pub(super) provider: SeamText,
    pub(super) constraints: *const SeamNumber,
    pub(super) constraint_count: usize,
}

/// A list of texts.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SeamTexts {
    pub(super) texts: *const SeamText,
    pub(super) count: usize,
}

/// A plugin's whole manifest, lent for the activate that hands it.
#[repr(C)]
pub struct SeamManifest {
    pub(super) size: u32,
    pub(super) api: u32,
    pub(super) has_entry: u32,
    pub(super) has_identity: u32,
    pub(super) import_name: SeamText,
    pub(super) entry: SeamText,
    pub(super) identity: SeamText,
    pub(super) targets: SeamTexts,
    pub(super) capabilities_provided: SeamTexts,
    pub(super) capabilities_consumed: SeamTexts,
    pub(super) permissions: SeamTexts,
    pub(super) services_provided: *const SeamOffer,
    pub(super) services_provided_count: usize,
    pub(super) services_consumed: *const SeamWant,
    pub(super) services_consumed_count: usize,
}

/// The host's context table of one plugin. `data` is the host's own id for it, never an address, so a call made after
/// the host let go of it answers destroyed rather than reaching freed memory.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SeamContext {
    pub(super) size: u32,
    pub(super) data: u64,
    pub(super) plugin: u64,
    pub(super) write: unsafe extern "C" fn(u64, u32, SeamText),
    pub(super) report: unsafe extern "C" fn(u64, u32, SeamText),
    pub(super) get: unsafe extern "C" fn(u64, SeamText, *const SeamAnswer),
    pub(super) ids: unsafe extern "C" fn(u64, *const SeamAnswer),
    pub(super) want:
        unsafe extern "C" fn(u64, SeamText, *const SeamWantLimits, u64, *const SeamAnswer),
    pub(super) withdraw: unsafe extern "C" fn(u64, u64, *const SeamAnswer),
    pub(super) watch: unsafe extern "C" fn(u64, SeamText, u32, u64, *const SeamAnswer),
    pub(super) stop_watch: unsafe extern "C" fn(u64, u64, *const SeamAnswer),
    pub(super) register: unsafe extern "C" fn(u64, SeamText, usize, u32, u64, *const SeamAnswer),
    pub(super) unregister: unsafe extern "C" fn(u64, u64, *const SeamAnswer),
    pub(super) release_holding: unsafe extern "C" fn(u64, u64, *const SeamAnswer),
    pub(super) provide: unsafe extern "C" fn(u64, SeamText, usize, *const SeamAnswer),
    pub(super) implementations: unsafe extern "C" fn(u64, SeamText, *const SeamAnswer),
    pub(super) dispatches_here: unsafe extern "C" fn(u64) -> u32,
    /// Appended: holds the library a mapping id names mapped, answering 1, or 0 once it has closed; `unhold` lets go of
    /// one hold.
    pub(super) hold: Option<unsafe extern "C" fn(u64, u64) -> u32>,
    pub(super) unhold: Option<unsafe extern "C" fn(u64, u64)>,
    /// Appended: tells the host that the plugin's object at the address offers the methods named, before it is
    /// registered or provided, so the host's object for it calls them back across the seam by name.
    pub(super) offer_methods:
        Option<unsafe extern "C" fn(u64, usize, SeamTexts, *const SeamAnswer)>,
}

/// What the host hands an activate or a deactivate to finish it with: `data` the host's id for it, so a finish after
/// the host let go of it, or a second one, does nothing.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SeamCompletion {
    pub(super) size: u32,
    pub(super) data: u64,
    pub(super) finish: unsafe extern "C" fn(u64, u32, SeamText),
}

/// Where a plugin hands its answer to a constrained want's question.
#[repr(C)]
pub struct SeamCapacity {
    pub(super) size: u32,
    pub(super) data: *mut c_void,
    pub(super) take: unsafe extern "C" fn(*mut c_void, u32, *const SeamNumber, usize),
}

pub(super) const EVENT_REVOKED: u32 = 1;
pub(super) const EVENT_HOLDINGS_RELEASED: u32 = 2;
pub(super) const EVENT_REACH_REFUSED: u32 = 3;
pub(super) const EVENT_DEGRADED: u32 = 4;
pub(super) const EVENT_WANT_SETTLED: u32 = 5;
pub(super) const EVENT_WATCH_REGISTERED: u32 = 6;
pub(super) const EVENT_WATCH_UNREGISTERED: u32 = 7;
pub(super) const EVENT_CAPABILITY_PROVIDED: u32 = 8;
pub(super) const EVENT_CAPABILITY_WITHDRAWN: u32 = 9;
pub(super) const EVENT_LIBRARY_UNLOADING: u32 = 10;

/// What the host tells a plugin of an event about it, lent for the call. `context` is the context table's `data`,
/// `token` the plugin's own id of the want or watch it concerns, and `settlement` a want's settlement.
#[repr(C)]
pub struct SeamEvent {
    pub(super) size: u32,
    pub(super) kind: u32,
    pub(super) context: u64,
    pub(super) token: u64,
    pub(super) address: usize,
    pub(super) registration: u64,
    pub(super) awaits_release: u32,
    pub(super) reach_kind: u32,
    pub(super) id: SeamText,
    pub(super) refusal: SeamRefusal,
    pub(super) settlement: *const SeamAnswered,
    /// Appended: the description of the definition at `address`, 0 for an object no description bound.
    pub(super) description: usize,
    /// Appended: the host's id of the library that definition lies in, and the library an unloading names.
    pub(super) mapping: u64,
}

/// A plugin's entry table, which its library's entry symbol answers. `data` is the plugin's; each function takes it
/// first. `activate` and `deactivate` answer 0 once they took the completion, which they then finish at most once;
/// `deactivate` answers 1, taking nothing, for an entry that offers none. `forget_context` tells the plugin the host
/// let go of a context, and `free` that the host is done with the table, after which nothing calls it.
/// `action_names` answers the names of the entry's actions as texts, and `call` calls one method by name with a value
/// tree in the seam's encoding: of the entry's actions for address 0, otherwise of the object at the address the
/// plugin registered or provided through the context the id names. A call answers `number` 0 for nothing, 1 for a
/// value tree in `bytes`, and 2 for a failure, its text the one of `texts`; a name it does not offer answers absent.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SeamEntry {
    pub(super) size: u32,
    pub(super) data: *const c_void,
    pub(super) activate: unsafe extern "C" fn(
        *const c_void,
        *const SeamContext,
        *const SeamManifest,
        *const SeamCompletion,
    ) -> u32,
    pub(super) deactivate: unsafe extern "C" fn(*const c_void, *const SeamCompletion) -> u32,
    pub(super) hear: unsafe extern "C" fn(*const c_void, *const SeamEvent),
    pub(super) capacity: unsafe extern "C" fn(
        *const c_void,
        u64,
        u64,
        u32,
        *const SeamNumber,
        usize,
        *const SeamCapacity,
    ),
    pub(super) forget_context: unsafe extern "C" fn(*const c_void, u64),
    pub(super) free: unsafe extern "C" fn(*const c_void),
    /// Appended.
    pub(super) action_names: Option<unsafe extern "C" fn(*const c_void, *const SeamAnswer)>,
    /// Appended.
    pub(super) call: Option<EntryCall>,
}

/// The entry table's `call`: its data, the context's id, the object's address, the method's name, the arguments' bytes
/// and their count, whether arguments were handed at all, and the caller's answer.
pub(super) type EntryCall = unsafe extern "C" fn(
    *const c_void,
    u64,
    usize,
    SeamText,
    *const u8,
    usize,
    u32,
    *const SeamAnswer,
);

macro_rules! first_shape {
    ($type:ty, $last:ident) => {{
        const fn size_of_field<T, F>(_: fn(&T) -> &F) -> usize {
            size_of::<F>()
        }
        offset_of!($type, $last) + size_of_field(|table: &$type| &table.$last)
    }};
}

/// Each table's first shape: the end of the fields it was first published with.
pub(super) const ENTRY_FIRST_SHAPE: usize = first_shape!(SeamEntry, free);
pub(super) const ENTRY_CALL_END: usize = first_shape!(SeamEntry, call);
pub(super) const CONTEXT_OFFER_METHODS_END: usize = first_shape!(SeamContext, offer_methods);
pub(super) const CONTEXT_FIRST_SHAPE: usize = first_shape!(SeamContext, dispatches_here);
pub(super) const COMPLETION_FIRST_SHAPE: usize = first_shape!(SeamCompletion, finish);
pub(super) const EVENT_FIRST_SHAPE: usize = first_shape!(SeamEvent, settlement);
pub(super) const EVENT_DESCRIPTION_END: usize = first_shape!(SeamEvent, description);
pub(super) const EVENT_MAPPING_END: usize = first_shape!(SeamEvent, mapping);
pub(super) const CONTEXT_HOLD_END: usize = first_shape!(SeamContext, unhold);
pub(super) const ANSWER_FIRST_SHAPE: usize = first_shape!(SeamAnswer, take);
pub(super) const ANSWERED_FIRST_SHAPE: usize = first_shape!(SeamAnswered, kind);
pub(super) const MANIFEST_FIRST_SHAPE: usize = first_shape!(SeamManifest, services_consumed_count);
pub(super) const WANT_LIMITS_FIRST_SHAPE: usize = first_shape!(SeamWantLimits, poll_millis);
pub(super) const CAPACITY_FIRST_SHAPE: usize = first_shape!(SeamCapacity, take);

/// Copies a table the other side lent, reading no byte past the `size` it leads with: every field it does not reach is
/// zero, which is `None` for an appended function, since every field past a table's first shape is nullable.
///
/// # Safety
/// `table` points at a table of `T` whose `size` reaches at least its first shape, and every field of `T` past that
/// first shape is valid as zero.
pub(super) unsafe fn copy_table<T>(table: *const T) -> T {
    // SAFETY: as the caller promised, the table begins with its `size`.
    let size = unsafe { table.cast::<u32>().read_unaligned() } as usize;
    let mut copied = std::mem::MaybeUninit::<T>::zeroed();
    // SAFETY: the other side's table spans `size` bytes, and only as many as both know are copied.
    unsafe {
        std::ptr::copy_nonoverlapping(
            table.cast::<u8>(),
            copied.as_mut_ptr().cast::<u8>(),
            size.min(size_of::<T>()),
        );
    }
    // SAFETY: every field up to the first shape was copied, and every field past it is valid as zero.
    unsafe { copied.assume_init() }
}

unsafe extern "C" fn take_into_slot(data: *mut c_void, answered: *const SeamAnswered) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: `data` is the caller's slot for the length of the call, and the answer is lent for it.
        let slot = unsafe { &mut *(data as *mut Option<Result<Answering, Answer>>) };
        *slot = Some(if answered.is_null() {
            Err(Answer::Defect("the other side answered nothing".to_owned()))
        } else {
            // SAFETY: as above.
            unsafe { read_answered(&*answered) }
        });
    }));
}

/// Makes one call of the other side, handing it an answer, and answers what it handed that answer.
pub(super) fn answered_by(call: impl FnOnce(*const SeamAnswer)) -> Result<Answering, Answer> {
    let mut slot: Option<Result<Answering, Answer>> = None;
    let answer = SeamAnswer {
        size: size_of_table::<SeamAnswer>(),
        data: &mut slot as *mut Option<Result<Answering, Answer>> as *mut c_void,
        take: take_into_slot,
    };
    call(&answer);
    slot.unwrap_or_else(|| Err(Answer::Defect("the other side answered nothing".to_owned())))
}

/// Whether a table of `size` reaches past a field ending at `end`.
pub(super) fn carries(size: u32, end: usize) -> bool {
    size as usize >= end
}

pub(super) fn size_of_table<T>() -> u32 {
    size_of::<T>() as u32
}

impl SeamText {
    pub(super) fn of(text: &str) -> SeamText {
        SeamText {
            pointer: text.as_ptr(),
            length: text.len(),
        }
    }

    pub(super) fn empty() -> SeamText {
        SeamText::of("")
    }

    /// The text lent.
    ///
    /// # Safety
    /// The text is lent for the length of the call that reads it.
    pub(super) unsafe fn read(&self) -> String {
        if self.pointer.is_null() || self.length == 0 {
            return String::new();
        }
        // SAFETY: as the caller promised.
        String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(self.pointer, self.length) })
            .into_owned()
    }
}

/// Reads `count` elements lent at `pointer`.
///
/// # Safety
/// The elements are lent for the length of the call.
pub(super) unsafe fn lent<'a, T>(pointer: *const T, count: usize) -> &'a [T] {
    if pointer.is_null() || count == 0 {
        return &[];
    }
    // SAFETY: as the caller promised.
    unsafe { std::slice::from_raw_parts(pointer, count) }
}

/// Numbers in the lent form, kept with the texts they point into.
pub(super) fn lend_numbers(numbers: &[Number]) -> Vec<SeamNumber> {
    numbers
        .iter()
        .map(|number| SeamNumber {
            name: SeamText::of(&number.name),
            value: number.value,
        })
        .collect()
}

/// # Safety
/// The numbers are lent for the length of the call.
pub(super) unsafe fn read_numbers(pointer: *const SeamNumber, count: usize) -> Vec<Number> {
    // SAFETY: as the caller promised.
    unsafe { lent(pointer, count) }
        .iter()
        .map(|number| Number {
            // SAFETY: as above.
            name: unsafe { number.name.read() },
            value: number.value,
        })
        .collect()
}

pub(super) fn lend_texts(texts: &[String]) -> Vec<SeamText> {
    texts.iter().map(|text| SeamText::of(text)).collect()
}

/// # Safety
/// The texts are lent for the length of the call.
pub(super) unsafe fn read_texts(texts: SeamTexts) -> Vec<String> {
    // SAFETY: as the caller promised.
    unsafe { lent(texts.texts, texts.count) }
        .iter()
        // SAFETY: as above.
        .map(|text| unsafe { text.read() })
        .collect()
}

pub(super) fn lend_refusal(refusal: Option<&Refusal>) -> SeamRefusal {
    match refusal {
        None => SeamRefusal {
            code: 0,
            has_subject: 0,
            has_fix: 0,
            subject: SeamText::empty(),
            detail: SeamText::empty(),
            fix: SeamText::empty(),
        },
        Some(refusal) => SeamRefusal {
            code: refusal.code.0,
            has_subject: u32::from(refusal.subject.is_some()),
            has_fix: u32::from(refusal.has_fix),
            subject: SeamText::of(refusal.subject.as_deref().unwrap_or("")),
            detail: SeamText::of(&refusal.detail),
            fix: SeamText::of(&refusal.fix),
        },
    }
}

/// # Safety
/// The refusal's texts are lent for the length of the call.
pub(super) unsafe fn read_refusal(refusal: &SeamRefusal) -> Refusal {
    // SAFETY: as the caller promised.
    unsafe {
        Refusal {
            code: RefusalCode(refusal.code),
            detail: refusal.detail.read(),
            fix: refusal.fix.read(),
            subject: (refusal.has_subject != 0).then(|| refusal.subject.read()),
            has_fix: refusal.has_fix != 0,
        }
    }
}

const ANSWERED: u32 = 0;
const ABSENT: u32 = 1;
const REFUSED: u32 = 2;
const DESTROYED: u32 = 3;
const DEFECT: u32 = 4;
const MISTAKE: u32 = 5;
const DETACHED: u32 = 6;
const CONTRACT_MISMATCH: u32 = 7;

/// What a callee answers across the seam, owned on its side until it is lent to the caller's [`SeamAnswer`].
#[derive(Default)]
pub(super) struct Answering {
    pub(super) number: u64,
    pub(super) texts: Vec<String>,
    pub(super) addresses: Vec<usize>,
    pub(super) description: usize,
    pub(super) descriptions: Vec<usize>,
    pub(super) mapping: u64,
    pub(super) mappings: Vec<u64>,
    pub(super) bytes: Vec<u8>,
}

/// Lends `answered` to `take` for the length of the call.
pub(super) fn lend_answered(
    answered: &Result<Answering, Answer>,
    take: impl FnOnce(&SeamAnswered),
) {
    let failed_with = match answered {
        Err(Answer::ContractMismatch {
            capability,
            operation,
        }) => Answering {
            texts: vec![capability.clone(), operation.clone()],
            ..Answering::default()
        },
        _ => Answering::default(),
    };
    let (value, failure) = match answered {
        Ok(value) => (value, None),
        Err(failure) => (&failed_with, Some(failure)),
    };
    let texts = lend_texts(&value.texts);
    let (kind, status, refusal, detail) = match failure {
        None => (ANSWERED, 0, None, ""),
        Some(Answer::Absent) => (ABSENT, 0, None, ""),
        Some(Answer::Refused { status, refusal }) => (
            REFUSED,
            match status {
                RefusalStatus::Refused => 0,
                RefusalStatus::Malformed => 1,
                RefusalStatus::OutOfMemory => 2,
            },
            refusal.as_ref(),
            "",
        ),
        Some(Answer::Destroyed) => (DESTROYED, 0, None, ""),
        Some(Answer::Defect(detail)) => (DEFECT, 0, None, detail.as_str()),
        Some(Answer::Mistake(detail)) => (MISTAKE, 0, None, detail.as_str()),
        Some(Answer::Detached) => (DETACHED, 0, None, ""),
        Some(Answer::ContractMismatch { .. }) => (CONTRACT_MISMATCH, 0, None, ""),
    };
    take(&SeamAnswered {
        size: size_of_table::<SeamAnswered>(),
        kind,
        status,
        has_refusal: u32::from(refusal.is_some()),
        refusal: lend_refusal(refusal),
        detail: SeamText::of(detail),
        number: value.number,
        texts: texts.as_ptr(),
        text_count: texts.len(),
        addresses: value.addresses.as_ptr(),
        address_count: value.addresses.len(),
        description: value.description,
        descriptions: if value.descriptions.len() == value.addresses.len() {
            value.descriptions.as_ptr()
        } else {
            std::ptr::null()
        },
        mapping: value.mapping,
        mappings: if value.mappings.len() == value.addresses.len() {
            value.mappings.as_ptr()
        } else {
            std::ptr::null()
        },
        bytes: value.bytes.as_ptr(),
        byte_count: value.bytes.len(),
    });
}

/// Reads what a callee answered: below its first shape, the callee's defect; each later field only where its `size`
/// reaches.
///
/// # Safety
/// `answered` and everything it lends live for the length of the call.
pub(super) unsafe fn read_answered(answered: &SeamAnswered) -> Result<Answering, Answer> {
    if !carries(answered.size, ANSWERED_FIRST_SHAPE) {
        return Err(Answer::Defect(
            "the other side of the plugin seam answered a struct too short to say what it answered"
                .to_owned(),
        ));
    }
    let size = answered.size;
    let reaches = |end: usize| carries(size, end);
    // SAFETY: as the caller promised, each field read only where the struct reaches past it.
    let detail = if reaches(first_shape!(SeamAnswered, detail)) {
        unsafe { answered.detail.read() }
    } else {
        String::new()
    };
    let refusal = (reaches(first_shape!(SeamAnswered, refusal)) && answered.has_refusal != 0)
        // SAFETY: as above.
        .then(|| unsafe { read_refusal(&answered.refusal) });
    let status = if reaches(first_shape!(SeamAnswered, status)) {
        answered.status
    } else {
        0
    };
    let texts = || {
        if reaches(first_shape!(SeamAnswered, text_count)) {
            // SAFETY: as above.
            unsafe {
                read_texts(SeamTexts {
                    texts: answered.texts,
                    count: answered.text_count,
                })
            }
        } else {
            Vec::new()
        }
    };
    match answered.kind {
        ANSWERED => Ok(Answering {
            number: if reaches(first_shape!(SeamAnswered, number)) {
                answered.number
            } else {
                0
            },
            texts: texts(),
            addresses: if reaches(first_shape!(SeamAnswered, address_count)) {
                // SAFETY: as above.
                unsafe { lent(answered.addresses, answered.address_count) }.to_vec()
            } else {
                Vec::new()
            },
            description: if reaches(first_shape!(SeamAnswered, description)) {
                answered.description
            } else {
                0
            },
            descriptions: if reaches(first_shape!(SeamAnswered, descriptions)) {
                // SAFETY: as above; the descriptions run beside the addresses.
                unsafe { lent(answered.descriptions, answered.address_count) }.to_vec()
            } else {
                Vec::new()
            },
            mapping: if reaches(first_shape!(SeamAnswered, mapping)) {
                answered.mapping
            } else {
                0
            },
            mappings: if reaches(first_shape!(SeamAnswered, mappings)) {
                // SAFETY: as above; the mappings run beside the addresses.
                unsafe { lent(answered.mappings, answered.address_count) }.to_vec()
            } else {
                Vec::new()
            },
            bytes: if reaches(first_shape!(SeamAnswered, byte_count)) {
                // SAFETY: as above.
                unsafe { lent(answered.bytes, answered.byte_count) }.to_vec()
            } else {
                Vec::new()
            },
        }),
        ABSENT => Err(Answer::Absent),
        REFUSED => Err(Answer::Refused {
            status: match status {
                1 => RefusalStatus::Malformed,
                2 => RefusalStatus::OutOfMemory,
                _ => RefusalStatus::Refused,
            },
            refusal,
        }),
        DESTROYED => Err(Answer::Destroyed),
        MISTAKE => Err(Answer::Mistake(detail)),
        DETACHED => Err(Answer::Detached),
        CONTRACT_MISMATCH => {
            let mut named = texts().into_iter();
            Err(Answer::ContractMismatch {
                capability: named.next().unwrap_or_default(),
                operation: named.next().unwrap_or_default(),
            })
        }
        _ => Err(Answer::Defect(detail)),
    }
}

/// A manifest in the lent form, with every array and text it points into.
pub(super) struct LentManifest<'a> {
    manifest: &'a Manifest,
    targets: Vec<SeamText>,
    capabilities_provided: Vec<SeamText>,
    capabilities_consumed: Vec<SeamText>,
    permissions: Vec<SeamText>,
    attributes: Vec<Vec<SeamNumber>>,
    constraints: Vec<Vec<SeamNumber>>,
    offers: Vec<SeamOffer>,
    wants: Vec<SeamWant>,
}

fn flag(value: Option<bool>) -> u32 {
    match value {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    }
}

fn flag_of(value: u32) -> Option<bool> {
    match value {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

impl<'a> LentManifest<'a> {
    pub(super) fn of(manifest: &'a Manifest) -> LentManifest<'a> {
        let mut lent = LentManifest {
            manifest,
            targets: lend_texts(&manifest.targets),
            capabilities_provided: lend_texts(&manifest.capabilities_provided),
            capabilities_consumed: lend_texts(&manifest.capabilities_consumed),
            permissions: lend_texts(&manifest.permissions),
            attributes: manifest
                .services_provided
                .iter()
                .map(|offer| lend_numbers(&offer.attributes))
                .collect(),
            constraints: manifest
                .services_consumed
                .iter()
                .map(|want| lend_numbers(&want.constraints))
                .collect(),
            offers: Vec::new(),
            wants: Vec::new(),
        };
        lent.offers = manifest
            .services_provided
            .iter()
            .zip(&lent.attributes)
            .map(|(offer, attributes)| SeamOffer {
                id: SeamText::of(&offer.id),
                attributes: attributes.as_ptr(),
                attribute_count: attributes.len(),
            })
            .collect();
        lent.wants = manifest
            .services_consumed
            .iter()
            .zip(&lent.constraints)
            .map(|(want, constraints)| SeamWant {
                id: SeamText::of(&want.id),
                has_provider: u32::from(want.provider.is_some()),
                optional: flag(want.optional),
                degradable: flag(want.degradable),
                provider: SeamText::of(want.provider.as_deref().unwrap_or("")),
                constraints: constraints.as_ptr(),
                constraint_count: constraints.len(),
            })
            .collect();
        lent
    }

    pub(super) fn table(&self) -> SeamManifest {
        let texts = |texts: &Vec<SeamText>| SeamTexts {
            texts: texts.as_ptr(),
            count: texts.len(),
        };
        SeamManifest {
            size: size_of_table::<SeamManifest>(),
            api: self.manifest.api,
            has_entry: u32::from(self.manifest.entry.is_some()),
            has_identity: u32::from(self.manifest.identity.is_some()),
            import_name: SeamText::of(&self.manifest.import_name),
            entry: SeamText::of(self.manifest.entry.as_deref().unwrap_or("")),
            identity: SeamText::of(self.manifest.identity.as_deref().unwrap_or("")),
            targets: texts(&self.targets),
            capabilities_provided: texts(&self.capabilities_provided),
            capabilities_consumed: texts(&self.capabilities_consumed),
            permissions: texts(&self.permissions),
            services_provided: self.offers.as_ptr(),
            services_provided_count: self.offers.len(),
            services_consumed: self.wants.as_ptr(),
            services_consumed_count: self.wants.len(),
        }
    }
}

/// Reads a lent manifest, refusing one below its first shape.
///
/// # Safety
/// The manifest and everything it lends live for the length of the call.
pub(super) unsafe fn read_manifest(manifest: &SeamManifest) -> Option<Manifest> {
    if !carries(manifest.size, MANIFEST_FIRST_SHAPE) {
        return None;
    }
    // SAFETY: as the caller promised.
    unsafe {
        Some(Manifest {
            import_name: manifest.import_name.read(),
            api: manifest.api,
            entry: (manifest.has_entry != 0).then(|| manifest.entry.read()),
            targets: read_texts(manifest.targets),
            capabilities_provided: read_texts(manifest.capabilities_provided),
            capabilities_consumed: read_texts(manifest.capabilities_consumed),
            services_provided: lent(manifest.services_provided, manifest.services_provided_count)
                .iter()
                .map(|offer| ServiceOffer {
                    id: offer.id.read(),
                    attributes: read_numbers(offer.attributes, offer.attribute_count),
                })
                .collect(),
            services_consumed: lent(manifest.services_consumed, manifest.services_consumed_count)
                .iter()
                .map(|want| ServiceWant {
                    id: want.id.read(),
                    provider: (want.has_provider != 0).then(|| want.provider.read()),
                    optional: flag_of(want.optional),
                    degradable: flag_of(want.degradable),
                    constraints: read_numbers(want.constraints, want.constraint_count),
                })
                .collect(),
            permissions: read_texts(manifest.permissions),
            identity: (manifest.has_identity != 0).then(|| manifest.identity.read()),
        })
    }
}

/// The number every kind of failure travels as; a code this build does not know is read as a plugin's callback.
pub(super) fn failure_kind_code(kind: FailureKind) -> u32 {
    match kind {
        FailureKind::Listener => 1,
        FailureKind::Step => 2,
        FailureKind::DeadlineHook => 3,
        FailureKind::Clock => 4,
        FailureKind::EventIncomplete => 5,
        FailureKind::LayerDefect => 6,
        FailureKind::DueAdvance => 7,
        FailureKind::PluginCallback => 8,
        FailureKind::Log => 9,
        FailureKind::ObjectUnknown => 10,
        FailureKind::CapacityUnanswered => 11,
        FailureKind::PluginDeactivate => 12,
        FailureKind::LibraryUnload => 13,
    }
}

pub(super) fn failure_kind_of(code: u32) -> FailureKind {
    match code {
        1 => FailureKind::Listener,
        2 => FailureKind::Step,
        3 => FailureKind::DeadlineHook,
        4 => FailureKind::Clock,
        5 => FailureKind::EventIncomplete,
        6 => FailureKind::LayerDefect,
        7 => FailureKind::DueAdvance,
        9 => FailureKind::Log,
        10 => FailureKind::ObjectUnknown,
        11 => FailureKind::CapacityUnanswered,
        12 => FailureKind::PluginDeactivate,
        13 => FailureKind::LibraryUnload,
        _ => FailureKind::PluginCallback,
    }
}
