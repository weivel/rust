//! The test plugins the suite loads across the plugin seam, each exported under a symbol of its own, so one library
//! carries every plugin a test hosts. Each writes what it saw to the host's log, where the suite reads it.

use std::sync::{Arc, Mutex};

use weivel_rust::context::{
    Context, Object, Pending, PluginEvent, ServiceRegistration, WantLimits, address_of,
};
use weivel_rust::core::LogLevel;
use weivel_rust::host::{BoundDefinition, Entry, Finished, Inputs, Value};

/// The typed view both the suite and these plugins compile, so either side can read the other's object.
#[repr(C)]
pub struct Probe {
    pub size: u32,
    pub value: u64,
}

/// The id the providing plugin registers its probe under.
pub const PLUGIN_PROBE: &str = "provider:probe";

/// The id the suite registers its own probe under, as a host service.
pub const HOST_PROBE: &str = "host:probe";

/// What the providing plugin's probe carries.
pub const PLUGIN_PROBE_VALUE: u64 = 0x5EA4;

/// How long the owning plugin's long log line is, past any fixed buffer a table could have carried.
pub const LONG_TEXT_LENGTH: usize = 4096;

fn finished(outcome: Result<(), String>) -> Pending<Finished> {
    Pending::settled(outcome)
}

fn write(context: &Context, text: String) {
    context.log().write(LogLevel::INFO, text);
}

/// Registers a probe of its own and gets the suite's, writing both addresses to the log.
#[derive(Default)]
pub struct Providing;

impl Entry for Providing {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let probe: Object = Arc::new(Probe {
            size: size_of::<Probe>() as u32,
            value: PLUGIN_PROBE_VALUE,
        });
        let registered_at = address_of(&probe);
        if let Err(refused) = context.register(PLUGIN_PROBE, probe) {
            return finished(Err(format!("the probe was not registered: {refused}")));
        }
        write(context, format!("registered {registered_at}"));
        match context.get(HOST_PROBE) {
            Ok(object) => write(context, format!("got {}", address_of(&object))),
            Err(refused) => {
                return finished(Err(format!("the host's probe was not got: {refused}")));
            }
        }
        finished(Ok(()))
    }
}

/// Wants the providing plugin's probe and writes the address its want settles with, and the ids it may reach.
#[derive(Default)]
pub struct Consuming;

impl Entry for Consuming {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let writing = context.clone();
        context
            .want(PLUGIN_PROBE, WantLimits::unbounded())
            .settlement()
            .on_settled(move |settled| match settled {
                Ok(object) => write(&writing, format!("got {}", address_of(object))),
                Err(refused) => write(&writing, format!("not got {refused}")),
            });
        match context.ids() {
            Ok(ids) => write(context, format!("ids {}", ids.join(","))),
            Err(refused) => write(context, format!("no ids {refused}")),
        }
        finished(Ok(()))
    }
}

/// Panics in its activate.
#[derive(Default)]
pub struct Panicking;

impl Entry for Panicking {
    fn activate(&self, _: &Context) -> Pending<Finished> {
        panic!("the test plugin's activate panics on purpose");
    }
}

/// Registers a feed awaiting releases, reaches for what it may not, writes a long line, and unregisters its feed in
/// its deactivate, hearing once every holder let go.
#[derive(Default)]
pub struct Owning {
    registration: Mutex<Option<ServiceRegistration>>,
    context: Mutex<Option<Context>>,
}

impl Entry for Owning {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let listening = context.clone();
        context.listen(move |event| match event {
            PluginEvent::HoldingsReleased { id, .. } => {
                write(&listening, format!("holdings released {id}"))
            }
            PluginEvent::ReachRefused { id, refusal, .. } => write(
                &listening,
                format!("reach refused {id} {:?}", refusal.subject),
            ),
            _ => {}
        });
        match context.register_awaiting_releases("owner:feed", Arc::new(())) {
            Ok(registration) => *self.registration.lock().unwrap() = Some(registration),
            Err(refused) => {
                return finished(Err(format!("the feed was not registered: {refused}")));
            }
        }
        match context.get("nobody:here") {
            Ok(_) => write(context, "got what it may not".to_owned()),
            Err(refused) => write(context, format!("refused {refused}")),
        }
        write(context, "x".repeat(LONG_TEXT_LENGTH));
        *self.context.lock().unwrap() = Some(context.clone());
        finished(Ok(()))
    }

    fn deactivate(&self) -> Option<Pending<Finished>> {
        let context = self.context.lock().unwrap().clone();
        let registration = self.registration.lock().unwrap().take();
        if let (Some(context), Some(registration)) = (context, registration) {
            write(&context, "deactivating".to_owned());
            if let Err(refused) = registration.unregister() {
                return Some(finished(Err(format!(
                    "the feed was not unregistered: {refused}"
                ))));
            }
        }
        Some(finished(Ok(())))
    }
}

/// Gets what its manifest names first and lets go of it as soon as it is revoked awaiting its release.
#[derive(Default)]
pub struct Holding;

