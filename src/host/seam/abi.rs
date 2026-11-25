//! The plugin seam's C tables, each led by its `size` as the core's structs are, so a side reads a field only when
//! the other side's table is long enough to carry it, and fields are only ever appended.

use std::ffi::c_void;

use crate::core::{Answer, Refusal, RefusalCode, RefusalStatus};

/// The longest text an answer carries across the seam, in bytes; a longer one is cut at a character boundary.
pub(super) const ANSWER_TEXT: usize = 512;

/// A text the caller lends for the length of one call, UTF-8 and not NUL-terminated.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SeamText {
    pub(super) pointer: *const u8,
    pub(super) length: usize,
}

impl SeamText {
    pub(super) fn of(text: &str) -> SeamText {
        SeamText {
            pointer: text.as_ptr(),
            length: text.len(),
        }
    }

    /// The text lent.
    ///
    /// # Safety
    /// The text is lent for the length of the call that reads it, as every function of the seam lends one.
    pub(super) unsafe fn read(&self) -> String {
        if self.pointer.is_null() {
            return String::new();
        }
        // SAFETY: as the caller promised.
        let bytes = unsafe { std::slice::from_raw_parts(self.pointer, self.length) };
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// What a call across the seam answered: `kind` 0 for success, and otherwise which [`Answer`] it was, with the texts
/// copied into the struct itself so nothing is owned across the call.
#[repr(C)]
pub struct SeamAnswer {
    pub(super) size: u32,
    pub(super) kind: u32,
    pub(super) refusal_status: u32,
    pub(super) has_refusal: u32,
    pub(super) code: u32,
    pub(super) has_fix: u32,
    pub(super) has_subject: u32,
    pub(super) detail_length: u32,
    pub(super) fix_length: u32,
    pub(super) subject_length: u32,
    pub(super) detail: [u8; ANSWER_TEXT],
    pub(super) fix: [u8; ANSWER_TEXT],
    pub(super) subject: [u8; ANSWER_TEXT],
}

/// The host's context table: what a plugin across the seam reaches the host's context of it through. `data` is the
/// host's; every function takes it first and fills the caller's answer.
#[repr(C)]
pub struct SeamContext {
    pub(super) size: u32,
    pub(super) data: *const c_void,
    pub(super) plugin: u64,
    pub(super) import_name: SeamText,
    pub(super) write: unsafe extern "C" fn(*const c_void, u32, SeamText),
    pub(super) report: unsafe extern "C" fn(*const c_void, u32, SeamText),
    pub(super) get: unsafe extern "C" fn(*const c_void, SeamText, *mut usize, *mut SeamAnswer),
    pub(super) register:
        unsafe extern "C" fn(*const c_void, SeamText, usize, u32, *mut u64, *mut SeamAnswer),
    pub(super) unregister: unsafe extern "C" fn(*const c_void, u64, *mut SeamAnswer),
    pub(super) release_holding: unsafe extern "C" fn(*const c_void, u64, *mut u32, *mut SeamAnswer),
    pub(super) provide: unsafe extern "C" fn(*const c_void, SeamText, usize, *mut SeamAnswer),
}

/// What the host hands an activate or a deactivate to finish it with, once: `failed` 0 for success, else 1 with why.
#[repr(C)]
pub struct SeamCompletion {
    pub(super) size: u32,
    pub(super) data: *const c_void,
    pub(super) finish: unsafe extern "C" fn(*const c_void, u32, SeamText),
}

/// The kinds of a [`SeamEvent`].
pub(super) const EVENT_REVOKED: u32 = 1;
pub(super) const EVENT_HOLDINGS_RELEASED: u32 = 2;

/// What the host tells a plugin across the seam of an event about it, lent for the call.
#[repr(C)]
pub struct SeamEvent {
    pub(super) size: u32,
    pub(super) kind: u32,
    pub(super) address: usize,
    pub(super) registration: u64,
    pub(super) awaits_release: u32,
    pub(super) id: SeamText,
}

/// A plugin's entry table, which its library's entry symbol answers: what the host calls to activate it, to
/// deactivate it, and to tell it of an event. `data` is the plugin's; each function takes it first. `activate` and
/// `deactivate` answer 0 once they have taken the completion, which they then finish exactly once; `deactivate`
/// answers 1, taking nothing, for an entry that offers none.
#[repr(C)]
pub struct SeamEntry {
    pub(super) size: u32,
    pub(super) data: *const c_void,
    pub(super) activate:
        unsafe extern "C" fn(*const c_void, *const SeamContext, *const SeamCompletion) -> u32,
    pub(super) deactivate: unsafe extern "C" fn(*const c_void, *const SeamCompletion) -> u32,
    pub(super) hear: unsafe extern "C" fn(*const c_void, *const SeamEvent),
}

pub(super) fn size_of_table<T>() -> u32 {
    size_of::<T>() as u32
}

fn cut(text: &str) -> &str {
    let mut end = text.len().min(ANSWER_TEXT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn copy_into(text: &str, into: &mut [u8; ANSWER_TEXT]) -> u32 {
    let text = cut(text);
    into[..text.len()].copy_from_slice(text.as_bytes());
    text.len() as u32
}

fn copied(from: &[u8; ANSWER_TEXT], length: u32) -> String {
    String::from_utf8_lossy(&from[..(length as usize).min(ANSWER_TEXT)]).into_owned()
}

const ANSWERED: u32 = 0;
const ABSENT: u32 = 1;
const REFUSED: u32 = 2;
const DESTROYED: u32 = 3;
const DEFECT: u32 = 4;
const MISTAKE: u32 = 5;
const DETACHED: u32 = 6;

impl SeamAnswer {
    pub(super) fn empty() -> SeamAnswer {
        SeamAnswer {
            size: size_of_table::<SeamAnswer>(),
            kind: ANSWERED,
            refusal_status: 0,
            has_refusal: 0,
            code: 0,
            has_fix: 0,
            has_subject: 0,
            detail_length: 0,
            fix_length: 0,
            subject_length: 0,
            detail: [0; ANSWER_TEXT],
            fix: [0; ANSWER_TEXT],
            subject: [0; ANSWER_TEXT],
        }
    }

    /// Writes `answer` into the struct, unless its `size` is too short to carry it whole.
    pub(super) fn write(&mut self, answer: &Answer) {
        if (self.size as usize) < size_of::<SeamAnswer>() {
            return;
        }
        let detail = match answer {
            Answer::Absent => {
                self.kind = ABSENT;
                String::new()
            }
            Answer::Refused { status, refusal } => {
                self.kind = REFUSED;
                self.refusal_status = match status {
                    RefusalStatus::Refused => 0,
                    RefusalStatus::Malformed => 1,
                    RefusalStatus::OutOfMemory => 2,
                };
                self.write_refusal(refusal.as_ref())
            }
            Answer::Destroyed => {
                self.kind = DESTROYED;
                String::new()
            }
            Answer::Defect(detail) => {
                self.kind = DEFECT;
                detail.clone()
            }
            Answer::Mistake(detail) => {
                self.kind = MISTAKE;
                detail.clone()
            }
            Answer::Detached => {
                self.kind = DETACHED;
                String::new()
            }
        };
        self.detail_length = copy_into(&detail, &mut self.detail);
    }

    fn write_refusal(&mut self, refusal: Option<&Refusal>) -> String {
        let Some(refusal) = refusal else {
            return String::new();
        };
        self.has_refusal = 1;
        self.code = refusal.code.0;
        self.has_fix = u32::from(refusal.has_fix);
        self.fix_length = copy_into(&refusal.fix, &mut self.fix);
        if let Some(subject) = &refusal.subject {
            self.has_subject = 1;
            self.subject_length = copy_into(subject, &mut self.subject);
        }
        refusal.detail.clone()
    }

    /// What the struct says was answered: `Ok` for success.
    pub(super) fn read(&self) -> Result<(), Answer> {
        let detail = copied(&self.detail, self.detail_length);
        match self.kind {
            ANSWERED => Ok(()),
            ABSENT => Err(Answer::Absent),
            REFUSED => Err(Answer::Refused {
                status: match self.refusal_status {
                    1 => RefusalStatus::Malformed,
                    2 => RefusalStatus::OutOfMemory,
                    _ => RefusalStatus::Refused,
                },
                refusal: (self.has_refusal != 0).then(|| Refusal {
                    code: RefusalCode(self.code),
                    detail,
                    fix: copied(&self.fix, self.fix_length),
                    subject: (self.has_subject != 0)
                        .then(|| copied(&self.subject, self.subject_length)),
                    has_fix: self.has_fix != 0,
                }),
            }),
            DESTROYED => Err(Answer::Destroyed),
            MISTAKE => Err(Answer::Mistake(detail)),
            DETACHED => Err(Answer::Detached),
            _ => Err(Answer::Defect(detail)),
        }
    }
}
