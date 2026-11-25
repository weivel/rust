//! One core's timer: armed from the latest deadline announcement drained, firing through the wrapper.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use super::scheduler::{ScheduledTask, Scheduler};
use crate::core::{
    Answer, Core, DeadlineHooks, Failure, FailureKind, HooksRegistration, panic_text,
};

/// Hears each move a timer makes: `Some` with the deadline it armed, `None` for a disarm. It is called on the thread
/// that drained the announcement, outside every lock of the crate's, before any event of that call is dispatched;
/// what it panics with goes to the error sink.
pub type TimerObserver = Arc<dyn Fn(Option<i64>) + Send + Sync>;

/// Every task carries the ticket its scheduling was given, and every arm, disarm and destroy draws a new one, so a
/// callback that runs after its task was replaced finds its ticket stale and does nothing.
#[derive(Default)]
struct TimerState {
    ticket: u64,
    task: Option<Box<dyn ScheduledTask>>,
    armed: Option<i64>,
    destroyed: bool,
}

struct TimerShared {
    core: Core,
    scheduler: Arc<dyn Scheduler>,
    observer: Option<TimerObserver>,
    longest_wait: i64,
    state: Mutex<TimerState>,
    registration: Mutex<Option<HooksRegistration>>,
}

/// Arms one core's next deadline on its caller's [`Scheduler`]. A timer made without one arms nothing and reads no
/// clock, and the wrapper then checks nothing before a call. Any thread may call; the timer's own lock is held only
/// inside its methods and never while the scheduler, the clock or the core runs.
pub struct CoreTimer {
    shared: Option<Arc<TimerShared>>,
}

impl CoreTimer {
    /// Starts the timer of `core` on `scheduler`, which becomes the core's deadline hooks, a deadline the core waits
    /// on already armed at once; with no scheduler it installs nothing. Answers [`Answer::Mistake`] when another
    /// timer follows the core already or the scheduler can wait no time, and [`Answer::Destroyed`] for a destroyed
    /// core.
    pub fn start(
        core: &Core,
        scheduler: Option<Arc<dyn Scheduler>>,
        observer: Option<TimerObserver>,
    ) -> Result<CoreTimer, Answer> {
        let Some(scheduler) = scheduler else {
            if core.is_destroyed() {
                return Err(Answer::Destroyed);
            }
            return Ok(CoreTimer { shared: None });
        };
        let longest_wait = scheduler.maximum_wait();
        if longest_wait <= 0 {
            return Err(Answer::Mistake(format!(
                "a scheduler whose longest wait is {longest_wait} milliseconds can wait for no deadline"
            )));
        }
        let shared = Arc::new(TimerShared {
            core: core.clone(),
            scheduler,
            observer,
            longest_wait,
            state: Mutex::new(TimerState::default()),
            registration: Mutex::new(None),
        });
        let registration = core
            .install_deadline_hooks(hooks_of(&shared))
            .map_err(|answer| match answer {
                Answer::Mistake(_) => Answer::Mistake(
                    "another timer follows this core already; destroy it before starting another"
                        .to_owned(),
                ),
                other => other,
            })?;
        *shared
            .registration
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(registration);
        Ok(CoreTimer {
            shared: Some(shared),
        })
    }

    /// The time now on the scheduler's clock, what the crate hands the core as `now` for a want that needs it;
    /// `None` with no scheduler, which has no clock to time a want by.
    pub fn time(&self) -> Option<i64> {
        self.shared.as_ref().map(|shared| shared.scheduler.now())
    }

    /// The deadline the timer is armed for, as it was last told; `None` when disarmed.
    pub fn armed(&self) -> Option<i64> {
        self.shared.as_ref().and_then(|shared| shared.lock().armed)
    }

    /// Disarms the timer for good: cancels its task and takes its own hooks off the core, and only its own. It must
    /// not be called under the layer lock, since taking the hooks off calls through the wrapper.
    pub fn destroy(&self) {
        let Some(shared) = &self.shared else {
            return;
        };
        let cancelled = {
            let mut state = shared.lock();
            if state.destroyed {
                return;
            }
            state.destroyed = true;
            state.armed = None;
            state.ticket += 1;
            state.task.take()
        };
        shared.cancel(cancelled);
        let registration = shared
            .registration
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(registration) = registration {
            let _ = shared.core.remove_deadline_hooks(registration);
        }
    }
}

