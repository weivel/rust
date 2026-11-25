//! Loads the core's binary and owns the one wrapper every call into it goes through.
//!
//! [`CoreBinary::load`] opens the core's shared binary from the path its caller gives and resolves every function the
//! core exports; [`Core::create`] makes a core through it. Every call is made under one lock and its events drained
//! under the same hold, each copied into an [`Event`] and released in the core before the next is drained. The events
//! are dispatched to the listeners first in, first out, outside the lock, by one thread at a time; a call made while a
//! dispatch runs joins the end of the queue. A call answers its result, or an [`Answer`] saying why not, and what a
//! listener, a step or a hook panics with goes to the [`ErrorSink`], never to a caller.

mod abi;
mod answer;
pub(crate) mod binary;
mod calls;
mod codes;
#[doc(hidden)]
pub mod declarations;
mod event;
mod marshal;
mod read;
mod table;
mod values;
mod wrapper;
#[cfg(test)]
mod wrapper_tests;

pub use answer::{Answer, RefusalStatus};
pub use codes::{
    Axis, Bound, CapabilityChange, CapacityQuestion, EdgeSource, HandleCondition, LogLevel,
    PluginStatus, ReachKind, RefusalCode, WatchChange,
};
pub use event::{Event, EventPayload};
pub use table::{CoreBinary, LoadFailure};
pub use values::{
    ActivationPlan, CapabilityImplementation, Capacity, ConstraintEntry, CoreOptions,
    DeclarationRefusal, Edge, Handle, LedgerRow, LedgerSnapshot, Manifest, Number, PluginRefusal,
    QuarantineReason, Refusal, ServiceOffer, ServiceWant, Support, WantOptions,
};
pub use wrapper::{
    Core, ErrorSink, Failure, FailureKind, Listener, ListenerRegistration, Wrapping,
};
pub(crate) use wrapper::{DeadlineHooks, Hold, HooksRegistration, panic_text};
