//! The core's structs as `weivel/core.h` declares them, written by hand. `tests/core/layouts.rs` compares every field's
//! offset and size and every struct's size with what the core's `layouts` program prints, so a field written wrong here
//! fails a test rather than a call.

use std::ffi::{c_char, c_void};

pub(crate) const STATUS_OK: i32 = 0;
pub(crate) const STATUS_NONE: i32 = 1;
pub(crate) const STATUS_MALFORMED: i32 = 2;
pub(crate) const STATUS_REFUSED: i32 = 3;
pub(crate) const STATUS_OUT_OF_MEMORY: i32 = 4;

pub(crate) const EVENT_LOG: u32 = 1;
pub(crate) const EVENT_REFUSAL: u32 = 2;
pub(crate) const EVENT_WANT_SETTLED: u32 = 3;
pub(crate) const EVENT_WATCH: u32 = 4;
pub(crate) const EVENT_REVOKED: u32 = 5;
pub(crate) const EVENT_REACH_REFUSED: u32 = 6;
pub(crate) const EVENT_CAPACITY_WANTED: u32 = 7;
pub(crate) const EVENT_CAPABILITY: u32 = 8;
pub(crate) const EVENT_DEADLINE: u32 = 9;
pub(crate) const EVENT_HOLDINGS_RELEASED: u32 = 10;

pub(crate) const CHOICE_DEFAULT: u32 = 0;
pub(crate) const CHOICE_NO: u32 = 1;
pub(crate) const CHOICE_YES: u32 = 2;

pub(crate) const EVENT_PAYLOAD_WORDS: usize = 16;

