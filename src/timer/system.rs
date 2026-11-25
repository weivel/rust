//! The default scheduler: one thread of its own, waiting on a condition variable for the earliest task.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle, ThreadId};
use std::time::{Duration, Instant};

use super::scheduler::{ScheduledTask, Scheduler, TaskCallback};

struct Waiting {
    id: u64,
    at: i64,
    callback: TaskCallback,
}

#[derive(Default)]
struct SystemState {
    tasks: Vec<Waiting>,
    last_id: u64,
    closed: bool,
}

struct SystemShared {
    origin: Instant,
    state: Mutex<SystemState>,
    woken: Condvar,
}

impl SystemShared {
    fn lock(&self) -> MutexGuard<'_, SystemState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn now(&self) -> i64 {
        i64::try_from(self.origin.elapsed().as_millis()).unwrap_or(i64::MAX)
    }

    /// Takes the earliest task once it is due, waiting for it; `None` once the scheduler is closed.
    fn next_due(&self) -> Option<TaskCallback> {
        let mut state = self.lock();
        loop {
            if state.closed {
                return None;
            }
            let earliest = state
                .tasks
                .iter()
                .enumerate()
                .min_by_key(|(_, task)| task.at)
                .map(|(index, task)| (index, task.at));
            let Some((index, at)) = earliest else {
                state = self
                    .woken
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                continue;
            };
            let now = self.now();
            if now >= at {
                return Some(state.tasks.swap_remove(index).callback);
            }
            let delay = Duration::from_millis(u64::try_from(at - now).unwrap_or(u64::MAX));
            state = self
                .woken
                .wait_timeout(state, delay)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    fn run(&self) {
        while let Some(callback) = self.next_due() {
            if catch_unwind(AssertUnwindSafe(callback)).is_err() {
                eprintln!("weivel: a task of the system scheduler panicked");
            }
        }
    }
}

/// A scheduler on one thread of its own, whose clock is the milliseconds since it was made, read from
/// [`Instant`]. It can wait any time. Its thread runs the callbacks it was given, and so, when a timer fires, the
/// listeners of the core that timer hands the time. Dropping it closes it: a task not yet begun never runs, and one
/// running finishes first.
pub struct SystemScheduler {
    shared: Arc<SystemShared>,
    thread: Mutex<Option<JoinHandle<()>>>,
    thread_id: ThreadId,
}

impl SystemScheduler {
    /// Makes a scheduler and starts its thread; `None` when the thread could not be started.
    pub fn start() -> Option<SystemScheduler> {
        let shared = Arc::new(SystemShared {
            origin: Instant::now(),
            state: Mutex::new(SystemState::default()),
            woken: Condvar::new(),
        });
        let running = shared.clone();
        let thread = thread::Builder::new()
            .name("weivel-timer".to_owned())
            .spawn(move || running.run())
            .ok()?;
        Some(SystemScheduler {
            thread_id: thread.thread().id(),
            shared,
            thread: Mutex::new(Some(thread)),
        })
    }

    /// Whether its thread was joined, which a close made from any other thread does before it returns.
    #[cfg(test)]
    pub(crate) fn has_joined(&self) -> bool {
        self.thread
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_none()
    }

    /// Closes the scheduler: a task not yet begun never runs, and one running finishes on its thread, which this
    /// waits for unless it is that thread.
    pub fn close(&self) {
        self.shared.lock().closed = true;
        self.shared.woken.notify_all();
        if thread::current().id() == self.thread_id {
            return;
        }
        let thread = self
            .thread
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }
}

impl Drop for SystemScheduler {
    fn drop(&mut self) {
        self.close();
    }
}

impl Scheduler for SystemScheduler {
    fn now(&self) -> i64 {
        self.shared.now()
    }

    fn schedule(&self, at: i64, callback: TaskCallback) -> Box<dyn ScheduledTask> {
        let mut state = self.shared.lock();
        state.last_id += 1;
        let id = state.last_id;
        if !state.closed {
            state.tasks.push(Waiting { id, at, callback });
        }
        drop(state);
        self.shared.woken.notify_all();
        Box::new(SystemTask {
            shared: self.shared.clone(),
            id,
        })
    }
}

struct SystemTask {
    shared: Arc<SystemShared>,
    id: u64,
}

impl ScheduledTask for SystemTask {
    fn cancel(&self) -> bool {
        let mut state = self.shared.lock();
        let Some(index) = state.tasks.iter().position(|task| task.id == self.id) else {
            return false;
        };
        let cancelled = state.tasks.swap_remove(index);
        drop(state);
        drop(cancelled);
        true
    }
}
