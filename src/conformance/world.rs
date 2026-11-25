//! A host on the real core and the suite's own time, over plugins the driver builds as values, every log and failure
//! kept.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use super::shapes::DriverFailure;
use super::shapes::texts_json;
use crate::context::{Context, HostLog, LogRecord, Pending, pending};
use crate::core::{
    CoreBinary, CoreOptions, EdgeSource, ErrorSink, Failure, Handle, LogLevel, Manifest,
    ServiceOffer, ServiceWant,
};
use crate::host::{DescriptionCheck, Entry, Finished, Host, HostOptions, HostScheduler};
use crate::standard::Json;
use crate::timer::{ScheduledTask, Scheduler, TaskCallback};

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct VirtualTask {
    id: u64,
    at: i64,
    callback: TaskCallback,
}

#[derive(Default)]
struct Clock {
    now: i64,
    tasks: Vec<VirtualTask>,
    fired: Vec<i64>,
    last_id: u64,
}

/// The clock and the scheduler the suite owns and hands the crate in place of its own: time moves only when the
/// suite moves it, and a task runs only when the suite runs time over its due time, at that time, on the suite's
/// thread.
#[derive(Clone, Default)]
pub(crate) struct VirtualTime {
    clock: Arc<Mutex<Clock>>,
}

impl VirtualTime {
    /// Runs every task due by `time`, earliest first and those scheduled meanwhile included, the clock standing at
    /// each one's due time as it runs, then stands at `time`.
    pub(crate) fn run_until(&self, time: i64) {
        while let Some(callback) = self.next_due(time) {
            callback();
        }
        self.move_clock_to(time);
    }

    /// Moves the clock to `time` and runs no task, as a scheduler that has not yet taken its turn.
    pub(crate) fn move_clock_to(&self, time: i64) {
        let mut clock = locked(&self.clock);
        clock.now = clock.now.max(time);
    }

    /// The clock's time each time a task actually ran, in order.
    pub(crate) fn fired(&self) -> Vec<i64> {
        locked(&self.clock).fired.clone()
    }

    fn next_due(&self, time: i64) -> Option<TaskCallback> {
        let mut clock = locked(&self.clock);
        let index = clock
            .tasks
            .iter()
            .enumerate()
            .filter(|(_, task)| task.at <= time)
            .min_by_key(|(_, task)| task.at)
            .map(|(index, _)| index)?;
        let due = clock.tasks.remove(index);
        clock.now = clock.now.max(due.at);
        let now = clock.now;
        clock.fired.push(now);
        Some(due.callback)
    }
}

impl Scheduler for VirtualTime {
    fn now(&self) -> i64 {
        locked(&self.clock).now
    }

    fn schedule(&self, at: i64, callback: TaskCallback) -> Box<dyn ScheduledTask> {
        let mut clock = locked(&self.clock);
        clock.last_id += 1;
        let id = clock.last_id;
        clock.tasks.push(VirtualTask { id, at, callback });
        Box::new(VirtualTaskHandle {
            clock: self.clock.clone(),
            id,
        })
    }
}

struct VirtualTaskHandle {
    clock: Arc<Mutex<Clock>>,
    id: u64,
}

impl ScheduledTask for VirtualTaskHandle {
    fn cancel(&self) -> bool {
        let mut clock = locked(&self.clock);
        let Some(index) = clock.tasks.iter().position(|task| task.id == self.id) else {
            return false;
        };
        let cancelled = clock.tasks.remove(index);
        drop(clock);
        drop(cancelled);
        true
    }
}

#[derive(Default)]
struct Kept {
    logs: Mutex<Vec<LogRecord>>,
    failures: Mutex<Vec<Failure>>,
    timer_moves: Mutex<Vec<Json>>,
}

struct KeptLog(Arc<Kept>);

impl HostLog for KeptLog {
    fn write(&self, record: &LogRecord) {
        locked(&self.0.logs).push(record.clone());
    }
}

struct KeptSink(Arc<Kept>);

impl ErrorSink for KeptSink {
    fn report(&self, failure: &Failure) {
        locked(&self.0.failures).push(failure.clone());
    }
}

/// An entry whose activate never finishes and that offers no deactivate.
pub(crate) struct InFlight;

impl Entry for InFlight {
    fn activate(&self, _: &Context) -> Pending<Finished> {
        pending().0
    }
}

pub(crate) struct DriverWorld {
    pub(crate) time: VirtualTime,
    pub(crate) host: Host,
    kept: Arc<Kept>,
    plugins: BTreeMap<String, Handle>,
    order: Vec<String>,
    /// What must outlive the host's destroy, dropped with the world after it.
    until_destroyed: Vec<Arc<dyn std::any::Any + Send + Sync>>,
}

impl DriverWorld {
    pub(crate) fn new(
        binary: &CoreBinary,
        core: CoreOptions,
        description_check: Option<Arc<dyn DescriptionCheck>>,
    ) -> Result<DriverWorld, DriverFailure> {
        let time = VirtualTime::default();
        let kept = Arc::new(Kept::default());
        let moves = kept.clone();
        let observed_time = time.clone();
        let host = Host::create(
            binary,
            HostOptions {
                core,
                scheduler: HostScheduler::Given(Arc::new(time.clone())),
                log: Some(Arc::new(KeptLog(kept.clone()))),
                sink: Some(Arc::new(KeptSink(kept.clone()))),
                timer_observer: Some(Arc::new(move |deadline: Option<i64>| {
                    locked(&moves.timer_moves).push(Json::Array(vec![
                        Json::Number(observed_time.now() as f64),
                        deadline.map_or(Json::Null, |at| Json::Number(at as f64)),
                    ]));
                })),
                description_check,
            },
        )
        .map_err(|answer| format!("the driver's host could not be created: {answer}"))?;
        Ok(DriverWorld {
            time,
            host,
            kept,
            plugins: BTreeMap::new(),
            order: Vec::new(),
            until_destroyed: Vec::new(),
        })
    }