fn hooks_of(shared: &Arc<TimerShared>) -> DeadlineHooks {
    let scheduler = shared.scheduler.clone();
    let following: Weak<TimerShared> = Arc::downgrade(shared);
    DeadlineHooks {
        now: Arc::new(move || scheduler.now()),
        follow: Arc::new(move |deadline| {
            if let Some(shared) = following.upgrade() {
                TimerShared::follow(&shared, deadline);
            }
        }),
    }
}

impl TimerShared {
    fn lock(&self) -> MutexGuard<'_, TimerState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn follow(shared: &Arc<TimerShared>, deadline: Option<i64>) {
        let (cancelled, arming) = {
            let mut state = shared.lock();
            if state.destroyed {
                return;
            }
            state.armed = deadline;
            state.ticket += 1;
            (state.task.take(), state.ticket)
        };
        shared.cancel(cancelled);
        if let Some(at) = deadline {
            TimerShared::arm(shared, arming, at, None);
        }
        shared.tell_the_observer(deadline);
    }

    fn tell_the_observer(&self, deadline: Option<i64>) {
        let Some(observer) = &self.observer else {
            return;
        };
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| observer(deadline))) {
            self.report(format!(
                "the timer's observer panicked when the timer moved: {}",
                panic_text(&*payload)
            ));
        }
    }

    /// Schedules the wait for `at` under `arming`, reading the clock only when the scheduler is bounded.
    fn arm(shared: &Arc<TimerShared>, arming: u64, at: i64, known_now: Option<i64>) {
        let wait_end = shared.wait_end(at, known_now);
        let firing = Arc::downgrade(shared);
        let scheduled = catch_unwind(AssertUnwindSafe(|| {
            shared.scheduler.schedule(
                wait_end,
                Box::new(move || {
                    if let Some(shared) = firing.upgrade() {
                        TimerShared::fire(&shared, arming);
                    }
                }),
            )
        }));
        let task = match scheduled {
            Ok(task) => task,
            Err(payload) => {
                shared.report(format!(
                    "the scheduler panicked scheduling the timer, so the deadline is left to the check before each call: {}",
                    panic_text(&*payload)
                ));
                shared.disarm_if_current(arming);
                return;
            }
        };
        let stale = {
            let mut state = shared.lock();
            if !state.destroyed && state.ticket == arming {
                state.task = Some(task);
                None
            } else {
                Some(task)
            }
        };
        shared.cancel(stale);
    }

    fn wait_end(&self, at: i64, known_now: Option<i64>) -> i64 {
        if self.longest_wait == i64::MAX {
            return at;
        }
        let now = known_now.unwrap_or_else(|| self.scheduler.now());
        at.min(now.saturating_add(self.longest_wait))
    }

    fn is_current(&self, ticket: u64) -> bool {
        let state = self.lock();
        !state.destroyed && state.ticket == ticket
    }

    fn fire(shared: &Arc<TimerShared>, firing: u64) {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
            TimerShared::fire_current(shared, firing)
        })) {
            shared.report(format!(
                "the timer panicked when it fired: {}",
                panic_text(&*payload)
            ));
        }
    }

    fn fire_current(shared: &Arc<TimerShared>, firing: u64) {
        if !shared.is_current(firing) {
            return;
        }
        let now = shared.scheduler.now();
        let (at, rearming) = {
            let mut state = shared.lock();
            let Some(at) = state
                .armed
                .filter(|_| !state.destroyed && state.ticket == firing)
            else {
                return;
            };
            state.task = None;
            if now < at {
                state.ticket += 1;
            }
            (at, state.ticket)
        };
        if now < at {
            TimerShared::arm(shared, rearming, at, Some(now));
            return;
        }
        if let Err(answer) = shared.core.advance(now)
            && shared.disarm_if_current(firing)
            && answer != Answer::Destroyed
        {
            shared.report(format!(
                    "the core did not take the time the timer handed it, so the deadline is left to the check before each call: {answer}"
                ));
        }
    }

    /// Disarms the timer when nothing armed it again since `arming`, so it never reads armed with no task.
    fn disarm_if_current(&self, arming: u64) -> bool {
        let mut state = self.lock();
        if state.destroyed || state.ticket != arming {
            return false;
        }
        state.armed = None;
        state.ticket += 1;
        true
    }

    fn cancel(&self, task: Option<Box<dyn ScheduledTask>>) {
        let Some(task) = task else {
            return;
        };
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| task.cancel())) {
            self.report(format!(
                "the scheduler panicked cancelling the timer's task: {}",
                panic_text(&*payload)
            ));
        }
    }

    fn report(&self, text: String) {
        self.core.report(&Failure {
            kind: FailureKind::DeadlineHook,
            text,
            sequence: None,
        });
    }
}
