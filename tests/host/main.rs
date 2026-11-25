//! The host's own tests beside the layer suite: plugins across the plugin seam, C libraries and their descriptions,
//! definitions bound from a description, and the races between a release, a settling activate and the destroy.

#[path = "../support/mod.rs"]
mod support;

mod descriptions;
mod libraries;
mod races;
mod seam;

use std::sync::{Arc, Mutex};

use weivel_rust::context::{Context, HostLog, LogRecord, Pending, pending};
use weivel_rust::core::{CoreOptions, ErrorSink, Failure, Manifest, ServiceOffer, ServiceWant};
use weivel_rust::host::{Entry, Finished, Host, HostOptions, HostScheduler};

/// A process maps a library once however often it is opened, so the tests that load the same C library take turns.
pub static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Every record the host's log and its error sink received.
#[derive(Default)]
pub struct Kept {
    pub logs: Mutex<Vec<LogRecord>>,
    pub failures: Mutex<Vec<Failure>>,
}

impl Kept {
    pub fn logs(&self) -> Vec<LogRecord> {
        self.logs.lock().unwrap().clone()
    }

    pub fn failures(&self) -> Vec<Failure> {
        self.failures.lock().unwrap().clone()
    }
}

struct KeptLog(Arc<Kept>);

impl HostLog for KeptLog {
    fn write(&self, record: &LogRecord) {
        self.0.logs.lock().unwrap().push(record.clone());
    }
}

struct KeptSink(Arc<Kept>);

impl ErrorSink for KeptSink {
    fn report(&self, failure: &Failure) {
        self.0.failures.lock().unwrap().push(failure.clone());
    }
}

/// A host on the pinned core whose log and sink are kept.
pub fn kept_host(scheduler: HostScheduler, options: HostOptions) -> (Host, Arc<Kept>) {
    let kept = Arc::new(Kept::default());
    let host = Host::create(
        &support::core_binary(),
        HostOptions {
            core: CoreOptions::default(),
            scheduler,
            log: Some(Arc::new(KeptLog(kept.clone()))),
            sink: Some(Arc::new(KeptSink(kept.clone()))),
            ..options
        },
    )
    .expect("the host is created");
    (host, kept)
}

/// A manifest providing and consuming services by id, each declaration as written.
pub fn manifest(name: &str, provides: &[&str], consumes: &[&str]) -> Manifest {
    Manifest {
        import_name: name.to_owned(),
        services_provided: provides
            .iter()
            .map(|id| ServiceOffer {
                id: (*id).to_owned(),
                attributes: Vec::new(),
            })
            .collect(),
        services_consumed: consumes
            .iter()
            .map(|id| ServiceWant {
                id: (*id).to_owned(),
                provider: None,
                optional: Some(false),
                degradable: Some(false),
                constraints: Vec::new(),
            })
            .collect(),
        ..Manifest::default()
    }
}

/// An entry whose activate settles only when its test settles it.
#[derive(Default)]
pub struct Held {
    pub settle: Mutex<Option<weivel_rust::context::Settle<Finished>>>,
}

impl Entry for Held {
    fn activate(&self, _: &Context) -> Pending<Finished> {
        let (activating, settle) = pending();
        *self.settle.lock().unwrap() = Some(settle);
        activating
    }
}

/// An entry whose activate settles at once, successfully.
pub struct Settles;

impl Entry for Settles {
    fn activate(&self, _: &Context) -> Pending<Finished> {
        Pending::settled(Ok(()))
    }
}