/// The `size` a struct of type `T` carries, as this crate was compiled.
pub(crate) const fn size_of<T>() -> u32 {
    std::mem::size_of::<T>() as u32
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Refusal {
    pub(crate) code: u32,
    pub(crate) detail: *const c_char,
    pub(crate) fix: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct LogEvent {
    pub(crate) level: u32,
    pub(crate) text: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct WantEvent {
    pub(crate) want: u64,
    pub(crate) plugin: u64,
    pub(crate) implementation: u64,
    pub(crate) refusal: Refusal,
    pub(crate) degraded: u32,
    pub(crate) subject: *const c_char,
    pub(crate) has_fix: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct WatchEvent {
    pub(crate) watch: u64,
    pub(crate) plugin: u64,
    pub(crate) implementation: u64,
    pub(crate) change: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct RevokedEvent {
    pub(crate) plugin: u64,
    pub(crate) implementation: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct HoldingsReleasedEvent {
    pub(crate) owner: u64,
    pub(crate) registration: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ReachRefusal {
    pub(crate) kind: u32,
    pub(crate) plugin: u64,
    pub(crate) id: *const c_char,
    pub(crate) refusal: Refusal,
    pub(crate) subject: *const c_char,
    pub(crate) has_fix: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct RefusalEvent {
    pub(crate) refusal: Refusal,
    pub(crate) subject: *const c_char,
    pub(crate) has_fix: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Number {
    pub(crate) size: u32,
    pub(crate) name: *const c_char,
    pub(crate) value: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Numbers {
    pub(crate) items: *const Number,
    pub(crate) count: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct CapacityEvent {
    pub(crate) want: u64,
    pub(crate) plugin: u64,
    pub(crate) registration: u64,
    pub(crate) request: u64,
    pub(crate) question: u32,
    pub(crate) constraints: Numbers,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct CapabilityEvent {
    pub(crate) watch: u64,
    pub(crate) plugin: u64,
    pub(crate) implementation: u64,
    pub(crate) provider: u64,
    pub(crate) change: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct DeadlineEvent {
    pub(crate) at: i64,
    pub(crate) set: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) union EventData {
    pub(crate) log: LogEvent,
    pub(crate) refusal: Refusal,
    pub(crate) want: WantEvent,
    pub(crate) watch: WatchEvent,
    pub(crate) revoked: RevokedEvent,
    pub(crate) reach: ReachRefusal,
    pub(crate) capacity: CapacityEvent,
    pub(crate) capability: CapabilityEvent,
    pub(crate) refused: RefusalEvent,
    pub(crate) deadline: DeadlineEvent,
    pub(crate) released: HoldingsReleasedEvent,
    pub(crate) reserved: [u64; EVENT_PAYLOAD_WORDS],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Event {
    pub(crate) size: u32,
    pub(crate) kind: u32,
    pub(crate) storage: *mut c_void,
    pub(crate) data: EventData,
}

impl Event {
    /// An empty event of this crate's size, ready for `weivel_next_event` to fill.
    pub(crate) fn empty() -> Self {
        Event {
            size: size_of::<Event>(),
            kind: 0,
            storage: std::ptr::null_mut(),
            data: EventData {
                reserved: [0; EVENT_PAYLOAD_WORDS],
            },
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Strings {
    pub(crate) items: *const *const c_char,
    pub(crate) count: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ServiceOffer {
    pub(crate) size: u32,
    pub(crate) id: *const c_char,
    pub(crate) attributes: Numbers,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ServiceOffers {
    pub(crate) items: *const ServiceOffer,
    pub(crate) count: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ServiceWant {
    pub(crate) size: u32,
    pub(crate) id: *const c_char,
    pub(crate) provider: *const c_char,
    pub(crate) optional: u32,
    pub(crate) degradable: u32,
    pub(crate) constraints: Numbers,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ServiceWants {
    pub(crate) items: *const ServiceWant,
    pub(crate) count: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct CapabilityDeclarations {
    pub(crate) provides: Strings,
    pub(crate) consumes: Strings,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ServiceDeclarations {
    pub(crate) provides: ServiceOffers,
    pub(crate) consumes: ServiceWants,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Manifest {
    pub(crate) size: u32,
    pub(crate) import_name: *const c_char,
    pub(crate) api: u32,
    pub(crate) entry: *const c_char,
    pub(crate) targets: Strings,
    pub(crate) capabilities: CapabilityDeclarations,
    pub(crate) services: ServiceDeclarations,
    pub(crate) permissions: Strings,
    pub(crate) identity: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ConstraintEntry {
    pub(crate) size: u32,
    pub(crate) constraint_name: *const c_char,
    pub(crate) attribute_name: *const c_char,
    pub(crate) bound: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ConstraintVocabulary {
    pub(crate) items: *const ConstraintEntry,
    pub(crate) count: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct CoreOptions {
    pub(crate) size: u32,
    pub(crate) initial_event_capacity: u32,
    pub(crate) minimum_log_level: u32,
    pub(crate) api: u32,
    pub(crate) applications: Strings,
    pub(crate) well_known_services: Strings,
    pub(crate) constraint_vocabulary: ConstraintVocabulary,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Support {
    pub(crate) size: u32,
    pub(crate) supported: u32,
    pub(crate) axis: u32,
    pub(crate) refusal: Refusal,
    pub(crate) storage: *mut c_void,
    pub(crate) subject: *const c_char,
    pub(crate) has_fix: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Edge {
    pub(crate) provider: u64,
    pub(crate) consumer: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Cycle {
    pub(crate) members: *const u64,
    pub(crate) count: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct PluginRefusal {
    pub(crate) plugin: u64,
    pub(crate) refusal: Refusal,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct RefusalSubject {
    pub(crate) subject: *const c_char,
    pub(crate) has_fix: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ActivationPlan {
    pub(crate) size: u32,
    pub(crate) order: *const u64,
    pub(crate) order_count: u32,
    pub(crate) edges: *const Edge,
    pub(crate) edge_count: u32,
    pub(crate) cycles: *const Cycle,
    pub(crate) cycle_count: u32,
    pub(crate) refusals: *const PluginRefusal,
    pub(crate) refusal_count: u32,
    pub(crate) storage: *mut c_void,
    pub(crate) refusal_subjects: *const RefusalSubject,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct PluginOptions {
    pub(crate) size: u32,
    pub(crate) payload: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct QuarantineReason {
    pub(crate) size: u32,
    pub(crate) code: u32,
    pub(crate) detail: *const c_char,
    pub(crate) fix: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct WantOptions {
    pub(crate) size: u32,
    pub(crate) timed: u32,
    pub(crate) now: i64,
    pub(crate) timeout_millis: i64,
    pub(crate) budgeted: u32,
    pub(crate) budget_millis: i64,
    pub(crate) poll_millis: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Capacity {
    pub(crate) size: u32,
    pub(crate) request: u64,
    pub(crate) attributes: Numbers,
    pub(crate) serves: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ServiceList {
    pub(crate) size: u32,
    pub(crate) ids: Strings,
    pub(crate) storage: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct CapabilityImplementation {
    pub(crate) size: u32,
    pub(crate) implementation: u64,
    pub(crate) provider: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct CapabilityList {
    pub(crate) size: u32,
    pub(crate) items: *const CapabilityImplementation,
    pub(crate) count: u32,
    pub(crate) storage: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct LedgerRow {
    pub(crate) size: u32,
    pub(crate) plugin: u64,
    pub(crate) import_name: *const c_char,
    pub(crate) identity: *const c_char,
    pub(crate) status: u32,
    pub(crate) verified: u32,
    pub(crate) refusal: Refusal,
    pub(crate) capabilities_declared: Strings,
    pub(crate) permissions: Strings,
    pub(crate) services_provided: Strings,
    pub(crate) capabilities_provided: Strings,
    pub(crate) services_consumed: Strings,
    pub(crate) storage: *mut c_void,
    pub(crate) capabilities_consumed: Strings,
    pub(crate) subject: *const c_char,
    pub(crate) has_fix: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct DeclarationRefusal {
    pub(crate) size: u32,
    pub(crate) kind: u32,
    pub(crate) plugin: u64,
    pub(crate) id: *const c_char,
    pub(crate) code: u32,
    pub(crate) subject: *const c_char,
    pub(crate) has_fix: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct LedgerSnapshot {
    pub(crate) size: u32,
    pub(crate) rows: *const LedgerRow,
    pub(crate) row_count: u32,
    pub(crate) refusals: *const DeclarationRefusal,
    pub(crate) refusal_count: u32,
    pub(crate) storage: *mut c_void,
}

/// A struct of the core's that begins with its `uint32_t size` and holds only integers and raw pointers, so all zero
/// bytes is a valid value of it.
///
/// # Safety
/// An implementer is such a struct.
pub(crate) unsafe trait SizePrefixed: Copy {}

// SAFETY: each is declared above as a `#[repr(C)]` struct of integers and raw pointers whose first field is `size`.
unsafe impl SizePrefixed for Number {}
unsafe impl SizePrefixed for Support {}
unsafe impl SizePrefixed for ActivationPlan {}
unsafe impl SizePrefixed for ServiceList {}
unsafe impl SizePrefixed for CapabilityImplementation {}
unsafe impl SizePrefixed for CapabilityList {}
unsafe impl SizePrefixed for LedgerRow {}
unsafe impl SizePrefixed for DeclarationRefusal {}
unsafe impl SizePrefixed for LedgerSnapshot {}
unsafe impl SizePrefixed for ServiceOffer {}
unsafe impl SizePrefixed for ServiceWant {}

/// A struct the core fills and frees, zeroed but for its `size`, which the core reads first.
pub(crate) fn answer_struct<T: SizePrefixed>() -> T {
    let mut value = std::mem::MaybeUninit::<T>::zeroed();
    // SAFETY: `T` is size-prefixed, so all zero bytes is a valid value and its first four bytes are its size.
    unsafe {
        value.as_mut_ptr().cast::<u32>().write(size_of::<T>());
        value.assume_init()
    }
}
