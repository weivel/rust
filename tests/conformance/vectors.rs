//! Each vector's test compares the run's witness with the vector's whole `then`, and holds the sink and the log to
//! what the vector allows.

use std::sync::{Mutex, OnceLock};

use weivel_rust::conformance::{LayerDriver, LayerVector, LayerWitness};
use weivel_rust::core::{FailureKind, LogLevel};

use crate::support;

fn driver() -> &'static LayerDriver {
    static DRIVER: OnceLock<LayerDriver> = OnceLock::new();
    DRIVER.get_or_init(|| {
        LayerDriver::new(support::core_binary(), support::standard_json())
            .with_echo_library(support::shared_library("echo"))
            .with_described_library(
                support::shared_library("weivel_standard_described"),
                support::standard_descriptions(),
            )
    })
}

pub fn vectors() -> Vec<LayerVector> {
    driver()
        .read(&support::layer_vector_directory())
        .expect("the pinned kit's layer vectors read")
}

fn vector(id: &str) -> LayerVector {
    vectors()
        .into_iter()
        .find(|vector| vector.id == id)
        .unwrap_or_else(|| panic!("the pinned kit carries no layer vector {id}"))
}

/// Taken by each `library` and `description` vector for its whole run: a process maps a library once however often
/// it is opened, so two runs at once would each keep the other's binary open and read the other's unloads and
/// releases.
static ONE_LIBRARY_RUN_AT_A_TIME: Mutex<()> = Mutex::new(());

fn witnessed(id: &str) -> LayerWitness {
    let _alone = (id.starts_with("library/") || id.starts_with("description/")).then(|| {
        ONE_LIBRARY_RUN_AT_A_TIME
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    });
    let vector = vector(id);
    let witness = driver()
        .witness(&vector)
        .unwrap_or_else(|failure| panic!("{id} could not be driven: {failure}"));
    assert_eq!(witness.observed, vector.then, "{id}");
    let errors: Vec<_> = witness
        .logs
        .iter()
        .filter(|record| record.level == LogLevel::ERROR)
        .collect();
    assert!(errors.is_empty(), "{id} logged errors: {errors:?}");
    witness
}

fn witnessed_quietly(id: &str) {
    let witness = witnessed(id);
    assert!(
        witness.failures.is_empty(),
        "{id} told the sink: {:?}",
        witness.failures
    );
}

/// The ids of every vector the suite sits, each one test of its own named after it, so the id stands alone in this
/// source as the kit's `layers` check asks.
macro_rules! layer_vectors {
    ($($test:ident => $id:literal,)*) => {
        pub const SAT: &[&str] = &[$($id),*];

        $(
            #[test]
            fn $test() {
                witnessed_quietly($id);
            }
        )*
    };
}

