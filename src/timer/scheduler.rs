//! The caller's clock and scheduler: the crate owns no clock and no timer of its own beyond [`super::SystemScheduler`].

/// What a scheduler runs once its time comes.
pub type TaskCallback = Box<dyn FnOnce() + Send>;

/// The caller's clock and scheduler. Each method is called outside every lock of the crate's, so each may call into
/// the core itself. A scheduler runs a task's callback at most once, on any thread, never before its clock reached the
/// time the task was scheduled for, and holds no lock its own [`Scheduler::schedule`] or [`ScheduledTask::cancel`]
/// takes while it runs one, since the callback calls into the core and what that queues schedules and cancels.
pub trait Scheduler: Send + Sync {
    /// The time now, in milliseconds on a monotonic scale of the caller's, the scale every deadline is counted on.
    fn now(&self) -> i64;

    /// Schedules `callback` to run once when the clock reaches `at`, at once when it already has.
    fn schedule(&self, at: i64, callback: TaskCallback) -> Box<dyn ScheduledTask>;

    /// The longest this scheduler can wait, in milliseconds; a deadline further away is waited for in steps of it.
    fn maximum_wait(&self) -> i64 {
        i64::MAX
    }
}

/// One scheduled callback.
pub trait ScheduledTask: Send {
    /// Cancels the task: true when the callback was taken out before it began, false when it began, has run, or the
    /// scheduler cannot tell, which is always safe to answer.
    fn cancel(&self) -> bool;
}