impl Entry for Holding {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let Some(wanted) = context
            .manifest()
            .services_consumed
            .first()
            .map(|want| want.id.clone())
        else {
            return finished(Err("the holder consumes nothing".to_owned()));
        };
        let listening = context.clone();
        context.listen(move |event| {
            if let PluginEvent::Revoked(revocation) = event {
                let released = listening.release_holding(revocation);
                write(&listening, format!("released {released:?}"));
            }
        });
        match context.get(&wanted) {
            Ok(object) => write(
                context,
                format!("holds {wanted} at {}", address_of(&object)),
            ),
            Err(refused) => return finished(Err(format!("{wanted} was not got: {refused}"))),
        }
        finished(Ok(()))
    }
}

/// Waits in its activate, on the thread dispatching the host's core, for a want nothing will settle meanwhile.
#[derive(Default)]
pub struct Waiting;

impl Entry for Waiting {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let settled = context
            .want(PLUGIN_PROBE, WantLimits::unbounded())
            .settlement()
            .wait();
        finished(settled.map(|_| ()).map_err(|refused| refused.to_string()))
    }
}

/// Gets the standard's described mirror and reads its enums and calls it, as an in-process plugin does.
#[derive(Default)]
pub struct Describing;

impl Entry for Describing {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let mirror = match context.get("described-mirror") {
            Ok(mirror) => mirror,
            Err(refused) => return finished(Err(format!("the mirror was not got: {refused}"))),
        };
        let shade = context
            .enums_of(&mirror)
            .and_then(|enums| enums.number("described_shade", "DESCRIBED_SHADE_DARK"));
        write(context, format!("shade {shade:?}"));
        let negated = mirror
            .downcast_ref::<BoundDefinition>()
            .map(|bound| {
                let inputs: Inputs = [("number".to_owned(), Value::I64(5))].into_iter().collect();
                format!(
                    "{:?}",
                    bound
                        .call("negate", &inputs)
                        .map(|answer| format!("{answer:?}"))
                )
            })
            .unwrap_or_else(|| "not bound".to_owned());
        write(context, format!("negated {negated}"));
        finished(Ok(()))
    }
}

weivel_rust::export_entry!(Providing);
weivel_rust::export_entry!(consuming_entry, Consuming);
weivel_rust::export_entry!(panicking_entry, Panicking);
weivel_rust::export_entry!(owning_entry, Owning::default());
weivel_rust::export_entry!(holding_entry, Holding);
weivel_rust::export_entry!(waiting_entry, Waiting);
weivel_rust::export_entry!(describing_entry, Describing);

/// Keeps the described mirror it gets and calls it in its deactivate, which may come after the library's unload.
#[derive(Default)]
pub struct Keeping {
    kept: Mutex<Option<(Context, Object)>>,
}

fn negate_through(mirror: &Object) -> String {
    let Some(bound) = mirror.downcast_ref::<BoundDefinition>() else {
        return "not bound".to_owned();
    };
    let inputs: Inputs = [("number".to_owned(), Value::I64(5))].into_iter().collect();
    format!("{:?}", bound.call("negate", &inputs))
}

impl Entry for Keeping {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        match context.get("described-mirror") {
            Ok(mirror) => {
                *self.kept.lock().unwrap() = Some((context.clone(), mirror));
                finished(Ok(()))
            }
            Err(refused) => finished(Err(refused.to_string())),
        }
    }

    fn deactivate(&self) -> Option<Pending<Finished>> {
        let kept = self.kept.lock().unwrap().take();
        if let Some((context, mirror)) = kept {
            write(&context, format!("answered {}", negate_through(&mirror)));
        }
        Some(finished(Ok(())))
    }
}

/// Finishes its deactivate later, from a thread of its own.
#[derive(Default)]
pub struct LateDeactivating;

impl Entry for LateDeactivating {
    fn activate(&self, _: &Context) -> Pending<Finished> {
        finished(Ok(()))
    }

    fn deactivate(&self) -> Option<Pending<Finished>> {
        let (later, settle) = weivel_rust::context::pending();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            settle.settle(Ok(()));
        });
        Some(later)
    }
}

static STASH: Mutex<Option<Object>> = Mutex::new(None);

/// Stashes the described mirror where code of this library outlives its plugin.
#[derive(Default)]
pub struct Stashing;

impl Entry for Stashing {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        match context.get("described-mirror") {
            Ok(mirror) => {
                *STASH.lock().unwrap() = Some(mirror);
                finished(Ok(()))
            }
            Err(refused) => finished(Err(refused.to_string())),
        }
    }
}

/// Calls the stashed mirror, after the plugin that stashed it is gone.
#[derive(Default)]
pub struct StashCalling;

impl Entry for StashCalling {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let Some(mirror) = STASH.lock().unwrap().clone() else {
            return finished(Err("nothing is stashed".to_owned()));
        };
        write(
            context,
            format!("stash answered {}", negate_through(&mirror)),
        );
        finished(Ok(()))
    }
}