layer_vectors! {
    a_broken_count_description_fails_the_load => "description/a-broken-count-description-fails-the-load",
    a_checked_description_registers_the_providers_own_pointer => "description/a-checked-description-registers-the-providers-own-pointer",
    a_definition_declared_without_its_description_loads_as_before => "description/a-definition-declared-without-its-description-loads-as-before",
    a_definition_described_without_its_nested_description_fails_the_load => "description/a-definition-described-without-its-nested-description-fails-the-load",
    a_description_answering_another_definition_fails_the_load => "description/a-description-answering-another-definition-fails-the-load",
    a_library_publishing_only_contracts_loads_and_registers_nothing => "description/a-library-publishing-only-contracts-loads-and-registers-nothing",
    a_blocking_operation_answers_as_any_does_and_is_reported_blocking => "description/binding/a-blocking-operation-answers-as-any-does-and-is-reported-blocking",
    a_broken_offset_description_fails_the_load => "description/binding/a-broken-offset-description-fails-the-load",
    a_handle_the_plugin_never_released_is_released_at_the_unload => "description/binding/a-handle-the-plugin-never-released-is-released-at-the-unload",
    a_missing_required_input_answers_malformed => "description/binding/a-missing-required-input-answers-malformed",
    a_nested_definitions_answers_are_released_exactly_once => "description/binding/a-nested-definitions-answers-are-released-exactly-once",
    a_none_answer_arrives_as_none => "description/binding/a-none-answer-arrives-as-none",
    a_refusal_arrives_with_its_detail_and_fix => "description/binding/a-refusal-arrives-with-its-detail-and-fix",
    an_answer_carrying_a_definition_is_callable_until_released => "description/binding/an-answer-carrying-a-definition-is-callable-until-released",
    an_input_the_operation_does_not_take_is_the_callers_mistake => "description/binding/an-input-the-operation-does-not-take-is-the-callers-mistake",
    an_operation_its_provider_does_not_offer_is_the_callers_mistake => "description/binding/an-operation-its-provider-does-not-offer-is-the-callers-mistake",
    every_answer_is_released_exactly_once => "description/binding/every-answer-is-released-exactly-once",
    find_answers_a_text_the_provider_keeps => "description/binding/find-answers-a-text-the-provider-keeps",
    grow_takes_and_answers_a_size => "description/binding/grow-takes-and-answers-a-size",
    halve_takes_and_answers_a_double => "description/binding/halve-takes-and-answers-a-double",
    invert_takes_and_answers_a_32_bit_flag_word => "description/binding/invert-takes-and-answers-a-32-bit-flag-word",
    measure_takes_bytes_and_their_length => "description/binding/measure-takes-bytes-and-their-length",
    negate_takes_and_answers_a_signed_64_bit_integer => "description/binding/negate-takes-and-answers-a-signed-64-bit-integer",
    open_answers_a_handle_and_use_takes_it => "description/binding/open-answers-a-handle-and-use-takes-it",
    outline_copies_nodes_holding_nodes_both_ways => "description/binding/outline-copies-nodes-holding-nodes-both-ways",
    reflect_answers_every_member_kind => "description/binding/reflect-answers-every-member-kind",
    swap_shade_takes_and_answers_an_enum_value => "description/binding/swap-shade-takes-and-answers-an-enum-value",
    tree_answers_a_value_tree => "description/binding/tree-answers-a-value-tree",
    visit_takes_a_handle_inside_a_struct => "description/binding/visit-takes-a-handle-inside-a-struct",
    weigh_takes_a_value_by_pointer => "description/binding/weigh-takes-a-value-by-pointer",
    a_refused_verify_quarantines_the_plugin => "consume/a-refused-verify-quarantines-the-plugin",
    a_release_deactivates_then_releases_then_forgets => "lifecycle/a-release-deactivates-then-releases-then-forgets",
    an_activate_refused_at_once_is_quarantined_before_the_next_starts => "lifecycle/an-activate-refused-at-once-is-quarantined-before-the-next-starts",
    an_activate_that_settles_at_once_is_verified_before_the_next_starts => "lifecycle/an-activate-that-settles-at-once-is-verified-before-the-next-starts",
    an_awaited_want_does_not_hold_back_a_later_plugin => "lifecycle/an-awaited-want-does-not-hold-back-a-later-plugin",
    destroy_releases_a_plugin_whose_activate_is_in_flight => "lifecycle/destroy-releases-a-plugin-whose-activate-is-in-flight",
    events_of_an_activate_wait_for_every_activate_of_the_plan => "lifecycle/events-of-an-activate-wait-for-every-activate-of-the-plan",
    a_binary_closes_once_every_definition_service_is_released => "library/a-binary-closes-once-every-definition-service-is-released",
    a_binary_stays_open_while_a_holding_lives => "library/a-binary-stays-open-while-a-holding-lives",
    a_binary_with_no_holding_closes_at_the_unload => "library/a-binary-with-no-holding-closes-at-the-unload",
    a_declaration_of_neither_a_service_nor_a_contract_fails_the_load => "library/a-declaration-of-neither-a-service-nor-a-contract-fails-the-load",
    a_definition_at_its_size_loads => "library/a-definition-at-its-size-loads",
    a_definition_below_its_size_fails_the_load => "library/a-definition-below-its-size-fails-the-load",
    a_definition_is_the_providers_own_pointer => "library/a-definition-is-the-providers-own-pointer",
    a_forgotten_holder_ends_its_late_holding => "library/a-forgotten-holder-ends-its-late-holding",
    a_held_bytes_answer_keeps_the_binary_open_past_the_last_holding => "library/a-held-bytes-answer-keeps-the-binary-open-past-the-last-holding",
    a_holding_ended_with_its_holder_closes_the_binary => "library/a-holding-ended-with-its-holder-closes-the-binary",
    a_null_definition_fails_the_load => "library/a-null-definition-fails-the-load",
    a_release_event_after_its_owner_was_quarantined_reaches_no_one => "library/a-release-event-after-its-owner-was-quarantined-reaches-no-one",
    an_owner_hears_the_release_its_own_release_queued => "library/an-owner-hears-the-release-its-own-release-queued",
    an_unload_revokes_every_holding_of_a_definition => "library/an-unload-revokes-every-holding-of-a-definition",
    a_registration_at_the_deadline_loses => "want/a-registration-at-the-deadline-loses",
    a_registration_past_an_unfired_deadline_loses => "want/a-registration-past-an-unfired-deadline-loses",
    a_settled_want_disarms_the_timer => "want/a-settled-want-disarms-the-timer",
    a_timed_want_arms_the_timer_at_its_deadline => "want/a-timed-want-arms-the-timer-at-its-deadline",
}

/// Its one listener panics on every call, which is a plugin callback's failure told to the sink, and nothing else.
#[test]
fn a_throwing_listener_does_not_silence_the_others() {
    let id = "consume/a-throwing-listener-does-not-silence-the-others";
    let witness = witnessed(id);
    let told: Vec<(FailureKind, bool)> = witness
        .failures
        .iter()
        .map(|failure| {
            (
                failure.kind,
                failure
                    .text
                    .contains("the throwing listener of screens was handed calendar provide"),
            )
        })
        .collect();
    assert_eq!(told, vec![(FailureKind::PluginCallback, true)]);
}

pub const SAT_WITH_ITS_OWN_TEST: &[&str] =
    &["consume/a-throwing-listener-does-not-silence-the-others"];
