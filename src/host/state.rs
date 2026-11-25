//! The host: what it keeps of each plugin, its creation, registration, ledger and destroy.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use super::entry::{Entry, HostOptions, HostScheduler, Outcome};
use super::libraries::LoadedLibrary;
use crate::context::{Context, Contexts, HostLog, LogRecord, Pending, Settle, pending_settled_by};
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
    /// Whether the activate of the plugin's current open was called, so its release owes a deactivate.
    pub(super) activate_called: bool,
    /// True from the call of the activate until that call returned; a release waits for it.
    pub(super) activate_running: bool,
    /// The thread calling the activate while it runs.
    pub(super) activating_thread: Option<std::thread::ThreadId>,
    /// True while a settled activate is verified or quarantined; a release waits for it.
    pub(super) settling: bool,
    pub(super) release: Option<Pending<Result<bool, Answer>>>,
    /// What settles a release asked for while the activate ran or settled, which begins once neither does.
    pub(super) waiting_release: Option<Settle<Result<bool, Answer>>>,
}

impl PluginRecord {
    pub(super) fn is_busy(&self) -> bool {
        self.activate_running || self.settling
    }
}

/// What a destroy waits for before it disarms the timer and destroys the core.
#[derive(Default)]
pub(super) struct UnderWay {
    pub(super) opens: u32,
    pub(super) settlings: u32,
    pub(super) releases: u32,
}

impl UnderWay {
    fn is_quiet(&self) -> bool {
        self.opens == 0 && self.settlings == 0 && self.releases == 0
    }
}

#[derive(Default)]
pub(super) struct Records {
    pub(super) plugins: HashMap<Handle, PluginRecord>,
    pub(super) opened: Vec<Handle>,
    pub(super) last_serial: u64,
    pub(super) destroy: Option<Pending<()>>,
    pub(super) destroy_finished: bool,
    pub(super) under_way: UnderWay,
    /// The destroy's last steps, kept until nothing it waits for is under way.
    pub(super) destroy_end: Option<Box<dyn FnOnce() + Send>>,
    /// How many registrations of the host's own each id has standing, a library's among them.
    pub(super) host_services: HashMap<String, u32>,
    /// The ids the host's own registrations have claimed and the core has not answered yet.
    pub(super) host_service_claims: HashMap<String, u32>,
    /// The C libraries the host loaded and has not unloaded.
    pub(super) libraries: Vec<LoadedLibrary>,
}

impl Records {
    /// Whether a registration of the host's own, its claim, or a live library holds `id`.
    pub(super) fn is_taken(&self, id: &str) -> bool {
        self.host_services.contains_key(id)
            || self.host_service_claims.contains_key(id)
            || self.libraries.iter().any(|library| library.declares(id))
    }
}

pub(super) fn change_count(counts: &mut HashMap<String, u32>, id: &str, up: bool) {
    let count = counts.get(id).copied().unwrap_or(0);
    let changed = if up {
        count + 1
    } else {
        count.saturating_sub(1)
    };
    if changed == 0 {
        counts.remove(id);
    } else {
        counts.insert(id.to_owned(), changed);
    }
}

pub(super) struct HostShared {
    pub(super) core: Core,
    pub(super) timer: Arc<CoreTimer>,
    pub(super) contexts: Contexts,
    pub(super) system_scheduler: Mutex<Option<Arc<SystemScheduler>>>,
    pub(super) description_check: Option<Arc<dyn super::DescriptionCheck>>,
    pub(super) records: Mutex<Records>,
    /// What the host keeps of the plugin seam, dropped with the host.
    pub(super) seam: Arc<super::seam::SeamRegistry>,
    /// Each id offered to plugins one by one; taken before the records whenever both are.
    pub(super) offered: Mutex<HashMap<String, super::offers::Offered>>,
}

