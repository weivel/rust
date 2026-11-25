//! The `timer` module's tests: the system scheduler's thread, a timer refused where it could follow nothing, and a
//! timer on a scheduler that cannot wait as long as a deadline waiting in steps, never advancing the core early.

#[path = "../support/mod.rs"]
mod support;

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use weivel_rust::context::{Context, Pending, WantLimits, pending};
use weivel_rust::core::{Answer, EdgeSource, Manifest, RefusalCode, ServiceWant};
use weivel_rust::host::{Entry, Finished, Host, HostOptions, HostScheduler};
use weivel_rust::timer::{CoreTimer, ScheduledTask, Scheduler, SystemScheduler, TaskCallback};

#[test]
fn the_system_scheduler_runs_a_task_once_its_time_comes_and_never_a_cancelled_one() {
    let scheduler = SystemScheduler::start().unwrap();
    let (ran, runs) = mpsc::channel();
    let cancelled_ran = ran.clone();
    let cancelled = scheduler.schedule(
        scheduler.now() + 20,
        Box::new(move || cancelled_ran.send("cancelled").unwrap()),
    );
    let at = scheduler.now() + 30;
    scheduler.schedule(at, Box::new(move || ran.send("due").unwrap()));
    assert!(cancelled.cancel());
    assert_eq!(runs.recv_timeout(Duration::from_secs(10)), Ok("due"));
    assert!(scheduler.now() >= at);
    scheduler.close();
    assert!(runs.try_recv().is_err());
}

#[derive(Default)]
struct Steps {
    now: i64,
    scheduled: Vec<i64>,
    waiting: Vec<(i64, TaskCallback)>,
}

/// A clock the test moves and a scheduler that waits at most `longest` milliseconds.
#[derive(Clone, Default)]
struct SteppedTime {
    steps: Arc<Mutex<Steps>>,
    longest: i64,
}

impl SteppedTime {
    fn run_until(&self, time: i64) {
        loop {
            let due = {
                let mut steps = self.steps.lock().unwrap();
                let index = steps.waiting.iter().position(|(at, _)| *at <= time);
                index.map(|index| {
                    let (at, callback) = steps.waiting.remove(index);
                    steps.now = steps.now.max(at);
                    callback
                })
            };
            match due {
                Some(callback) => callback(),
                None => break,
            }
        }
        let mut steps = self.steps.lock().unwrap();
        steps.now = steps.now.max(time);
    }
}

struct Nothing;

impl ScheduledTask for Nothing {
    fn cancel(&self) -> bool {
        false
    }
}

impl Scheduler for SteppedTime {
    fn now(&self) -> i64 {
        self.steps.lock().unwrap().now
    }

    fn schedule(&self, at: i64, callback: TaskCallback) -> Box<dyn ScheduledTask> {
        let mut steps = self.steps.lock().unwrap();
        steps.scheduled.push(at);
        steps.waiting.push((at, callback));
        Box::new(Nothing)
    }

    fn maximum_wait(&self) -> i64 {
        self.longest
    }
}

struct InFlight;

impl Entry for InFlight {
    fn activate(&self, _: &Context) -> Pending<Finished> {
        pending().0
    }
}

#[test]
fn a_scheduler_that_can_wait_no_time_and_a_second_timer_are_mistakes() {
    let (core, _, _) = support::recorded_core(&Default::default());
    let never = SteppedTime::default();
    assert!(matches!(
        CoreTimer::start(&core, Some(Arc::new(never)), None),
        Err(Answer::Mistake(_))
    ));
    let stepped = SteppedTime {
        longest: 100,
        ..SteppedTime::default()
    };
    let first = CoreTimer::start(&core, Some(Arc::new(stepped.clone())), None).unwrap();
    assert!(matches!(
        CoreTimer::start(&core, Some(Arc::new(stepped.clone())), None),
        Err(Answer::Mistake(_))
    ));
    first.destroy();
    CoreTimer::start(&core, Some(Arc::new(stepped)), None).unwrap();
    core.destroy();
}

#[test]
fn a_deadline_further_than_the_longest_wait_is_waited_for_in_steps() {
    let time = SteppedTime {
        longest: 300,
        ..SteppedTime::default()
    };
    let host = Host::create(
        &support::core_binary(),
        HostOptions {
            scheduler: HostScheduler::Given(Arc::new(time.clone())),
            ..HostOptions::default()
        },
    )
    .unwrap();
    let consumer = host
        .register(
            Manifest {
                import_name: "agenda".to_owned(),
                services_consumed: vec![ServiceWant {
                    id: "calendar:events".to_owned(),
                    provider: None,
                    optional: Some(true),
                    degradable: Some(false),
                    constraints: Vec::new(),
                }],
                ..Manifest::default()
            },
            Arc::new(InFlight),
        )
        .unwrap();
    host.activate(&[consumer], EdgeSource::SERVICES).unwrap();
    let settlement = host
        .context(consumer)
        .unwrap()
        .want("calendar:events", WantLimits::timeout(1000))
        .settlement();
    time.run_until(999);
    assert!(!settlement.is_settled());
    time.run_until(1000);
    let refused = settlement
        .value()
        .expect("the want timed out at its deadline");
    assert!(matches!(
        refused,
        Err(Answer::Refused { refusal: Some(ref refusal), .. }) if refusal.code == RefusalCode::TIMED_OUT
    ));
    assert_eq!(
        time.steps.lock().unwrap().scheduled,
        vec![300, 600, 900, 1000]
    );
    host.destroy().wait();
}
