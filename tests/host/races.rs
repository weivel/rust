//! A release or the destroy arriving from another thread while an activate runs, settles or opens. Each test stops a
//! chosen thread at a chosen point: inside an activate, or at the clock a core call reads once a deadline is armed.

use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, ThreadId};

use weivel_rust::context::{Context, Pending, WantLimits, pending};
use weivel_rust::core::{EdgeSource, FailureKind};
use weivel_rust::host::{ActivationState, Entry, Finished, Host, HostOptions, HostScheduler};
use weivel_rust::timer::{ScheduledTask, Scheduler, TaskCallback};

use crate::{Held, Kept, kept_host, manifest};

/// A point one thread stops at until the test lets it go on.
#[derive(Default)]
struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

#[derive(Default)]
struct GateState {
    reached: bool,
    open: bool,
}

impl Gate {
    fn stop_here(&self) {
        let mut state = self.state.lock().unwrap();
        state.reached = true;
        self.changed.notify_all();
        while !state.open {
            state = self.changed.wait(state).unwrap();
        }
    }

    fn wait_until_reached(&self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut state = self.state.lock().unwrap();
        while !state.reached {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!left.is_zero(), "the stopped thread never reached its stop");
            state = self.changed.wait_timeout(state, left).unwrap().0;
        }
    }

    fn open(&self) {
        self.state.lock().unwrap().open = true;
        self.changed.notify_all();
    }
}

/// A clock standing at 0 whose tasks never fire, which stops one chosen thread at its chosen read of the clock.
#[derive(Default)]
struct StoppingClock {
    chosen: Mutex<Option<(ThreadId, u32)>>,
    gate: Gate,
}

struct NeverFires;

impl ScheduledTask for NeverFires {
    fn cancel(&self) -> bool {
        true
    }
}

impl StoppingClock {
    /// Stops the calling thread at its `read`th read of the clock from now, counting from 1.
    fn stop_this_thread_at(&self, read: u32) {
        *self.chosen.lock().unwrap() = Some((thread::current().id(), read));
    }
}

impl Scheduler for StoppingClock {
    fn now(&self) -> i64 {
        let stop = {
            let mut chosen = self.chosen.lock().unwrap();
            match chosen.as_mut() {
                Some((thread, read)) if *thread == thread::current().id() => {
                    *read -= 1;
                    *read == 0
                }
                _ => false,
            }
        };
        if stop {
            *self.chosen.lock().unwrap() = None;
            self.gate.stop_here();
        }
        0
    }

    fn schedule(&self, _: i64, _: TaskCallback) -> Box<dyn ScheduledTask> {
        Box::new(NeverFires)
    }
}

/// Arms the core's deadline with a want that waits for good, so every later core call reads the clock first.
struct ArmsTheDeadline;

impl Entry for ArmsTheDeadline {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        drop(context.want("nothing:here", WantLimits::timeout(1_000_000)));
        Pending::settled(Ok(()))
    }
}

fn clocked_host() -> (Host, Arc<Kept>, Arc<StoppingClock>) {
    let clock = Arc::new(StoppingClock::default());
    let (host, kept) = kept_host(HostScheduler::Given(clock.clone()), HostOptions::default());
    let arming = host
        .register(
            manifest("arming", &[], &["nothing:here"]),
            Arc::new(ArmsTheDeadline),
        )
        .unwrap();
    host.activate(&[arming], EdgeSource::CAPABILITIES).unwrap();
    (host, kept, clock)
}

fn defects(kept: &Kept) -> Vec<String> {
    kept.failures()
        .into_iter()
        .filter(|failure| failure.kind == FailureKind::LayerDefect)
        .map(|failure| failure.text)
        .collect()
}

#[test]
fn a_release_from_another_thread_waits_for_a_settling_verify() {
    let (host, kept, clock) = clocked_host();
    let held = Arc::new(Held::default());
    let plugin = host
        .register(manifest("held", &[], &[]), held.clone())
        .unwrap();
    let activation = host.activate(&[plugin], EdgeSource::SERVICES).unwrap();
    let settle = held
        .settle
        .lock()
        .unwrap()
        .take()
        .expect("the activate was called");
    let settler_clock = clock.clone();
    let settler = thread::spawn(move || {
        settler_clock.stop_this_thread_at(1);
        settle.settle(Ok(()));
    });
    clock.gate.wait_until_reached();
    let released = host.release(plugin);
    let waited = !released.is_settled();
    clock.gate.open();
    settler.join().unwrap();
    assert!(
        waited,
        "the release ran while the activate's verify was settling"
    );
    assert_eq!(released.wait(), Ok(true));
    let outcome = activation.outcomes[0].wait();
    assert_eq!(outcome.state, ActivationState::Active);
    assert!(defects(&kept).is_empty(), "{:?}", defects(&kept));
    host.destroy().wait();
}

