//! The values a call of the core takes and answers, as the crate's own: owned texts and lists, nothing borrowed from
//! the core's memory.

use super::codes::{Axis, Bound, LogLevel, PluginStatus, ReachKind, RefusalCode};

/// Anything the core tracks: an integer it issued, valid only for that core, compared only for equality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handle(pub u64);

impl Handle {
    /// The host itself, as the owner of a host service.
    pub const HOST: Handle = Handle(u64::MAX);
}

/// Why the core refused something, with what it is about, as the core answered it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Refusal {
    /// What kind of refusal it is, and so what `subject` names.
    pub code: RefusalCode,
    /// What happened, in English.
    pub detail: String,
    /// What would make it succeed, in English; empty when the core has none.
    pub fix: String,
    /// What the refusal is about, as its code says; `None` for a refusal of code `NONE`.
    pub subject: Option<String>,
    /// Whether `fix` is non-empty, as the core answered it.
    pub has_fix: bool,
}

/// A named number: an attribute a provider offers or a constraint a consumer asks.
#[derive(Debug, Clone, PartialEq)]
pub struct Number {
    /// The attribute's or constraint's name.
    pub name: String,
    /// Its value.
    pub value: f64,
}

/// One entry of the core's constraint vocabulary: which constraint compares against which attribute, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintEntry {
    /// The constraint's name, as a consumer asks it.
    pub constraint_name: String,
    /// The attribute's name, as a provider offers it.
    pub attribute_name: String,
    /// How the two compare.
    pub bound: Bound,
}

/// What a core is created with; every field at its default asks for the core's own default.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CoreOptions {
    /// How many events the queue holds before it first grows; 0 for the core's default.
    pub initial_event_capacity: u32,
    /// The least level of log event the core queues.
    pub minimum_log_level: LogLevel,
    /// The highest api level the host implements; 0 for the core's own.
    pub api: u32,
    /// The applications the host hosts; empty for every application.
    pub applications: Vec<String>,
    /// The service ids any plugin may register.
    pub well_known_services: Vec<String>,
    /// Which constraint compares against which attribute.
    pub constraint_vocabulary: Vec<ConstraintEntry>,
}

/// A service a plugin offers, with the attributes it offers it with.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ServiceOffer {
    /// The service's id.
    pub id: String,
    /// What it offers.
    pub attributes: Vec<Number>,
}

/// A service a plugin wants.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ServiceWant {
    /// The service's id.
    pub id: String,
    /// The one provider it accepts, or `None` for any.
    pub provider: Option<String>,
    /// Whether the plugin still starts when nothing provides it; `None` for the core's default.
    pub optional: Option<bool>,
    /// Whether the plugin accepts a provider that meets its constraints only in part; `None` for the core's default.
    pub degradable: Option<bool>,
    /// What it asks of a provider.
    pub constraints: Vec<Number>,
}

/// A plugin's manifest, as the core reads it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Manifest {
    /// The plugin's import name.
    pub import_name: String,
    /// The api level it was written against.
    pub api: u32,
    /// Where its code is found, as its language reads it; `None` for none.
    pub entry: Option<String>,
    /// The applications it targets.
    pub targets: Vec<String>,
    /// The capabilities it provides.
    pub capabilities_provided: Vec<String>,
    /// The capabilities it consumes.
    pub capabilities_consumed: Vec<String>,
    /// The services it provides.
    pub services_provided: Vec<ServiceOffer>,
    /// The services it consumes.
    pub services_consumed: Vec<ServiceWant>,
    /// The permissions it asks.
    pub permissions: Vec<String>,
    /// Its identity; `None` for none.
    pub identity: Option<String>,
}

/// Whether a core supports a manifest, and if not, why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Support {
    /// Whether it does.
    pub supported: bool,
    /// The axis it refused on.
    pub axis: Axis,
    /// Why not; code `NONE` when supported.
    pub refusal: Refusal,
}

/// A provider an activation plan orders before its consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    /// The plugin that provides.
    pub provider: Handle,
    /// The plugin that consumes.
    pub consumer: Handle,
}

/// One plugin an activation plan refused, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRefusal {
    /// The plugin.
    pub plugin: Handle,
    /// Why.
    pub refusal: Refusal,
}

/// The order a set of registered plugins activates in, with cycles and refusals as data.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ActivationPlan {
    /// Every plugin that can activate, each provider before its consumers.
    pub order: Vec<Handle>,
    /// Each provider to consumer edge the order honours.
    pub edges: Vec<Edge>,
    /// Each cycle found, its members in order.
    pub cycles: Vec<Vec<Handle>>,
    /// Each plugin refused.
    pub refusals: Vec<PluginRefusal>,
}

/// How a want waits: for ever, until a deadline, and for a constrained want how long and how often it polls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WantOptions {
    /// Whether the want has a deadline; `None` for the core's default.
    pub timed: Option<bool>,
    /// The caller's time now, in its own milliseconds.
    pub now: i64,
    /// How long after `now` a timed want gives up.
    pub timeout_millis: i64,
    /// Whether a constrained want has a budget; `None` for the core's default.
    pub budgeted: Option<bool>,
    /// How long a constrained want polls before it degrades or is refused.
    pub budget_millis: i64,
    /// How often a constrained want polls.
    pub poll_millis: i64,
}

/// A provider's answer to a capacity question.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Capacity {
    /// The question's request, as the event asked it.
    pub request: u64,
    /// Its live attributes.
    pub attributes: Vec<Number>,
    /// Whether it serves the consumer.
    pub serves: bool,
}

/// Why a plugin is quarantined, as the host knows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QuarantineReason {
    /// The refusal code its ledger row shows.
    pub code: RefusalCode,
    /// What happened.
    pub detail: String,
    /// What would fix it.
    pub fix: String,
}

/// One verified implementation of a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityImplementation {
    /// The implementation's handle.
    pub implementation: Handle,
    /// The plugin that provides it.
    pub provider: Handle,
}

/// One plugin's row of the core's ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerRow {
    /// The plugin.
    pub plugin: Handle,
    /// Its import name.
    pub import_name: String,
    /// Its identity, empty for none.
    pub identity: String,
    /// Where it stands.
    pub status: PluginStatus,
    /// Whether its activation was verified.
    pub verified: bool,
    /// Why it is quarantined or refused, code `NONE` otherwise.
    pub refusal: Refusal,
    /// The capabilities its manifest declares.
    pub capabilities_declared: Vec<String>,
    /// The permissions it asks.
    pub permissions: Vec<String>,
    /// The services it registered.
    pub services_provided: Vec<String>,
    /// The capabilities it provided.
    pub capabilities_provided: Vec<String>,
    /// The services it consumed.
    pub services_consumed: Vec<String>,
    /// The capabilities it consumed.
    pub capabilities_consumed: Vec<String>,
}

/// A refusal of reach reported for a plugin, as the ledger records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclarationRefusal {
    /// What the plugin reached for.
    pub kind: ReachKind,
    /// The plugin that reached.
    pub plugin: Handle,
    /// The id it reached for.
    pub id: String,
    /// Why it was refused.
    pub code: RefusalCode,
    /// What the refusal is about.
    pub subject: Option<String>,
    /// Whether the core has a fix for it.
    pub has_fix: bool,
}

/// The whole of the core's ledger at one moment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LedgerSnapshot {
    /// One row per plugin.
    pub rows: Vec<LedgerRow>,
    /// Every declaration refused.
    pub refusals: Vec<DeclarationRefusal>,
}
