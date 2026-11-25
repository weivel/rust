//! What the crate declares by hand of the core's header, described so a test can hold it to the header and to the C
//! compiler: every struct's layout, read from the declarations themselves, and every function's name.

use std::mem::offset_of;

use super::abi;
use super::table::FUNCTION_NAMES;

/// Where one field of a hand-declared struct lies, as this compiler lays it out. A member of a union field is named
/// after the union's field, as `data.want`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldLayout {
    /// The field's name, as the C header names it.
    pub name: &'static str,
    /// Its offset in bytes.
    pub offset: usize,
    /// Its size in bytes.
    pub size: usize,
}

/// One hand-declared struct's fields, in declaration order, and its size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructLayout {
    /// The struct's name, as the C header names it.
    pub name: &'static str,
    /// Its size in bytes.
    pub size: usize,
    /// Its fields.
    pub fields: Vec<FieldLayout>,
}

/// The size of what a field projection points at, the projection never being called.
const fn pointee_size<Struct, Field>(_projection: fn(*const Struct) -> *const Field) -> usize {
    std::mem::size_of::<Field>()
}

macro_rules! field {
    ($type:ty, $($field:ident).+) => {
        FieldLayout {
            name: stringify!($($field).+),
            offset: offset_of!($type, $($field).+),
            // SAFETY: the projection only names a field's place and is never called.
            size: pointee_size(|value: *const $type| unsafe { &raw const (*value).$($field).+ }),
        }
    };
}

macro_rules! layout {
    ($type:ty as $name:literal { $($($field:ident).+),* $(,)? }) => {
        StructLayout { name: $name, size: std::mem::size_of::<$type>(), fields: vec![$(field!($type, $($field).+)),*] }
    };
}

/// Every struct of the core's header, as the crate declares it.
pub fn core_struct_layouts() -> Vec<StructLayout> {
    let mut layouts = event_layouts();
    layouts.extend(declaration_layouts());
    layouts.extend(answer_layouts());
    layouts
}

fn event_layouts() -> Vec<StructLayout> {
    vec![
        layout!(abi::Refusal as "weivel_refusal" { code, detail, fix }),
        layout!(abi::LogEvent as "weivel_log_event" { level, text }),
        layout!(abi::WantEvent as "weivel_want_event" { want, plugin, implementation, refusal, degraded, subject, has_fix }),
        layout!(abi::WatchEvent as "weivel_watch_event" { watch, plugin, implementation, change }),
        layout!(abi::RevokedEvent as "weivel_revoked_event" { plugin, implementation }),
        layout!(abi::HoldingsReleasedEvent as "weivel_holdings_released_event" { owner, registration }),
        layout!(abi::ReachRefusal as "weivel_reach_refusal" { kind, plugin, id, refusal, subject, has_fix }),
        layout!(abi::RefusalEvent as "weivel_refusal_event" { refusal, subject, has_fix }),
        layout!(abi::Number as "weivel_number" { size, name, value }),
        layout!(abi::Numbers as "weivel_numbers" { items, count }),
        layout!(abi::CapacityEvent as "weivel_capacity_event" { want, plugin, registration, request, question, constraints }),
        layout!(abi::CapabilityEvent as "weivel_capability_event" { watch, plugin, implementation, provider, change }),
        layout!(abi::DeadlineEvent as "weivel_deadline_event" { at, set }),
        layout!(abi::Event as "weivel_event" {
            size, kind, storage, data, data.log, data.refusal, data.want, data.watch, data.revoked, data.reach,
            data.capacity, data.capability, data.refused, data.deadline, data.released, data.reserved,
        }),
    ]
}

fn declaration_layouts() -> Vec<StructLayout> {
    vec![
        layout!(abi::Strings as "weivel_strings" { items, count }),
        layout!(abi::ServiceOffer as "weivel_service_offer" { size, id, attributes }),
        layout!(abi::ServiceOffers as "weivel_service_offers" { items, count }),
        layout!(abi::ServiceWant as "weivel_service_want" { size, id, provider, optional, degradable, constraints }),
        layout!(abi::ServiceWants as "weivel_service_wants" { items, count }),
        layout!(abi::CapabilityDeclarations as "weivel_capability_declarations" { provides, consumes }),
        layout!(abi::ServiceDeclarations as "weivel_service_declarations" { provides, consumes }),
        layout!(abi::Manifest as "weivel_manifest" {
            size, import_name, api, entry, targets, capabilities, services, permissions, identity,
        }),
        layout!(abi::ConstraintEntry as "weivel_constraint_entry" { size, constraint_name, attribute_name, bound }),
        layout!(abi::ConstraintVocabulary as "weivel_constraint_vocabulary" { items, count }),
        layout!(abi::CoreOptions as "weivel_core_options" {
            size, initial_event_capacity, minimum_log_level, api, applications, well_known_services,
            constraint_vocabulary,
        }),
        layout!(abi::PluginOptions as "weivel_plugin_options" { size, payload }),
        layout!(abi::QuarantineReason as "weivel_quarantine_reason" { size, code, detail, fix }),
        layout!(abi::WantOptions as "weivel_want_options" {
            size, timed, now, timeout_millis, budgeted, budget_millis, poll_millis,
        }),
        layout!(abi::Capacity as "weivel_capacity" { size, request, attributes, serves }),
    ]
}

fn answer_layouts() -> Vec<StructLayout> {
    vec![
        layout!(abi::Support as "weivel_support" { size, supported, axis, refusal, storage, subject, has_fix }),
        layout!(abi::Edge as "weivel_edge" { provider, consumer }),
        layout!(abi::Cycle as "weivel_cycle" { members, count }),
        layout!(abi::PluginRefusal as "weivel_plugin_refusal" { plugin, refusal }),
        layout!(abi::RefusalSubject as "weivel_refusal_subject" { subject, has_fix }),
        layout!(abi::ActivationPlan as "weivel_activation_plan" {
            size, order, order_count, edges, edge_count, cycles, cycle_count, refusals, refusal_count, storage,
            refusal_subjects,
        }),
        layout!(abi::ServiceList as "weivel_service_list" { size, ids, storage }),
        layout!(abi::CapabilityImplementation as "weivel_capability_implementation" { size, implementation, provider }),
        layout!(abi::CapabilityList as "weivel_capability_list" { size, items, count, storage }),
        layout!(abi::LedgerRow as "weivel_ledger_row" {
            size, plugin, import_name, identity, status, verified, refusal, capabilities_declared, permissions,
            services_provided, capabilities_provided, services_consumed, storage, capabilities_consumed, subject,
            has_fix,
        }),
        layout!(abi::DeclarationRefusal as "weivel_declaration_refusal" { size, kind, plugin, id, code, subject, has_fix }),
        layout!(abi::LedgerSnapshot as "weivel_ledger_snapshot" { size, rows, row_count, refusals, refusal_count, storage }),
    ]
}

/// The name of every function of the core the crate resolves, in the order its table holds them.
pub fn core_function_names() -> &'static [&'static str] {
    FUNCTION_NAMES
}
