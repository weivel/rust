//! Plugins built into the test plugins' own library, loaded across the plugin seam.

use std::sync::{Arc, Mutex};

use weivel_rust::context::{Context, ForeignObject, Object, Pending, address_of};
use weivel_rust::core::{Answer, EdgeSource, FailureKind};
use weivel_rust::host::{ActivationState, Entry, Finished, HostOptions, HostScheduler};

use crate::support::shared_library;
use crate::{Settles, kept_host, manifest};

/// The typed view the test plugins compile too.
#[repr(C)]
struct Probe {
    size: u32,
    value: u64,
}

const PLUGIN_PROBE_VALUE: u64 = 0x5EA4;

fn plugins_library() -> std::path::PathBuf {
    shared_library("weivel_test_plugins")
}

fn addresses_written_by(kept: &crate::Kept, plugin: &str, word: &str) -> Vec<usize> {
    kept.logs()
        .into_iter()
        .filter(|record| record.plugin.as_deref() == Some(plugin))
        .filter_map(|record| {
            record
                .text
                .strip_prefix(&format!("{word} "))
                .map(str::to_owned)
        })
        .map(|address| {
            address
                .parse()
                .expect("a plugin writes an address as a number")
        })
        .collect()
}

/// Gets what the plugin across the seam registered, from this binary's side.
#[derive(Default)]
struct Getting {
    got: Mutex<Option<Object>>,
}

impl Entry for Getting {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        *self.got.lock().unwrap() = context.get("provider:probe").ok();
        Pending::settled(Ok(()))
    }
}

#[test]
fn a_plugin_across_the_seam_and_the_host_hand_each_other_their_own_pointers() {
    let (host, kept) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let host_probe: Object = Arc::new(Probe {
        size: size_of::<Probe>() as u32,
        value: 7,
    });
    let _host_service = host
        .register_host_service("host:probe", host_probe.clone())
        .expect("the host's probe is registered");
    let providing = host
        .load_entry(&plugins_library(), None)
        .expect("the providing plugin loads");
    let consuming = host
        .load_entry(&plugins_library(), Some("consuming_entry"))
        .expect("the consuming plugin loads");
    let getting = Arc::new(Getting::default());
    let plugins = [
        host.register(
            manifest("provider", &["provider:probe"], &["host:probe"]),
            providing,
        )
        .unwrap(),
        host.register(manifest("consumer", &[], &["provider:probe"]), consuming)
            .unwrap(),
        host.register(
            manifest("getter", &[], &["provider:probe"]),
            getting.clone(),
        )
        .unwrap(),
    ];
    let activation = host.activate(&plugins, EdgeSource::SERVICES).unwrap();
    let states: Vec<_> = activation
        .outcomes
        .iter()
        .map(|outcome| outcome.value().expect("every activate settled").state)
        .collect();
    let reasons: Vec<String> = host
        .ledger()
        .unwrap()
        .into_iter()
        .map(|row| row.refusal.detail)
        .collect();
    assert_eq!(states, vec![ActivationState::Active; 3], "{reasons:?}");

    let registered = addresses_written_by(&kept, "provider", "registered");
    assert_eq!(
        addresses_written_by(&kept, "provider", "got"),
        vec![address_of(&host_probe)]
    );
    assert_eq!(addresses_written_by(&kept, "consumer", "got"), registered);
    let got = getting
        .got
        .lock()
        .unwrap()
        .clone()
        .expect("the getter got the probe");
    let address = got
        .downcast_ref::<ForeignObject>()
        .expect("an object across the seam is its provider's address")
        .address();
    assert_eq!(vec![address], registered);
    // SAFETY: the address is the providing plugin's own `Probe`, which it keeps while its registration stands.
    let probe = unsafe { &*(address as *const Probe) };
    assert_eq!(probe.value, PLUGIN_PROBE_VALUE);
    host.destroy().wait();
    assert!(kept.failures().is_empty(), "{:?}", kept.failures());
}

#[test]
fn a_panic_in_an_activate_across_the_seam_reaches_the_sink_and_the_host_goes_on() {
    let (host, kept) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let panicking = host
        .load_entry(&plugins_library(), Some("panicking_entry"))
        .expect("the panicking plugin loads");
    let plugins = [
        host.register(manifest("panicking", &[], &[]), panicking)
            .unwrap(),
        host.register(manifest("calm", &[], &[]), Arc::new(Settles))
            .unwrap(),
    ];
    let activation = host.activate(&plugins, EdgeSource::SERVICES).unwrap();
    let states: Vec<_> = activation
        .outcomes
        .iter()
        .map(|outcome| outcome.value().expect("every activate settled").state)
        .collect();
    assert_eq!(
        states,
        vec![ActivationState::Quarantined, ActivationState::Active]
    );
    let told: Vec<_> = kept
        .failures()
        .into_iter()
        .map(|failure| (failure.kind, failure.text.contains("panics on purpose")))
        .collect();
    assert_eq!(told, vec![(FailureKind::PluginCallback, true)]);
    assert_eq!(host.ledger().expect("the host goes on").len(), 2);
    host.destroy().wait();
}

#[test]
fn an_entry_symbol_the_library_lacks_is_the_callers_mistake() {
    let (host, _) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let loaded = host.load_entry(&plugins_library(), Some("no_such_entry"));
    assert!(matches!(loaded, Err(Answer::Mistake(_))));
    host.destroy().wait();
}

fn texts_written_by(kept: &crate::Kept, plugin: &str) -> Vec<String> {
    kept.logs()
        .into_iter()
        .filter(|record| record.plugin.as_deref() == Some(plugin))
        .map(|record| record.text)
        .collect()
}

