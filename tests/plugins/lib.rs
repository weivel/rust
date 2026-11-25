//! The test plugins the suite loads across the plugin seam, each exported under a symbol of its own, so one library
//! carries every plugin a test hosts.

use std::sync::Arc;

use weivel_rust::context::{Context, Pending, address_of};
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

fn finished(outcome: Result<(), String>) -> Pending<Finished> {
    Pending::settled(outcome)
}

/// Registers a probe of its own and gets the suite's, writing both addresses to the log.
#[derive(Default)]
pub struct Providing;

impl Entry for Providing {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let probe: weivel_rust::context::Object = Arc::new(Probe {
            size: size_of::<Probe>() as u32,
            value: PLUGIN_PROBE_VALUE,
        });
        let registered_at = address_of(&probe);
        if let Err(refused) = context.register(PLUGIN_PROBE, probe) {
            return finished(Err(format!("the probe was not registered: {refused}")));
        }
        let log = context.log();
        log.write(LogLevel::INFO, format!("registered {registered_at}"));
        match context.get(HOST_PROBE) {
            Ok(object) => log.write(LogLevel::INFO, format!("got {}", address_of(&object))),
            Err(refused) => {
                return finished(Err(format!("the host's probe was not got: {refused}")));
            }
        }
        finished(Ok(()))
    }
}

/// Gets the providing plugin's probe, writing its address to the log.
#[derive(Default)]
pub struct Consuming;

impl Entry for Consuming {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        match context.get(PLUGIN_PROBE) {
            Ok(object) => {
                context
                    .log()
                    .write(LogLevel::INFO, format!("got {}", address_of(&object)));
                finished(Ok(()))
            }
            Err(refused) => finished(Err(format!("the plugin's probe was not got: {refused}"))),
        }
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

weivel_rust::export_entry!(Providing);
weivel_rust::export_entry!(consuming_entry, Consuming);
weivel_rust::export_entry!(panicking_entry, Panicking);
