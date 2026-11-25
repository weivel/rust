//! The wrapper's crate-internal parts against the real core: deferred steps, destroy and the deadline hooks, which
//! the timer and the host build on and no caller of the crate reaches.

use std::env::consts::{DLL_PREFIX, DLL_SUFFIX};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use super::wrapper::{DeadlineHooks, HooksRegistration};
use super::{
    Answer, Core, CoreBinary, CoreOptions, ErrorSink, Event, EventPayload, Failure, FailureKind,
    Handle, Manifest, RefusalCode, ServiceOffer, ServiceWant, WantOptions, WatchChange, Wrapping,
};

/// Every event a listener heard, in the order it heard them.
#[derive(Clone, Default)]
struct Heard {
    events: Arc<Mutex<Vec<Event>>>,
}

impl Heard {
    fn listener(&self) -> super::Listener {
        let events = self.events.clone();
        Arc::new(move |_core: &Core, event: &Event| events.lock().unwrap().push(event.clone()))
    }

    fn payloads(&self) -> Vec<EventPayload> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|event| event.payload.clone())
            .collect()
    }

    fn clear(&self) {
        self.events.lock().unwrap().clear();
    }
}

#[derive(Default)]
struct Told {
    failures: Mutex<Vec<Failure>>,
}

impl ErrorSink for Told {
    fn report(&self, failure: &Failure) {
        self.failures.lock().unwrap().push(failure.clone());
    }
}

impl Told {
    fn all(&self) -> Vec<Failure> {
        self.failures.lock().unwrap().clone()
    }
}

fn recorded_core(options: &CoreOptions) -> (Core, Heard, Arc<Told>) {
    let library = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("build")
        .join("native")
        .join(format!("{DLL_PREFIX}weivel_core{DLL_SUFFIX}"));
    assert!(
        library.is_file(),
        "{} is missing: run `cargo run -p native` first",
        library.display()
    );
    let binary = CoreBinary::load(&library).expect("the pinned core loads");
    let heard = Heard::default();
    let told = Arc::new(Told::default());
    let wrapping = Wrapping {
        listener: Some(heard.listener()),
        sink: Some(told.clone()),
    };
    let core = Core::create(&binary, options, wrapping).expect("the core is created");
    (core, heard, told)
}

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

/// Records, when it is dropped, whether the layer lock was held at that moment.
struct LockProbe {
    core: Core,
    dropped_under_the_lock: Arc<AtomicBool>,
}

impl Drop for LockProbe {
    fn drop(&mut self) {
        self.dropped_under_the_lock
            .store(self.core.is_locked(), Ordering::SeqCst);
    }
}

fn timed_want() -> WantOptions {
    WantOptions {
        timed: Some(true),
        now: 0,
        timeout_millis: 100,
        ..WantOptions::default()
    }
}

#[test]
fn a_follow_hook_dropped_while_it_is_told_is_dropped_outside_the_lock() {
    let (core, _heard, _told) = recorded_core(&CoreOptions::default());
    let (_provider, consumer) = two_open_plugins(&core);
    let under_the_lock = Arc::new(AtomicBool::new(false));
    let probe = LockProbe {
        core: core.clone(),
        dropped_under_the_lock: under_the_lock.clone(),
    };
    let registration = Arc::new(Mutex::new(None::<HooksRegistration>));
    let inner_registration = registration.clone();
    let hooks = DeadlineHooks {
        now: Arc::new(|| 0),
        follow: Arc::new(move |_deadline| {
            let _held = &probe;
            if let Some(installed) = inner_registration.lock().unwrap().take() {
                probe.core.remove_deadline_hooks(installed).unwrap();
            }
        }),
    };
    *registration.lock().unwrap() = Some(core.install_deadline_hooks(hooks).unwrap());
    core.want_service(consumer, "p:a", &timed_want()).unwrap();
    assert!(
        registration.lock().unwrap().is_none(),
        "follow removed its own hooks"
    );
    assert!(
        !under_the_lock.load(Ordering::SeqCst),
        "the last follow hook was dropped under the layer lock"
    );
}

#[test]
fn a_clock_dropped_while_it_is_read_is_dropped_outside_the_lock() {
    let (core, _heard, _told) = recorded_core(&CoreOptions::default());
    let (_provider, consumer) = two_open_plugins(&core);
    let under_the_lock = Arc::new(AtomicBool::new(false));
    let probe = LockProbe {
        core: core.clone(),
        dropped_under_the_lock: under_the_lock.clone(),
    };
    let registration = Arc::new(Mutex::new(None::<HooksRegistration>));
    let inner_registration = registration.clone();
    let hooks = DeadlineHooks {
        now: Arc::new(move || {
            let _held = &probe;
            if let Some(installed) = inner_registration.lock().unwrap().take() {
                probe.core.remove_deadline_hooks(installed).unwrap();
            }
            0
        }),
        follow: Arc::new(|_deadline| {}),
    };
    let installed = core.install_deadline_hooks(hooks).unwrap();
    core.want_service(consumer, "p:a", &timed_want()).unwrap();
    *registration.lock().unwrap() = Some(installed);
    core.ledger().unwrap();
    assert!(
        registration.lock().unwrap().is_none(),
        "the clock removed its own hooks"
    );
    assert!(
        !under_the_lock.load(Ordering::SeqCst),
        "the last clock was dropped under the layer lock"
    );
}
