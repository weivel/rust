//! The test plugins the suite loads across the plugin seam, each exported under a symbol of its own, so one library
//! carries every plugin a test hosts. Each writes what it saw to the host's log, where the suite reads it.

use std::sync::{Arc, Mutex};

use weivel_rust::context::{
    Context, Object, Pending, PluginEvent, ServiceRegistration, WantLimits, address_of,
};
use weivel_rust::core::LogLevel;
use weivel_rust::host::{Entry, Finished};

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

weivel_rust::export_entry!(Providing);
weivel_rust::export_entry!(consuming_entry, Consuming);
weivel_rust::export_entry!(panicking_entry, Panicking);
weivel_rust::export_entry!(owning_entry, Owning::default());
weivel_rust::export_entry!(holding_entry, Holding);
weivel_rust::export_entry!(waiting_entry, Waiting);
