//! Four threads calling through their contexts while a fifth dispatches: none waits for the dispatch, and every event
//! their calls queued is heard by the dispatching thread alone, after the one it was dispatching, once each.

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use weivel_rust::context::{Context, Object, Pending, ServiceWatcher, pending};
use weivel_rust::core::{EdgeSource, Manifest, ServiceOffer, ServiceWant};
use weivel_rust::host::{Entry, Finished, Host, HostOptions, HostScheduler};

use crate::support;

const CALLERS: usize = 4;
const GATE: &str = "gatekeeper:gate";

struct InFlight;

impl Entry for InFlight {
    fn activate(&self, _: &Context) -> Pending<Finished> {
        pending().0
    }
}

/// Records each registration it hears with the thread that dispatched it; hearing the gate, it blocks until the test
/// lets it go, which holds the dispatch open.
struct HeldWatch {
    id: String,
    heard: Arc<Mutex<Vec<(String, ThreadId)>>>,
    gate: Option<Mutex<Receiver<()>>>,
}

impl ServiceWatcher for HeldWatch {
    fn registered(&self, _: &Object) {
        self.heard
            .lock()
            .unwrap()
            .push((self.id.clone(), thread::current().id()));
        if let Some(gate) = &self.gate {
            gate.lock().unwrap().recv().unwrap();
        }
    }

    fn unregistered(&self) {}
}

fn offering(name: &str, id: &str) -> Manifest {
    Manifest {
        import_name: name.to_owned(),
        services_provided: vec![ServiceOffer {
            id: id.to_owned(),
            attributes: Vec::new(),
        }],
        ..Manifest::default()
    }
}

fn wanting(name: &str, ids: &[String]) -> Manifest {
    Manifest {
        import_name: name.to_owned(),
        services_consumed: ids
            .iter()
            .map(|id| ServiceWant {
                id: id.clone(),
                provider: None,
                optional: Some(true),
                degradable: Some(false),
                constraints: Vec::new(),
            })
            .collect(),
        ..Manifest::default()
    }
}

fn service_id(caller: usize) -> String {
    format!("provider-{caller}:service")
}

#[test]
fn four_threads_call_while_one_dispatches() {
    let host = Host::create(
        &support::core_binary(),
        HostOptions {
            scheduler: HostScheduler::NoClock,
            ..HostOptions::default()
        },
    )
    .unwrap();
    let mut ids: Vec<String> = (0..CALLERS).map(service_id).collect();
    ids.push(GATE.to_owned());
    let watcher = host
        .register(wanting("watcher", &ids), Arc::new(InFlight))
        .unwrap();
    let gatekeeper = host
        .register(offering("gatekeeper", GATE), Arc::new(InFlight))
        .unwrap();
    let providers: Vec<_> = (0..CALLERS)
        .map(|caller| {
            host.register(
                offering(&format!("provider-{caller}"), &service_id(caller)),
                Arc::new(InFlight),
            )
            .unwrap()
        })
        .collect();
    let mut every = vec![watcher, gatekeeper];
    every.extend(&providers);
    host.activate(&every, EdgeSource::CAPABILITIES).unwrap();

    let heard = Arc::new(Mutex::new(Vec::new()));
    let (open_the_gate, gate) = mpsc::channel();
    let watching = host.context(watcher).unwrap();
    let mut gate = Some(Mutex::new(gate));
    let mut watches = Vec::new();
    for id in &ids {
        let watch = HeldWatch {
            id: id.clone(),
            heard: heard.clone(),
            gate: if id == GATE { gate.take() } else { None },
        };
        watches.push(watching.watch(id, Arc::new(watch)).unwrap());
    }

    let gatekeeping = host.context(gatekeeper).unwrap();
    let dispatcher = thread::spawn(move || {
        let service: Object = Arc::new(());
        gatekeeping.register(GATE, service).unwrap();
        thread::current().id()
    });
    let held_by = Instant::now() + Duration::from_secs(10);
    while heard.lock().unwrap().is_empty() {
        assert!(
            !dispatcher.is_finished() && Instant::now() < held_by,
            "the gate's registration was never heard"
        );
        thread::yield_now();
    }

    let (called, calls) = mpsc::channel();
    let callers: Vec<_> = providers
        .iter()
        .enumerate()
        .map(|(caller, provider)| {
            let context = host.context(*provider).unwrap();
            let called = called.clone();
            thread::spawn(move || {
                let service: Object = Arc::new(caller);
                let registered = context.register(&service_id(caller), service.clone());
                let got = context.get(&service_id(caller));
                called
                    .send((
                        registered.is_ok(),
                        got.is_ok_and(|object| Arc::ptr_eq(&object, &service)),
                    ))
                    .unwrap();
            })
        })
        .collect();
    for _ in 0..CALLERS {
        let answered = calls
            .recv_timeout(Duration::from_secs(10))
            .expect("a call waited for the dispatch another thread holds open");
        assert_eq!(answered, (true, true));
    }
    assert_eq!(
        heard.lock().unwrap().len(),
        1,
        "an event was heard while the dispatch was held"
    );

    open_the_gate.send(()).unwrap();
    let dispatching = dispatcher.join().unwrap();
    callers
        .into_iter()
        .for_each(|caller| caller.join().unwrap());

    let heard = heard.lock().unwrap().clone();
    assert_eq!(heard[0].0, GATE);
    let mut later: Vec<String> = heard[1..].iter().map(|(id, _)| id.clone()).collect();
    later.sort();
    assert_eq!(later, (0..CALLERS).map(service_id).collect::<Vec<_>>());
    assert!(
        heard.iter().all(|(_, thread)| *thread == dispatching),
        "an event was heard on a thread other than the one dispatching: {heard:?}"
    );
    host.destroy().wait();
}
