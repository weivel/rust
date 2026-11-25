//! Arms the core's next deadline on the caller's scheduler, as the core's `docs/contract/timer.md` asks: the core
//! owns no clock and takes time only as a number its caller hands in.
//!
//! [`CoreTimer`] follows the latest deadline announcement the wrapper drained and nothing else, hands the core the
//! time through the wrapper when its task runs, waits in steps on a scheduler that cannot wait as long, and gives the
//! wrapper the clock for its check before each call. [`SystemScheduler`] is the default: one thread of its own,
//! waiting on a condition variable, its clock read from [`std::time::Instant`].

mod core_timer;
mod scheduler;
mod system;

pub use core_timer::{CoreTimer, TimerObserver};
pub use scheduler::{ScheduledTask, Scheduler, TaskCallback};
pub use system::SystemScheduler;
