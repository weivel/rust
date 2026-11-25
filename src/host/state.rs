//! The host: what it keeps of each plugin, its creation, registration, ledger and destroy.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use super::entry::{Entry, HostOptions, HostScheduler, Outcome};
use crate::context::{Context, Contexts, HostLog, LogRecord, Pending, Settle, pending};
use crate::core::{
    Answer, Core, CoreBinary, Failure, FailureKind, Handle, LedgerRow, Manifest, Wrapping,
};
use crate::timer::{CoreTimer, Scheduler, SystemScheduler};

/// An activate in flight: the serial that tells it from a later one, and what settles its outcome.
pub(super) struct InFlight {
    pub(super) serial: u64,
    pub(super) settle: Option<Settle<Outcome>>,
}

/// What the host keeps of one plugin it registered, from its registration until it is forgotten.
pub(super) struct PluginRecord {
    pub(super) manifest: Manifest,
    pub(super) entry: Arc<dyn Entry>,
    pub(super) context: Option<Context>,
    pub(super) in_flight: Option<InFlight>,
    pub(super) activate_called: bool,
    pub(super) release: Option<Pending<Result<bool, Answer>>>,
}

#[derive(Default)]
pub(super) struct Records {
    pub(super) plugins: HashMap<Handle, PluginRecord>,
    pub(super) opened: Vec<Handle>,
    pub(super) last_serial: u64,
    pub(super) destroy: Option<Pending<()>>,
}

pub(super) struct HostShared {
    pub(super) core: Core,
    pub(super) timer: Arc<CoreTimer>,
    pub(super) contexts: Contexts,
    pub(super) system_scheduler: Mutex<Option<Arc<SystemScheduler>>>,
    pub(super) records: Mutex<Records>,
}

/// Registers plugins written in Rust, activates, verifies and releases them, and destroys them with the core, as the
/// core's `docs/contract/host.md` asks. Cloning it shares the host. Any thread may call.
#[derive(Clone)]
pub struct Host {
    pub(super) shared: Arc<HostShared>,
}

impl std::fmt::Debug for Host {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Host")
    }
}

struct SilentLog;

impl HostLog for SilentLog {
    fn write(&self, _: &LogRecord) {}
}

impl Host {
    /// Makes a host over a core created through `binary`, with its timer and its plugins' contexts.
    pub fn create(binary: &CoreBinary, options: HostOptions) -> Result<Host, Answer> {
        let core = Core::create(
            binary,
            &options.core,
            Wrapping {
                listener: None,
                sink: options.sink.clone(),
            },
        )?;
        let (scheduler, system) = scheduler_of(&options.scheduler)?;
        let timer = match CoreTimer::start(&core, scheduler, options.timer_observer.clone()) {
            Ok(timer) => Arc::new(timer),
            Err(answer) => {
                core.destroy();
                return Err(answer);
            }
        };
        let log = options.log.clone().unwrap_or_else(|| Arc::new(SilentLog));
        let contexts = Contexts::create(&core, timer.clone(), log)?;
        Ok(Host {
            shared: Arc::new(HostShared {
                core,
                timer,
                contexts,
                system_scheduler: Mutex::new(system),
                records: Mutex::new(Records::default()),
            }),
        })
    }

    /// Registers a plugin from its manifest value and its entry, answering its handle or the core's refusal.
    pub fn register(&self, manifest: Manifest, entry: Arc<dyn Entry>) -> Result<Handle, Answer> {
        if self.shared.is_destroy_called() {
            return Err(Answer::Destroyed);
        }
        let plugin = self.shared.core.register_manifest(&manifest)?;
        self.shared.records().plugins.insert(
            plugin,
            PluginRecord {
                manifest,
                entry,
                context: None,
                in_flight: None,
                activate_called: false,
                release: None,
            },
        );
        Ok(plugin)
    }

    /// The manifest the host registered `plugin` with; `None` once it is forgotten, or for one never registered.
    pub fn manifest(&self, plugin: Handle) -> Option<Manifest> {
        self.shared
            .records()
            .plugins
            .get(&plugin)
            .map(|record| record.manifest.clone())
    }