/// Registers plugins written in Rust, activates, verifies and releases them, offers host services, loads C libraries
/// as host services, and destroys them with the core, as the core's `docs/contract/host.md` asks. Cloning it shares
/// the host. Any thread may call.
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
                description_check: options.description_check.clone(),
                records: Mutex::new(Records::default()),
                seam: Arc::default(),
                offered: Mutex::new(HashMap::new()),
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
                activate_running: false,
                activating_thread: None,
                settling: false,
                release: None,
                waiting_release: None,
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
    /// waits for every open, settling and release under way, closes every C library it loaded to calls, then disarms
    /// the timer and destroys the core. The result settles once all of it has run; a second destroy answers the first
    /// one's.
    pub fn destroy(&self) -> Pending<()> {
        let (destroyed, settle) = self.shared.settled_by_its_dispatch();
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

/// Counts one thing a destroy waits for while it lives, and lets a waiting destroy end once it was the last.
pub(super) struct UnderWayMark {
    shared: Arc<HostShared>,
    count: fn(&mut UnderWay) -> &mut u32,
}

impl UnderWayMark {
    pub(super) fn opening(shared: &Arc<HostShared>, records: &mut Records) -> UnderWayMark {
        UnderWayMark::counted(shared, records, |under_way| &mut under_way.opens)
    }

    pub(super) fn settling(shared: &Arc<HostShared>, records: &mut Records) -> UnderWayMark {
        UnderWayMark::counted(shared, records, |under_way| &mut under_way.settlings)
    }

    fn counted(
        shared: &Arc<HostShared>,
        records: &mut Records,
        count: fn(&mut UnderWay) -> &mut u32,
    ) -> UnderWayMark {
        *count(&mut records.under_way) += 1;
        UnderWayMark {
            shared: shared.clone(),
            count,
        }
    }
}

impl Drop for UnderWayMark {
    fn drop(&mut self) {
        let mut records = self.shared.records();
        let count = (self.count)(&mut records.under_way);
        *count = count.saturating_sub(1);
        HostShared::end_the_destroy_when_quiet(&self.shared, records);
    }
}

impl HostShared {
    /// A result the host's own steps settle, which a wait on a thread dispatching its core would never see.
    pub(super) fn settled_by_its_dispatch<T>(&self) -> (Pending<T>, Settle<T>) {
        let core = self.core.clone();
        pending_settled_by(Arc::new(move || core.dispatches_here()))
    }

    pub(super) fn records(&self) -> MutexGuard<'_, Records> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn offered(&self) -> MutexGuard<'_, HashMap<String, super::offers::Offered>> {
        self.offered
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

    /// Releases each plugin the destroy found opened, one after the other, then ends the destroy once the last
    /// release, and everything under way that the destroy waits for, has finished.
    fn release_for_the_destroy(
        shared: &Arc<HostShared>,
        mut opened: Vec<Handle>,
        settle: Settle<()>,
    ) {
        let Some(plugin) = opened.first().copied() else {
            let finishing = shared.clone();
            let mut records = shared.records();
            records.destroy_end = Some(Box::new(move || {
                let ending = finishing.clone();
                finishing.run_deferred(move || {
                    ending.finish_the_destroy();
                    settle.settle(());
                });
            }));
            HostShared::end_the_destroy_when_quiet(shared, records);
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

    /// Runs the destroy's last steps once nothing it waits for is under way, letting go of the records first.
    pub(super) fn end_the_destroy_when_quiet(
        _: &Arc<HostShared>,
        mut records: MutexGuard<'_, Records>,
    ) {
        if !records.under_way.is_quiet() {
            return;
        }
        let Some(end) = records.destroy_end.take() else {
            return;
        };
        drop(records);
        end();
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
        self.close_every_library();
        self.timer.destroy();
        self.core.destroy();
        self.records().destroy_finished = true;
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

#[cfg(test)]
mod tests {
    use std::env::consts::{DLL_PREFIX, DLL_SUFFIX};
    use std::path::Path;

    use super::*;

    #[test]
    fn the_destroy_joins_the_system_schedulers_thread() {
        let library = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("build")
            .join("native")
            .join(format!("{DLL_PREFIX}weivel_core{DLL_SUFFIX}"));
        let binary = CoreBinary::load(&library)
            .expect("the pinned core loads: run `cargo run -p native` first");
        let host = Host::create(&binary, HostOptions::default()).expect("the host is created");
        let system = host
            .shared
            .system_scheduler
            .lock()
            .unwrap()
            .clone()
            .expect("a host given no scheduler starts the system one");
        assert!(!system.has_joined());
        host.destroy().wait();
        assert!(system.has_joined());
    }
}
