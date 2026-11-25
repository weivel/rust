//! The wrapper's rules a caller can reach, against the real core: refusals, dispatch order, listeners, the
//! sink and calls from several threads. Its deferred steps, destroy and deadline hooks are crate-internal and
//! tested in `src/core/wrapper_tests.rs`.

use std::sync::{Arc, Mutex};
use std::thread;

use weivel_rust::core::{
    Answer, Core, CoreOptions, EventPayload, FailureKind, Handle, Manifest, RefusalCode,
    RefusalStatus, ServiceOffer, ServiceWant, WatchChange,
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
