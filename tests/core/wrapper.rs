//! The wrapper's rules against the real core: refusals, dispatch order, deferred steps, the sink, destroy, the
//! deadline hooks, and calls from several threads.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use weivel_rust::core::{
    Answer, Core, CoreOptions, DeadlineHooks, EventPayload, FailureKind, Handle, HooksRegistration,
    Manifest, RefusalCode, RefusalStatus, ServiceOffer, ServiceWant, WantOptions, WatchChange,
};

use crate::support::{Heard, recorded_core};

/// A provider of `p:a` and a consumer of it, both registered and opened.
fn two_open_plugins(core: &Core) -> (Handle, Handle) {
    let provider = core
        .register_manifest(&Manifest {
            import_name: "p".to_owned(),
            api: 1,
            services_provided: vec![ServiceOffer {
                id: "p:a".to_owned(),
                attributes: Vec::new(),
            }],
            ..Manifest::default()
        })
        .unwrap();
    let consumer = core
        .register_manifest(&Manifest {
            import_name: "c".to_owned(),
            api: 1,
            services_consumed: vec![ServiceWant {
                id: "p:a".to_owned(),
                ..ServiceWant::default()
            }],
            ..Manifest::default()
        })
        .unwrap();
    core.open_plugin(provider, 1).unwrap();
    core.open_plugin(consumer, 2).unwrap();
    (provider, consumer)
}