/// Opens a cursor through the described walker and keeps it, so the library's unload is what releases it.
#[derive(Default)]
pub struct Cursoring {
    kept: Mutex<Vec<Value>>,
}

impl Entry for Cursoring {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let walker = match context.get("described-walker") {
            Ok(walker) => walker,
            Err(refused) => return finished(Err(refused.to_string())),
        };
        let Some(bound) = walker.downcast_ref::<BoundDefinition>() else {
            return finished(Err("the walker is not bound".to_owned()));
        };
        let inputs: Inputs = [("label".to_owned(), Value::Text(Some("walk".to_owned())))]
            .into_iter()
            .collect();
        match bound.call("open", &inputs) {
            Ok(weivel_rust::host::DescribedAnswer::Answered(answers)) => {
                if let Some(cursor) = answers.value().cloned() {
                    self.kept.lock().unwrap().push(cursor);
                }
                write(context, "opened".to_owned());
                finished(Ok(()))
            }
            other => finished(Err(format!("open answered {other:?}"))),
        }
    }
}

/// The plugin a `host` vector's plugin is: what it does is the vector's own words for it, which the suite writes in
/// the run's directory, found through the run pointer file of this process.
#[derive(Default)]
pub struct HostVectorPlugin {
    run: Mutex<Option<Arc<host_vector::Run>>>,
    registrations: Mutex<Vec<weivel_rust::context::ServiceRegistration>>,
}

impl Entry for HostVectorPlugin {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let run = match host_vector::Run::of(context) {
            Ok(run) => Arc::new(run),
            Err(failure) => return finished(Err(failure)),
        };
        *self.run.lock().unwrap() = Some(run.clone());
        for (id, methods) in run.provided() {
            match context.register(&id, Arc::new(methods)) {
                Ok(registration) => self.registrations.lock().unwrap().push(registration),
                Err(refused) => {
                    return finished(Err(format!("{id} was not registered: {refused}")));
                }
            }
        }
        let (activating, settle) = weivel_rust::context::pending();
        std::thread::spawn(move || settle.settle(run.activate()));
        activating
    }

    fn actions(&self) -> weivel_rust::host::Methods {
        match self.run.lock().unwrap().as_ref() {
            Some(run) => run.actions(),
            None => weivel_rust::host::Methods::new(),
        }
    }
}

weivel_rust::export_entry!(keeping_entry, Keeping::default());
weivel_rust::export_entry!(late_entry, LateDeactivating);
weivel_rust::export_entry!(stashing_entry, Stashing);
weivel_rust::export_entry!(stash_calling_entry, StashCalling);
weivel_rust::export_entry!(cursoring_entry, Cursoring::default());
/// Registers a service offering one method by name, `ping`, answering `pong`, when its action `register` runs, a
/// call made outside any activation, so the registration's events are dispatched within that call.
#[derive(Default)]
pub struct MethodsProviding {
    context: Arc<Mutex<Option<Context>>>,
    registration: Arc<Mutex<Option<weivel_rust::context::ServiceRegistration>>>,
}

impl Entry for MethodsProviding {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        *self.context.lock().unwrap() = Some(context.clone());
        finished(Ok(()))
    }

    fn actions(&self) -> weivel_rust::host::Methods {
        let context = self.context.clone();
        let registration = self.registration.clone();
        weivel_rust::host::Methods::new().with("register", move |_| {
            let context = context.lock().unwrap().clone().ok_or("not activated")?;
            let methods = weivel_rust::host::Methods::new().with("ping", |_| {
                Ok(Some(weivel_rust::host::Tree::Text("pong".to_owned())))
            });
            let registered = context
                .register("provider:methods", Arc::new(methods))
                .map_err(|refused| refused.to_string())?;
            *registration.lock().unwrap() = Some(registered);
            Ok(None)
        })
    }
}

/// Gets its config and keeps it, reading `greeting` through it whenever its action `greeting` runs.
#[derive(Default)]
pub struct ConfigKeeping {
    config: Arc<Mutex<Option<Object>>>,
}

impl Entry for ConfigKeeping {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        match context.get("weivel.config") {
            Ok(config) => {
                *self.config.lock().unwrap() = Some(config);
                finished(Ok(()))
            }
            Err(refused) => finished(Err(refused.to_string())),
        }
    }

    fn actions(&self) -> weivel_rust::host::Methods {
        let kept = self.config.clone();
        weivel_rust::host::Methods::new().with("greeting", move |_| {
            let object = kept.lock().unwrap().clone().ok_or("no config is kept")?;
            let config =
                weivel_rust::standard::Config::of(&object).ok_or("the config is not bound")?;
            match config.get("greeting") {
                Ok(setting) => Ok(Some(weivel_rust::host::Tree::Text(format!("{setting:?}")))),
                Err(refused) => Err(refused.to_string()),
            }
        })
    }
}

weivel_rust::export_entry!(config_keeping_entry, ConfigKeeping::default());
weivel_rust::export_entry!(methods_entry, MethodsProviding::default());
weivel_rust::export_entry!(host_vector_entry, HostVectorPlugin::default());

mod host_vector;
