//! A plugin's entry in Rust, what an activation answers, and what a host is made with.

use std::sync::Arc;

use crate::context::{Context, HostLog, Pending};
use crate::core::{ActivationPlan, CoreOptions, ErrorSink, Handle, Refusal};
use crate::timer::{Scheduler, TimerObserver};

/// How an activate or a deactivate ended: `Ok`, or why it failed, in English.
pub type Finished = Result<(), String>;

/// A plugin's entry: what the host calls to activate it, and optionally to deactivate it.
pub trait Entry: Send + Sync {
    /// Activates the plugin. The host calls it once per open, in plan order, and never waits for it before starting
    /// the next plugin. An activate may run as the dispatcher, so one that must wait for an event another thread
    /// causes answers a result that settles later rather than blocking. A panic, or a result that settles with an
    /// error, is a failed activation, which the host quarantines.
    fn activate(&self, context: &Context) -> Pending<Finished>;

    /// Deactivates the plugin, `None` for an entry that offers no deactivate. A release waits for the result however
    /// long it takes; a panic, or a result that settles with an error, goes to the error sink and the release goes on.
    fn deactivate(&self) -> Option<Pending<Finished>> {
        None
    }
}

/// Where one plugin's activation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationState {
    /// Verified and running.
    Active,
    /// Quarantined: its activate failed, or the core refused its verification.
    Quarantined,
    /// The core refused to open it.
    Refused,
    /// Released while its activate was in flight, or kept from its activate by the destroy.
    Released,
    /// Its open failed for a reason other than the core's refusal: a defect of the crate, told to the error sink.
    Failed,
}

/// One plugin's outcome, and the core's refusal that decided it: of its open for [`ActivationState::Refused`], of its
/// verification for a [`ActivationState::Quarantined`] the core caused. A quarantine for a reason of the host's
/// carries none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The plugin.
    pub plugin: Handle,
    /// Where its activation ended.
    pub state: ActivationState,
    /// The core's refusal that decided it.
    pub refusal: Option<Refusal>,
}

/// What an activation answers once every activate of its plan has been started: the plan, and each plugin's outcome
/// to come, at the index of its plugin in the plan's order.
#[derive(Debug, Clone)]
pub struct Activation {
    /// The core's plan, its order the order the activates were started in.
    pub plan: ActivationPlan,
    /// Each plugin's outcome, settling once.
    pub outcomes: Vec<Pending<Outcome>>,
}

/// Which clock and scheduler the host's timer runs on.
#[derive(Clone, Default)]
pub enum HostScheduler {
    /// A [`crate::timer::SystemScheduler`] the host starts and closes with its destroy: the default.
    #[default]
    System,
    /// The caller's own.
    Given(Arc<dyn Scheduler>),
    /// None: the timer arms nothing, and a want that needs the time is the caller's mistake.
    NoClock,
}

/// What a host is made with.
#[derive(Clone, Default)]
pub struct HostOptions {
    /// The core's options.
    pub core: CoreOptions,
    /// The timer's scheduler.
    pub scheduler: HostScheduler,
    /// The host's log; the core's log events and the plugins' are dropped when `None`.
    pub log: Option<Arc<dyn HostLog>>,
    /// The error sink; standard error when `None`.
    pub sink: Option<Arc<dyn ErrorSink>>,
    /// Hears each move of the host's timer.
    pub timer_observer: Option<TimerObserver>,
}
