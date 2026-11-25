//! An event the core queued, copied into the crate's own values before the core's struct is released.

use super::abi;
use super::codes::{CapabilityChange, CapacityQuestion, LogLevel, ReachKind, WatchChange};
use super::read::Reader;
use super::values::{Handle, Number, Refusal};

/// One event, in the order the core queued it.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// Its place among every event this core's wrapper drained, from 1.
    pub sequence: u64,
    /// Whether a text the core handed out could not be read, and so carries a fixed sentence in its place. Every
    /// handle, code and number of an incomplete event is still exact.
    pub incomplete: bool,
    /// What happened.
    pub payload: EventPayload,
}

/// What an event says happened.
#[derive(Debug, Clone, PartialEq)]
pub enum EventPayload {
    /// A diagnostic.
    Log {
        /// How much it matters.
        level: LogLevel,
        /// What it says.
        text: String,
    },
    /// A refusal, the answer of the call that queued it.
    Refusal(Refusal),
    /// A want settled, with a service or with a refusal, exactly once.
    WantSettled {
        /// The want.
        want: Handle,
        /// The plugin that wanted.
        plugin: Handle,
        /// The service it settled with, when it settled with one.
        implementation: Handle,
        /// Why it was rejected, code `NONE` when it settled with a service.
        refusal: Refusal,
        /// Whether it settled with a provider that met its constraints only in part.
        degraded: bool,
    },
    /// A watched service was registered or unregistered.
    Watch {
        /// The watch.
        watch: Handle,
        /// The plugin that watches.
        plugin: Handle,
        /// The service.
        implementation: Handle,
        /// What changed.
        change: WatchChange,
    },
    /// A service a plugin holds was withdrawn, so the plugin must stop using it.
    Revoked {
        /// The plugin that held it.
        plugin: Handle,
        /// The service.
        implementation: Handle,
    },
    /// A plugin reached for what it may not, reported once per plugin, kind and id.
    ReachRefused {
        /// What it reached for.
        kind: ReachKind,
        /// The plugin.
        plugin: Handle,
        /// The id it reached for.
        id: String,
        /// Why it may not.
        refusal: Refusal,
    },
    /// A constrained want asks a provider's object a question.
    CapacityWanted {
        /// The want.
        want: Handle,
        /// The plugin that wants.
        plugin: Handle,
        /// The provider's registration.
        registration: Handle,
        /// The request the answer names.
        request: u64,
        /// What it asks.
        question: CapacityQuestion,
        /// The consumer's constraints.
        constraints: Vec<Number>,
    },
    /// A watched capability gained or lost a verified implementation.
    Capability {
        /// The watch.
        watch: Handle,
        /// The plugin that watches.
        plugin: Handle,
        /// The implementation.
        implementation: Handle,
        /// The plugin that provides it.
        provider: Handle,
        /// What changed.
        change: CapabilityChange,
    },
    /// The core's earliest deadline moved: `Some` with the deadline in the caller's milliseconds, `None` when nothing
    /// waits on time any more.
    Deadline(Option<i64>),
    /// A service registration and every holding of it were released.
    HoldingsReleased {
        /// The registration's owner.
        owner: Handle,
        /// The registration.
        registration: Handle,
    },
}

impl Event {
    /// The refusal this event carries for the call that queued it: a refusal, or a refusal of reach.
    pub(crate) fn own_refusal(&self) -> Option<&Refusal> {
        match &self.payload {
            EventPayload::Refusal(refusal) | EventPayload::ReachRefused { refusal, .. } => {
                Some(refusal)
            }
            _ => None,
        }
    }
}

/// Copies a drained event, or answers `None` for a kind the crate does not know, which the core asks be ignored.
///
/// # Safety
/// `event` was just filled by `weivel_next_event` and is not yet released.
pub(crate) unsafe fn copy(event: &abi::Event, sequence: u64) -> Option<Event> {
    let mut reader = Reader::default();
    // SAFETY: the union member read is the one the event's kind names, and its texts are alive until the release.
    let payload = unsafe { payload_of(event, &mut reader) }?;
    Some(Event {
        sequence,
        incomplete: reader.incomplete,
        payload,
    })
}

unsafe fn payload_of(event: &abi::Event, reader: &mut Reader) -> Option<EventPayload> {
    let data = &event.data;
    // SAFETY: each arm reads the union member its kind names, as the core's header pairs them.
    unsafe {
        Some(match event.kind {
            abi::EVENT_LOG => EventPayload::Log {
                level: LogLevel(data.log.level),
                text: reader.text(data.log.text),
            },
            abi::EVENT_REFUSAL => {
                let refused = data.refused;
                EventPayload::Refusal(reader.refusal(
                    refused.refusal,
                    refused.subject,
                    refused.has_fix,
                ))
            }
            abi::EVENT_WANT_SETTLED => want_settled(data.want, reader),
            abi::EVENT_WATCH => watch(data.watch),
            abi::EVENT_REVOKED => EventPayload::Revoked {
                plugin: Handle(data.revoked.plugin),
                implementation: Handle(data.revoked.implementation),
            },
            abi::EVENT_REACH_REFUSED => reach_refused(data.reach, reader),
            abi::EVENT_CAPACITY_WANTED => capacity_wanted(data.capacity, reader),
            abi::EVENT_CAPABILITY => capability(data.capability),
            abi::EVENT_DEADLINE => {
                EventPayload::Deadline((data.deadline.set != 0).then_some(data.deadline.at))
            }
            abi::EVENT_HOLDINGS_RELEASED => EventPayload::HoldingsReleased {
                owner: Handle(data.released.owner),
                registration: Handle(data.released.registration),
            },
            _ => return None,
        })
    }
}

unsafe fn want_settled(want: abi::WantEvent, reader: &mut Reader) -> EventPayload {
    EventPayload::WantSettled {
        want: Handle(want.want),
        plugin: Handle(want.plugin),
        implementation: Handle(want.implementation),
        // SAFETY: the want's texts are alive until the event's release.
        refusal: unsafe { reader.refusal(want.refusal, want.subject, want.has_fix) },
        degraded: want.degraded != 0,
    }
}

fn watch(watch: abi::WatchEvent) -> EventPayload {
    EventPayload::Watch {
        watch: Handle(watch.watch),
        plugin: Handle(watch.plugin),
        implementation: Handle(watch.implementation),
        change: WatchChange(watch.change),
    }
}

unsafe fn reach_refused(reach: abi::ReachRefusal, reader: &mut Reader) -> EventPayload {
    EventPayload::ReachRefused {
        kind: ReachKind(reach.kind),
        plugin: Handle(reach.plugin),
        // SAFETY: the reach's texts are alive until the event's release.
        id: unsafe { reader.text(reach.id) },
        refusal: unsafe { reader.refusal(reach.refusal, reach.subject, reach.has_fix) },
    }
}

unsafe fn capacity_wanted(capacity: abi::CapacityEvent, reader: &mut Reader) -> EventPayload {
    EventPayload::CapacityWanted {
        want: Handle(capacity.want),
        plugin: Handle(capacity.plugin),
        registration: Handle(capacity.registration),
        request: capacity.request,
        question: CapacityQuestion(capacity.question),
        // SAFETY: the constraints live in the event's storage until its release.
        constraints: unsafe { reader.numbers(capacity.constraints) },
    }
}

fn capability(capability: abi::CapabilityEvent) -> EventPayload {
    EventPayload::Capability {
        watch: Handle(capability.watch),
        plugin: Handle(capability.plugin),
        implementation: Handle(capability.implementation),
        provider: Handle(capability.provider),
        change: CapabilityChange(capability.change),
    }
}