fn watch_changes(heard: &Heard) -> Vec<(Handle, WatchChange)> {
    heard
        .payloads()
        .into_iter()
        .filter_map(|payload| match payload {
            EventPayload::Watch { watch, change, .. } => Some((watch, change)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_refused_call_is_answered_with_the_refusal_the_core_queued_and_its_event_is_heard() {
    let (core, heard, told) = recorded_core(&CoreOptions::default());
    let answered = core
        .release_plugin(Handle(0x0000_0007_0000_0009))
        .unwrap_err();
    let Answer::Refused {
        status: RefusalStatus::Refused,
        refusal: Some(refusal),
    } = answered
    else {
        panic!("a stale handle is refused with the core's refusal")
    };
    assert_eq!(refusal.code, RefusalCode::STALE_HANDLE);
    assert!(
        heard.payloads().contains(&EventPayload::Refusal(refusal)),
        "the refusal's event was heard as well"
    );
    assert!(told.all().is_empty());
}

#[test]
fn a_handle_that_names_nothing_is_answered_with_the_status_alone() {
    let (core, _heard, _told) = recorded_core(&CoreOptions::default());
    let answered = core.handle_state(Handle(0x0000_0003_0000_0001));
    assert_eq!(
        answered,
        Err(Answer::Refused {
            status: RefusalStatus::Refused,
            refusal: None
        })
    );
}

#[test]
fn events_are_heard_in_the_order_the_core_queued_them() {
    let (core, heard, _told) = recorded_core(&CoreOptions::default());
    let (provider, consumer) = two_open_plugins(&core);
    let first = core.watch_service(consumer, "p:a").unwrap();
    let second = core.watch_service(consumer, "p:a").unwrap();
    heard.clear();
    core.register_service(provider, "p:a").unwrap();
    assert_eq!(
        watch_changes(&heard),
        vec![
            (first, WatchChange::REGISTERED),
            (second, WatchChange::REGISTERED)
        ]
    );
    let sequences: Vec<u64> = heard.all().iter().map(|event| event.sequence).collect();
    assert!(
        sequences.windows(2).all(|pair| pair[0] < pair[1]),
        "{sequences:?}"
    );
}

#[test]
fn a_call_inside_a_listener_joins_the_end_and_never_re_enters_the_dispatcher() {
    let (core, heard, _told) = recorded_core(&CoreOptions::default());
    let (provider, consumer) = two_open_plugins(&core);
    let first = core.watch_service(consumer, "p:a").unwrap();
    let second = core.watch_service(consumer, "p:a").unwrap();
    let heard_when_the_inner_call_returned = Arc::new(Mutex::new(None::<usize>));
    let (inner_heard, inner_returned) = (heard.clone(), heard_when_the_inner_call_returned.clone());
    core.add_listener(Arc::new(move |core: &Core, event| {
        if let EventPayload::Watch {
            watch,
            implementation,
            change: WatchChange::REGISTERED,
            ..
        } = event.payload
            && watch == first
        {
            core.unregister(implementation).unwrap();
            *inner_returned.lock().unwrap() = Some(watch_changes(&inner_heard).len());
        }
    }))
    .unwrap();
    heard.clear();
    core.register_service(provider, "p:a").unwrap();
    assert_eq!(
        watch_changes(&heard),
        vec![
            (first, WatchChange::REGISTERED),
            (second, WatchChange::REGISTERED),
            (first, WatchChange::UNREGISTERED),
            (second, WatchChange::UNREGISTERED),
        ]
    );
    assert_eq!(
        *heard_when_the_inner_call_returned.lock().unwrap(),
        Some(1),
        "the inner call returned before its events were heard"
    );
}

#[test]
fn a_panicking_listener_goes_to_the_sink_and_the_others_still_hear() {
    let (core, heard, told) = recorded_core(&CoreOptions::default());
    core.add_listener(Arc::new(|_core: &Core, _event| {
        panic!("a listener that fails")
    }))
    .unwrap();
    let after = Heard::default();
    core.add_listener(after.listener()).unwrap();
    let _ = core.release_plugin(Handle(0x0000_0007_0000_0009));
    assert_eq!(heard.all().len(), 1);
    assert_eq!(
        after.all().len(),
        1,
        "a listener after the failing one still heard the event"
    );
    let failures = told.all();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].kind, FailureKind::Listener);
    assert_eq!(failures[0].sequence, Some(heard.all()[0].sequence));
}

#[test]
fn listeners_hear_in_the_order_added_until_removed() {
    let (core, _heard, _told) = recorded_core(&CoreOptions::default());
    let order = Arc::new(Mutex::new(Vec::new()));
    let (first_order, second_order) = (order.clone(), order.clone());
    let first = core
        .add_listener(Arc::new(move |_core: &Core, _event| {
            first_order.lock().unwrap().push("first")
        }))
        .unwrap();
    core.add_listener(Arc::new(move |_core: &Core, _event| {
        second_order.lock().unwrap().push("second")
    }))
    .unwrap();
    let _ = core.release_plugin(Handle(0x0000_0007_0000_0009));
    core.remove_listener(first).unwrap();
    let _ = core.release_plugin(Handle(0x0000_0007_0000_0009));
    assert_eq!(*order.lock().unwrap(), vec!["first", "second", "second"]);
    assert!(matches!(
        core.remove_listener(first),
        Err(Answer::Mistake(_))
    ));
}

#[test]
fn a_step_deferred_while_no_dispatch_runs_has_run_when_the_deferral_returns() {
    let (core, _heard, _told) = recorded_core(&CoreOptions::default());
    let ran = Arc::new(Mutex::new(false));
    let inner = ran.clone();
    core.defer(move |_core| *inner.lock().unwrap() = true)
        .unwrap();
    assert!(*ran.lock().unwrap());
}

#[test]
fn a_deferred_step_runs_behind_what_was_already_drained() {
    let (core, heard, _told) = recorded_core(&CoreOptions::default());
    let (provider, consumer) = two_open_plugins(&core);
    let first = core.watch_service(consumer, "p:a").unwrap();
    core.watch_service(consumer, "p:a").unwrap();
    let heard_when_the_step_ran = Arc::new(Mutex::new(None::<usize>));
    let (inner_heard, inner_ran) = (heard.clone(), heard_when_the_step_ran.clone());
    core.add_listener(Arc::new(move |core: &Core, event| {
        if matches!(event.payload, EventPayload::Watch { watch, .. } if watch == first) {
            let (step_heard, step_ran) = (inner_heard.clone(), inner_ran.clone());
            core.defer(move |_core| {
                *step_ran.lock().unwrap() = Some(watch_changes(&step_heard).len())
            })
            .unwrap();
        }
    }))
    .unwrap();
    heard.clear();
    core.register_service(provider, "p:a").unwrap();
    assert_eq!(
        *heard_when_the_step_ran.lock().unwrap(),
        Some(2),
        "the step ran after both watch events"
    );
}

#[test]
fn a_panicking_step_goes_to_the_sink() {
    let (core, _heard, told) = recorded_core(&CoreOptions::default());
    core.defer(|_core| panic!("a step that fails")).unwrap();
    assert_eq!(
        told.all()
            .iter()
            .map(|failure| failure.kind)
            .collect::<Vec<_>>(),
        vec![FailureKind::Step]
    );
}

#[test]
fn a_destroyed_core_answers_every_call_destroyed() {
    let (core, _heard, _told) = recorded_core(&CoreOptions::default());
    assert!(!core.is_destroyed());
    core.destroy();
    core.destroy();
    assert!(core.is_destroyed());
    assert_eq!(core.ledger(), Err(Answer::Destroyed));
    assert_eq!(core.defer(|_core| {}), Err(Answer::Destroyed));
    assert!(matches!(
        core.add_listener(Heard::default().listener()),
        Err(Answer::Destroyed)
    ));
}

#[test]
fn a_destroy_inside_a_listener_drops_what_was_not_dispatched() {
    let (core, heard, _told) = recorded_core(&CoreOptions::default());
    let (provider, consumer) = two_open_plugins(&core);
    core.watch_service(consumer, "p:a").unwrap();
    core.watch_service(consumer, "p:a").unwrap();
    core.add_listener(Arc::new(|core: &Core, event| {
        if matches!(event.payload, EventPayload::Watch { .. }) {
            core.destroy();
        }
    }))
    .unwrap();
    heard.clear();
    core.register_service(provider, "p:a").unwrap();
    assert_eq!(
        watch_changes(&heard).len(),
        1,
        "the second watch event was dropped by the destroy"
    );
}

/// One deadline `follow` heard, and whether any deadline event had been dispatched by then.
#[derive(Debug, PartialEq)]
struct Followed {
    deadline: Option<i64>,
    deadline_event_heard: bool,
}

/// A clock the test sets, and every deadline `follow` heard.
struct Timing {
    now: Arc<AtomicI64>,
    followed: Arc<Mutex<Vec<Followed>>>,
}

fn install_timing(core: &Core, heard: &Heard) -> (Timing, HooksRegistration) {
    let timing = Timing {
        now: Arc::new(AtomicI64::new(0)),
        followed: Arc::new(Mutex::new(Vec::new())),
    };
    let (clock, followed, heard) = (timing.now.clone(), timing.followed.clone(), heard.clone());
    let registration = core
        .install_deadline_hooks(DeadlineHooks {
            now: Arc::new(move || clock.load(Ordering::SeqCst)),
            follow: Arc::new(move |deadline| {
                let deadline_heard = heard
                    .payloads()
                    .iter()
                    .any(|payload| matches!(payload, EventPayload::Deadline(_)));
                followed.lock().unwrap().push(Followed {
                    deadline,
                    deadline_event_heard: deadline_heard,
                });
            }),
        })
        .unwrap();
    (timing, registration)
}

#[test]
fn a_deadline_is_followed_after_its_drain_and_before_its_dispatch() {
    let (core, heard, _told) = recorded_core(&CoreOptions::default());
    let (_provider, consumer) = two_open_plugins(&core);
    let (timing, _registration) = install_timing(&core, &heard);
    let options = WantOptions {
        timed: Some(true),
        now: 0,
        timeout_millis: 100,
        ..WantOptions::default()
    };
    core.want_service(consumer, "p:a", &options).unwrap();
    assert_eq!(
        *timing.followed.lock().unwrap(),
        vec![Followed {
            deadline: Some(100),
            deadline_event_heard: false
        }]
    );
    assert!(
        heard
            .payloads()
            .contains(&EventPayload::Deadline(Some(100)))
    );
}

#[test]
fn a_due_deadline_is_handed_to_the_core_before_the_call() {
    let (core, heard, _told) = recorded_core(&CoreOptions::default());
    let (_provider, consumer) = two_open_plugins(&core);
    let (timing, _registration) = install_timing(&core, &heard);
    let options = WantOptions {
        timed: Some(true),
        now: 0,
        timeout_millis: 100,
        ..WantOptions::default()
    };
    let want = core.want_service(consumer, "p:a", &options).unwrap();
    timing.now.store(100, Ordering::SeqCst);
    heard.clear();
    core.ledger().unwrap();
    let settled = heard
        .payloads()
        .into_iter()
        .find_map(|payload| match payload {
            EventPayload::WantSettled {
                want: settled,
                refusal,
                ..
            } if settled == want => Some(refusal.code),
            _ => None,
        });
    assert_eq!(settled, Some(RefusalCode::TIMED_OUT));
    assert_eq!(
        timing.followed.lock().unwrap().last(),
        Some(&Followed {
            deadline: None,
            deadline_event_heard: false
        }),
        "follow heard the cleared deadline before its event was dispatched"
    );
    assert!(heard.payloads().contains(&EventPayload::Deadline(None)));
}

#[test]
fn a_second_set_of_deadline_hooks_is_the_callers_mistake_and_removed_hooks_hear_no_more() {
    let (core, heard, _told) = recorded_core(&CoreOptions::default());
    let (_provider, consumer) = two_open_plugins(&core);
    let (timing, registration) = install_timing(&core, &heard);
    let second = DeadlineHooks {
        now: Arc::new(|| 0),
        follow: Arc::new(|_deadline| {}),
    };
    assert!(matches!(
        core.install_deadline_hooks(second),
        Err(Answer::Mistake(_))
    ));
    core.remove_deadline_hooks(registration).unwrap();
    assert_eq!(
        core.remove_deadline_hooks(registration),
        Err(Answer::Absent)
    );
    let options = WantOptions {
        timed: Some(true),
        now: 0,
        timeout_millis: 100,
        ..WantOptions::default()
    };
    core.want_service(consumer, "p:a", &options).unwrap();
    assert!(timing.followed.lock().unwrap().is_empty());
}

#[test]
fn calls_from_several_threads_each_event_heard_once_and_in_order() {
    let (core, heard, told) = recorded_core(&CoreOptions::default());
    two_open_plugins(&core);
    let first = core.last_sequence();
    let workers: Vec<_> = (0..4)
        .map(|worker| {
            let core = core.clone();
            thread::spawn(move || {
                for round in 0..50 {
                    let _ =
                        core.release_plugin(Handle(0x0000_0100_0000_0000 + worker * 1000 + round));
                }
            })
        })
        .collect();
    workers
        .into_iter()
        .for_each(|worker| worker.join().unwrap());
    let sequences: Vec<u64> = heard
        .all()
        .iter()
        .map(|event| event.sequence)
        .filter(|sequence| *sequence > first)
        .collect();
    let expected: Vec<u64> = (first + 1..=core.last_sequence()).collect();
    assert_eq!(sequences, expected);
    assert_eq!(sequences.len(), 200);
    assert!(told.all().is_empty());
}
