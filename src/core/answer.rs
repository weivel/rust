//! What a call answers when it does not do what it was asked.

use std::fmt;

use super::values::Refusal;

/// How the core declined a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalStatus {
    /// The core refused it.
    Refused,
    /// The core read something the caller built as malformed.
    Malformed,
    /// The core had no memory for it, and changed nothing.
    OutOfMemory,
}

/// Why a call answered no result. A refusal is the core's own, never one the crate invents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The core had nothing to answer: a get found nothing registered, or no deadline is set. Not a refusal.
    Absent,
    /// The core declined the call, with the refusal it queued for it, or `None` where it queued none.
    Refused {
        /// How it declined.
        status: RefusalStatus,
        /// The refusal read from the call's own drained events.
        refusal: Option<Refusal>,
    },
    /// The core was destroyed, so no call was made.
    Destroyed,
    /// A defect of the crate: the core called a call the crate built whole malformed. Also told to the error sink.
    Defect(String),
    /// The caller's mistake, found before anything reached the core, such as a text holding a NUL character.
    Mistake(String),
    /// The call was made through a context whose plugin the core has forgotten, so it never reached the core.
    Detached,
}

impl fmt::Display for Answer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Answer::Absent => formatter.write_str("the core had nothing to answer"),
            Answer::Refused {
                status,
                refusal: Some(refusal),
            } => {
                write!(
                    formatter,
                    "the core declined the call ({status:?}): {}",
                    refusal.detail
                )
            }
            Answer::Refused {
                status,
                refusal: None,
            } => {
                write!(
                    formatter,
                    "the core declined the call ({status:?}) and queued no refusal"
                )
            }
            Answer::Destroyed => formatter.write_str("the core was destroyed"),
            Answer::Defect(detail) => write!(formatter, "a defect of the layer: {detail}"),
            Answer::Mistake(detail) => write!(formatter, "the caller's mistake: {detail}"),
            Answer::Detached => formatter.write_str("the context's plugin is forgotten"),
        }
    }
}

impl std::error::Error for Answer {}