    /// The context of `plugin`, made no later than its first open; `None` before that and once it is forgotten.
    pub fn context(&self, plugin: Handle) -> Option<Context> {
        self.shared
            .records()
            .plugins
            .get(&plugin)
            .and_then(|record| record.context.clone())
    }

    /// Every plugin's row of the core's ledger.
    pub fn ledger(&self) -> Result<Vec<LedgerRow>, Answer> {
        if self.shared.is_destroy_called() {
            return Err(Answer::Destroyed);
        }
        Ok(self.shared.core.ledger()?.rows)
    }

    /// Destroys the host: releases every plugin it opened in the reverse of the order it opened them, one whose
    /// activate is in flight and one quarantined included, lets every event and deferred step of those releases run,
    /// then disarms the timer and destroys the core. The result settles once all of it has run; a second destroy
    /// answers the first one's.
    pub fn destroy(&self) -> Pending<()> {
        let (destroyed, settle) = pending();
        let opened = {
            let mut records = self.shared.records();
            if let Some(destroy) = &records.destroy {
                return destroy.clone();
            }
            records.destroy = Some(destroyed.clone());
            let mut opened = records.opened.clone();
            opened.reverse();
            opened
        };
        let shared = self.shared.clone();
        self.shared.run_deferred(move || {
            HostShared::release_for_the_destroy(&shared, opened, settle);
        });
        destroyed
    }
}

fn take_step<T>(kept: &Mutex<Option<T>>) -> Option<T> {
    kept.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}

/// The scheduler the timer follows, and the system scheduler the host closes at its destroy when it made one.
type ChosenScheduler = (Option<Arc<dyn Scheduler>>, Option<Arc<SystemScheduler>>);

fn scheduler_of(choice: &HostScheduler) -> Result<ChosenScheduler, Answer> {
    match choice {
        HostScheduler::System => {
            let system = Arc::new(SystemScheduler::start().ok_or_else(|| {
                Answer::Defect("the system scheduler's thread could not be started".to_owned())
            })?);
            Ok((Some(system.clone()), Some(system)))
        }
        HostScheduler::Given(scheduler) => Ok((Some(scheduler.clone()), None)),
        HostScheduler::NoClock => Ok((None, None)),
    }
}

impl HostShared {
    pub(super) fn records(&self) -> MutexGuard<'_, Records> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn is_destroy_called(&self) -> bool {
        self.records().destroy.is_some()
    }

    pub(super) fn report(&self, kind: FailureKind, text: String) {
        self.core.report(&Failure {
            kind,
            text,
            sequence: None,
        });
    }

    /// Releases each plugin the destroy found opened, one after the other, then finishes once the last release and
    /// everything it queued has run.
    fn release_for_the_destroy(
        shared: &Arc<HostShared>,
        mut opened: Vec<Handle>,
        settle: Settle<()>,
    ) {
        let Some(plugin) = opened.first().copied() else {
            let finishing = shared.clone();
            shared.run_deferred(move || {
                finishing.finish_the_destroy();
                settle.settle(());
            });
            return;
        };
        opened.remove(0);
        let next: Weak<HostShared> = Arc::downgrade(shared);
        HostShared::release_record(shared, plugin).on_settled(move |_| {
            if let Some(shared) = next.upgrade() {
                HostShared::release_for_the_destroy(&shared, opened, settle);
            }
        });
    }

    /// Runs `step` on the dispatch queue behind everything queued, or at once once the core is destroyed and no
    /// dispatch is left to run it.
    pub(super) fn run_deferred(&self, step: impl FnOnce() + Send + 'static) {
        let step = Arc::new(Mutex::new(Some(step)));
        let queued = step.clone();
        let deferred = self.core.defer(move |_| {
            if let Some(step) = take_step(&queued) {
                step();
            }
        });
        if deferred.is_err()
            && let Some(step) = take_step(&step)
        {
            step();
        }
    }

    fn finish_the_destroy(&self) {
        self.timer.destroy();
        self.core.destroy();
        let _ = self.contexts.close();
        let system = self
            .system_scheduler
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(system) = system {
            system.close();
        }
    }
}
