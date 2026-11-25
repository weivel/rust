The `timer` module: the scheduler a caller hands the crate, the default one on a thread of its own, and the timer that follows one core's deadline announcements on it. What it guarantees is [the timer doc](../../../timer.md)'s.

<!-- generated: source-children -->
- [core_timer.rs](../../../../src/timer/core_timer.rs) - the timer of one core: armed from the latest announcement, firing through the wrapper, waiting in steps
- [mod.rs](../../../../src/timer/mod.rs) - the module's public surface
- [scheduler.rs](../../../../src/timer/scheduler.rs) - the `Scheduler` and `ScheduledTask` traits a caller implements
- [system.rs](../../../../src/timer/system.rs) - the system scheduler: one thread of its own on a condition variable
<!-- end generated -->