/// Holds what it gets and lets go as soon as it is revoked awaiting its release.
#[derive(Default)]
struct HoldsAndLetsGo {
    released: Arc<Mutex<Vec<Result<bool, Answer>>>>,
}

impl Entry for HoldsAndLetsGo {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let held = context.get("owner:feed");
        let releasing = context.clone();
        let released = self.released.clone();
        context.listen(move |event| {
            if let weivel_rust::context::PluginEvent::Revoked(revocation) = event {
                released
                    .lock()
                    .unwrap()
                    .push(releasing.release_holding(revocation));
            }
        });
        Pending::settled(held.map(|_| ()).map_err(|refused| refused.to_string()))
    }
}

#[test]
fn a_registration_awaiting_releases_across_the_seam_hears_its_holder_let_go_in_its_deactivate() {
    let (host, kept) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let owning = host
        .load_entry(&plugins_library(), Some("owning_entry"))
        .expect("the owning plugin loads");
    let holding = Arc::new(HoldsAndLetsGo::default());
    let owner = host
        .register(manifest("owner", &["owner:feed"], &[]), owning)
        .unwrap();
    let holder = host
        .register(manifest("holder", &[], &["owner:feed"]), holding.clone())
        .unwrap();
    let activation = host
        .activate(&[owner, holder], EdgeSource::SERVICES)
        .unwrap();
    let states: Vec<_> = activation
        .outcomes
        .iter()
        .map(|outcome| outcome.wait().state)
        .collect();
    assert_eq!(
        states,
        vec![ActivationState::Active; 2],
        "{:?}",
        kept.failures()
    );
    assert_eq!(host.release(owner).wait(), Ok(true));
    assert_eq!(*holding.released.lock().unwrap(), vec![Ok(true)]);
    let written = texts_written_by(&kept, "owner");
    assert!(written.contains(&"deactivating".to_owned()), "{written:?}");
    assert!(
        written.contains(&"holdings released owner:feed".to_owned()),
        "{written:?}"
    );
    host.destroy().wait();
}

#[test]
fn a_refusal_and_a_long_text_cross_the_seam_whole() {
    let (host, kept) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let owning = host
        .load_entry(&plugins_library(), Some("owning_entry"))
        .expect("the owning plugin loads");
    let owner = host
        .register(manifest("owner", &["owner:feed"], &[]), owning)
        .unwrap();
    host.activate(&[owner], EdgeSource::SERVICES).unwrap();
    let written = texts_written_by(&kept, "owner");
    let refused = written
        .iter()
        .find(|text| text.starts_with("refused "))
        .expect("the owner wrote the refusal its get met");
    assert!(refused.contains("declined"), "{refused}");
    assert!(
        written.contains(&"reach refused nobody:here Some(\"nobody:here\")".to_owned()),
        "{written:?}"
    );
    assert!(
        written
            .iter()
            .any(|text| text.len() == 4096 && text.bytes().all(|byte| byte == b'x'))
    );
    host.destroy().wait();
}

#[test]
fn a_plugin_across_the_seam_lets_go_of_a_c_definition_it_hears_revoked() {
    let _turn = crate::ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (host, kept) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let declaration = weivel_rust::host::LibraryDeclaration {
        import_name: "echo".to_owned(),
        version: "0.2.0".to_owned(),
        release: None,
        services: vec![weivel_rust::host::LibraryServiceDeclaration {
            id: "echo".to_owned(),
            functions: vec![weivel_rust::host::LibraryFunction {
                name: "definition".to_owned(),
                symbol: "echo_service_definition".to_owned(),
                kind: weivel_rust::host::FunctionKind::Definition,
                context: 0,
            }],
            description: None,
        }],
        contracts: Vec::new(),
    };
    let loaded = host
        .load_library(&shared_library("echo"), &declaration)
        .expect("the echo library loads");
    let definition = address_of(&loaded.services()[0].1);
    let holding = host
        .load_entry(&plugins_library(), Some("holding_entry"))
        .expect("the holding plugin loads");
    let holder = host
        .register(manifest("holder", &[], &["echo"]), holding)
        .unwrap();
    let activation = host.activate(&[holder], EdgeSource::CAPABILITIES).unwrap();
    assert_eq!(
        activation.outcomes[0].wait().state,
        ActivationState::Active,
        "{:?}",
        kept.failures()
    );
    assert_eq!(host.unload_library(&loaded), Ok(true));
    let written = texts_written_by(&kept, "holder");
    assert_eq!(
        written,
        vec![
            format!("holds echo at {definition}"),
            "released Ok(true)".to_owned()
        ]
    );
    host.destroy().wait();
}

#[test]
fn a_plugin_across_the_seam_waiting_on_the_dispatching_thread_is_quarantined_rather_than_hung() {
    let (host, _) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let waiting = host
        .load_entry(&plugins_library(), Some("waiting_entry"))
        .expect("the waiting plugin loads");
    let plugin = host
        .register(manifest("waiting", &[], &["provider:probe"]), waiting)
        .unwrap();
    let activation = host.activate(&[plugin], EdgeSource::CAPABILITIES).unwrap();
    assert_eq!(
        activation.outcomes[0].wait().state,
        ActivationState::Quarantined
    );
    let ledger = host.ledger().unwrap();
    let row = ledger
        .iter()
        .find(|row| row.import_name == "waiting")
        .unwrap();
    assert!(
        row.refusal.detail.contains("would never settle"),
        "{:?}",
        row.refusal
    );
    host.destroy().wait();
}