    /// Keeps `kept` until the world is dropped, which is after its host's destroy.
    pub(crate) fn keep_until_destroyed<T: Send + Sync + 'static>(
        &mut self,
        kept: Arc<T>,
    ) -> Arc<T> {
        self.until_destroyed.push(kept.clone());
        kept
    }

    /// Registers a plugin through the host, failing unless the core registered it.
    pub(crate) fn register(
        &mut self,
        manifest: Manifest,
        entry: Arc<dyn Entry>,
    ) -> Result<(), DriverFailure> {
        let name = manifest.import_name.clone();
        let plugin = self
            .host
            .register(manifest, entry)
            .map_err(|answer| format!("the host did not register {name}: {answer}"))?;
        self.plugins.insert(name.clone(), plugin);
        self.order.push(name);
        Ok(())
    }

    /// Opens registered plugins through the host's activation, each entry's activate left in flight, the plan drawn
    /// from `edge_source`, which a vector that opens plugins this way declares only where every plugin can be placed.
    pub(crate) fn open_in_flight(
        &self,
        names: &[String],
        edge_source: EdgeSource,
    ) -> Result<(), DriverFailure> {
        let handles = names
            .iter()
            .map(|name| self.plugin(name))
            .collect::<Result<Vec<_>, _>>()?;
        let activation = self
            .host
            .activate(&handles, edge_source)
            .map_err(|answer| format!("the host did not open {names:?}: {answer}"))?;
        if handles
            .iter()
            .all(|handle| activation.plan.order.contains(handle))
        {
            Ok(())
        } else {
            Err(format!("the host did not open every plugin of {names:?}"))
        }
    }

    pub(crate) fn context(&self, name: &str) -> Result<Context, DriverFailure> {
        self.host
            .context(self.plugin(name)?)
            .ok_or_else(|| format!("the host holds no context for {name}"))
    }

    pub(crate) fn plugin(&self, name: &str) -> Result<Handle, DriverFailure> {
        self.plugins
            .get(name)
            .copied()
            .ok_or_else(|| format!("the vector names a plugin it does not give: {name}"))
    }

    pub(crate) fn name_of(&self, plugin: Handle) -> Result<String, DriverFailure> {
        self.plugins
            .iter()
            .find(|(_, handle)| **handle == plugin)
            .map(|(name, _)| name.clone())
            .ok_or_else(|| {
                format!(
                    "the layer named a plugin {} the driver never registered",
                    plugin.0
                )
            })
    }

    pub(crate) fn names(&self) -> Vec<String> {
        self.order.clone()
    }

    pub(crate) fn timer_moves(&self) -> Json {
        Json::Array(locked(&self.kept.timer_moves).clone())
    }

    pub(crate) fn logs(&self) -> Vec<LogRecord> {
        locked(&self.kept.logs).clone()
    }

    pub(crate) fn failures(&self) -> Vec<Failure> {
        locked(&self.kept.failures).clone()
    }

    /// How many failures the sink and records the log hold now, to read what was reported from then on.
    pub(crate) fn reported_marks(&self) -> (usize, usize) {
        (self.failures().len(), self.logs().len())
    }

    /// Every failure the sink and every record the log received since `marks`, as texts.
    pub(crate) fn reported_since(&self, (failures, logs): (usize, usize)) -> Json {
        let sunk = self
            .failures()
            .into_iter()
            .skip(failures)
            .map(|failure| format!("{failure:?}"));
        let logged = self
            .logs()
            .into_iter()
            .skip(logs)
            .map(|record| format!("{record:?}"));
        texts_json(sunk.chain(logged))
    }

    /// How many records the log and the error sink hold now, counting only the log's errors when asked.
    pub(crate) fn written(&self, errors_only: bool) -> usize {
        let logged = self
            .logs()
            .iter()
            .filter(|record| !errors_only || record.level == LogLevel::ERROR)
            .count();
        self.failures().len() + logged
    }
}

/// A vector's plugin as a manifest value, every declaration exactly as written, so the core reads it itself.
pub(crate) fn manifest(
    import_name: &str,
    services_provided: &[String],
    services_consumed: &[String],
    capabilities_provided: &[String],
    capabilities_consumed: &[String],
) -> Manifest {
    Manifest {
        import_name: import_name.to_owned(),
        capabilities_provided: capabilities_provided.to_vec(),
        capabilities_consumed: capabilities_consumed.to_vec(),
        services_provided: services_provided
            .iter()
            .map(|id| ServiceOffer {
                id: id.clone(),
                attributes: Vec::new(),
            })
            .collect(),
        services_consumed: services_consumed
            .iter()
            .map(|id| ServiceWant {
                id: id.clone(),
                provider: None,
                optional: Some(false),
                degradable: Some(false),
                constraints: Vec::new(),
            })
            .collect(),
        ..Manifest::default()
    }
}
