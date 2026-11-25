//! Each vector's test compares the run's witness with the vector's whole `then`, and holds the sink and the log to
//! what the vector allows.

use std::sync::OnceLock;

use weivel_rust::conformance::{LayerDriver, LayerVector, LayerWitness};
use weivel_rust::core::{FailureKind, LogLevel};

use crate::support;

fn driver() -> &'static LayerDriver {
    static DRIVER: OnceLock<LayerDriver> = OnceLock::new();
    DRIVER.get_or_init(|| LayerDriver::new(support::core_binary(), support::standard_json()))
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

fn witnessed(id: &str) -> LayerWitness {
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
    a_refused_verify_quarantines_the_plugin => "consume/a-refused-verify-quarantines-the-plugin",
    a_release_deactivates_then_releases_then_forgets => "lifecycle/a-release-deactivates-then-releases-then-forgets",
    an_activate_refused_at_once_is_quarantined_before_the_next_starts => "lifecycle/an-activate-refused-at-once-is-quarantined-before-the-next-starts",
    an_activate_that_settles_at_once_is_verified_before_the_next_starts => "lifecycle/an-activate-that-settles-at-once-is-verified-before-the-next-starts",
    an_awaited_want_does_not_hold_back_a_later_plugin => "lifecycle/an-awaited-want-does-not-hold-back-a-later-plugin",
    destroy_releases_a_plugin_whose_activate_is_in_flight => "lifecycle/destroy-releases-a-plugin-whose-activate-is-in-flight",
    events_of_an_activate_wait_for_every_activate_of_the_plan => "lifecycle/events-of-an-activate-wait-for-every-activate-of-the-plan",
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