/// An activate that stops until its test lets it return, and records its return and its deactivate.
struct StopsInItsActivate {
    gate: Gate,
    trace: Mutex<Vec<&'static str>>,
}

impl Entry for StopsInItsActivate {
    fn activate(&self, _: &Context) -> Pending<Finished> {
        self.gate.stop_here();
        self.trace.lock().unwrap().push("activate returns");
        pending().0
    }

    fn deactivate(&self) -> Option<Pending<Finished>> {
        self.trace.lock().unwrap().push("deactivate");
        Some(Pending::settled(Ok(())))
    }
}

#[test]
fn a_release_from_another_thread_deactivates_only_once_the_activate_returned() {
    let (host, kept) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let entry = Arc::new(StopsInItsActivate {
        gate: Gate::default(),
        trace: Mutex::new(Vec::new()),
    });
    let plugin = host
        .register(manifest("stopping", &[], &[]), entry.clone())
        .unwrap();
    let activating_host = host.clone();
    let activating = thread::spawn(move || {
        activating_host
            .activate(&[plugin], EdgeSource::SERVICES)
            .unwrap()
    });
    entry.gate.wait_until_reached();
    let released = host.release(plugin);
    entry.gate.open();
    let activation = activating.join().unwrap();
    assert_eq!(released.wait(), Ok(true));
    assert_eq!(
        *entry.trace.lock().unwrap(),
        vec!["activate returns", "deactivate"]
    );
    assert_eq!(
        activation.outcomes[0].wait().state,
        ActivationState::Released
    );
    assert!(defects(&kept).is_empty(), "{:?}", defects(&kept));
    host.destroy().wait();
}

/// Holds the dispatch: its activate runs as the dispatcher and stops until its test lets it return.
struct HoldsTheDispatch {
    gate: Gate,
}

impl Entry for HoldsTheDispatch {
    fn activate(&self, _: &Context) -> Pending<Finished> {
        self.gate.stop_here();
        Pending::settled(Ok(()))
    }
}

/// One thread holds the dispatch, so the destroy runs as a step of that dispatch once it is let go, while a second
/// thread's open, started as a call made during that dispatch, is stopped at the clock its core call reads.
#[test]
fn the_destroy_waits_for_an_open_under_way() {
    let (host, kept, clock) = clocked_host();
    let holding = Arc::new(HoldsTheDispatch {
        gate: Gate::default(),
    });
    let holder = host
        .register(manifest("holder", &[], &[]), holding.clone())
        .unwrap();
    let plugin = host
        .register(manifest("opening", &[], &[]), Arc::new(Held::default()))
        .unwrap();
    let dispatching_host = host.clone();
    let dispatching = thread::spawn(move || {
        dispatching_host
            .activate(&[holder], EdgeSource::SERVICES)
            .unwrap()
    });
    holding.gate.wait_until_reached();
    let opening_host = host.clone();
    let opening_clock = clock.clone();
    let opening = thread::spawn(move || {
        opening_clock.stop_this_thread_at(2);
        opening_host
            .activate(&[plugin], EdgeSource::SERVICES)
            .unwrap()
    });
    clock.gate.wait_until_reached();
    let destroyed = host.destroy();
    holding.gate.open();
    dispatching.join().unwrap();
    let waited = !destroyed.is_settled();
    clock.gate.open();
    let activation = opening.join().unwrap();
    destroyed.wait();
    assert!(waited, "the destroy ended while an open was under way");
    assert_eq!(
        activation.outcomes[0].wait().state,
        ActivationState::Released
    );
    assert!(defects(&kept).is_empty(), "{:?}", defects(&kept));
}

/// Waits on the activating thread, which runs the plan as the dispatcher, for a want only that dispatch settles.
struct WaitsAsTheDispatcher;

impl Entry for WaitsAsTheDispatcher {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let settled = context
            .want("nothing:here", WantLimits::unbounded())
            .settlement()
            .wait();
        Pending::settled(settled.map(|_| ()).map_err(|refused| refused.to_string()))
    }
}

#[test]
fn waiting_on_the_dispatching_thread_for_its_own_want_panics_rather_than_hangs() {
    let (host, _) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let plugin = host
        .register(
            manifest("waits", &[], &["nothing:here"]),
            Arc::new(WaitsAsTheDispatcher),
        )
        .unwrap();
    let activation = host.activate(&[plugin], EdgeSource::CAPABILITIES).unwrap();
    assert_eq!(
        activation.outcomes[0].wait().state,
        ActivationState::Quarantined
    );
    let ledger = host.ledger().unwrap();
    let row = ledger
        .iter()
        .find(|row| row.import_name == "waits")
        .unwrap();
    assert!(
        row.refusal.detail.contains("would never settle"),
        "{:?}",
        row.refusal
    );
    host.destroy().wait();
}

/// Releases its own plugin from inside its activate and waits for that release, which waits for this activate.
struct ReleasesItselfAndWaits {
    host: Mutex<Option<Host>>,
}

impl Entry for ReleasesItselfAndWaits {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let host = self
            .host
            .lock()
            .unwrap()
            .clone()
            .expect("the test hands the host");
        let released = host.release(context.plugin()).wait();
        Pending::settled(Err(format!("the wait came back with {released:?}")))
    }
}

#[test]
fn a_release_waited_for_inside_the_released_plugins_own_activate_panics_rather_than_hangs() {
    let (host, _) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let holding = Arc::new(HoldsTheDispatch {
        gate: Gate::default(),
    });
    let holder = host
        .register(manifest("holder", &[], &[]), holding.clone())
        .unwrap();
    let dispatching_host = host.clone();
    let dispatching = thread::spawn(move || {
        dispatching_host
            .activate(&[holder], EdgeSource::SERVICES)
            .unwrap()
    });
    holding.gate.wait_until_reached();
    let entry = Arc::new(ReleasesItselfAndWaits {
        host: Mutex::new(Some(host.clone())),
    });
    let plugin = host
        .register(manifest("self", &[], &[]), entry.clone())
        .unwrap();
    let activation = host.activate(&[plugin], EdgeSource::SERVICES).unwrap();
    holding.gate.open();
    dispatching.join().unwrap();
    assert_eq!(
        activation.outcomes[0].wait().state,
        ActivationState::Quarantined
    );
    entry.host.lock().unwrap().take();
    host.destroy().wait();
}

/// Waits, on the thread dispatching its own host, for a want of another host that another thread settles.
struct WaitsOnAnotherHost {
    want: Mutex<Option<Pending<weivel_rust::context::WantSettlement>>>,
    providing: Mutex<Option<(Host, weivel_rust::core::Handle)>>,
}

impl Entry for WaitsOnAnotherHost {
    fn activate(&self, _: &Context) -> Pending<Finished> {
        let want = self
            .want
            .lock()
            .unwrap()
            .take()
            .expect("the test hands the want");
        let (other, provider) = self
            .providing
            .lock()
            .unwrap()
            .take()
            .expect("the test hands the provider");
        let providing = thread::spawn(move || {
            other
                .activate(&[provider], EdgeSource::CAPABILITIES)
                .unwrap();
        });
        let settled = want.wait();
        providing.join().unwrap();
        Pending::settled(settled.map(|_| ()).map_err(|refused| refused.to_string()))
    }
}

/// Registers its service, so the other host's want settles.
struct RegistersTheService;

impl Entry for RegistersTheService {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let registered = context.register("other:feed", Arc::new(()));
        Pending::settled(
            registered
                .map(|_| ())
                .map_err(|refused| refused.to_string()),
        )
    }
}

#[test]
fn waiting_while_dispatching_one_host_for_a_result_another_host_settles_is_waited_for() {
    let (waiting_host, _) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let (other, _) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let consumer = other
        .register(
            manifest("consumer", &[], &["other:feed"]),
            Arc::new(Held::default()),
        )
        .unwrap();
    let provider = other
        .register(
            manifest("other", &["other:feed"], &[]),
            Arc::new(RegistersTheService),
        )
        .unwrap();
    other
        .activate(&[consumer], EdgeSource::CAPABILITIES)
        .unwrap();
    let want = other
        .context(consumer)
        .unwrap()
        .want("other:feed", WantLimits::unbounded())
        .settlement();
    let entry = Arc::new(WaitsOnAnotherHost {
        want: Mutex::new(Some(want)),
        providing: Mutex::new(Some((other.clone(), provider))),
    });
    let waiter = waiting_host
        .register(manifest("waiter", &[], &[]), entry)
        .unwrap();
    let activation = waiting_host
        .activate(&[waiter], EdgeSource::SERVICES)
        .unwrap();
    assert_eq!(activation.outcomes[0].wait().state, ActivationState::Active);
    waiting_host.destroy().wait();
    other.destroy().wait();
}
