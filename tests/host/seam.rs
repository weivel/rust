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

#[test]
fn a_plugin_across_the_seam_reads_a_bound_definitions_enums_and_calls_it_as_an_in_process_plugin_does()
 {
    let _turn = crate::ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let check =
        weivel_rust::standard::StandardDescriptions::load(shared_library("weivel_standard"))
            .expect("the standard's description checks load");
    let (host, kept) = kept_host(
        HostScheduler::NoClock,
        HostOptions {
            description_check: Some(Arc::new(check)),
            ..HostOptions::default()
        },
    );
    let _loaded = host
        .load_library(
            &shared_library("weivel_standard_described"),
            &crate::libraries::described_declaration("described_walker_description"),
        )
        .expect("the described library loads");
    let describing = host
        .load_entry(&plugins_library(), Some("describing_entry"))
        .expect("the describing plugin loads");
    let plugin = host
        .register(manifest("reader", &[], &["described-mirror"]), describing)
        .unwrap();
    let activation = host.activate(&[plugin], EdgeSource::CAPABILITIES).unwrap();
    assert_eq!(
        activation.outcomes[0].wait().state,
        ActivationState::Active,
        "{:?}",
        kept.failures()
    );
    let written = texts_written_by(&kept, "reader");
    assert_eq!(written[0], "shade Some(2)");
    assert!(written[1].contains("I64(-5)"), "{written:?}");
    host.destroy().wait();
}

#[test]
fn a_deactivate_finished_on_the_plugins_own_thread_neither_deadlocks_nor_closes_the_library_under_it()
 {
    let (host, _) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let late = host
        .load_entry(&plugins_library(), Some("late_entry"))
        .expect("the late plugin loads");
    let plugin = host.register(manifest("late", &[], &[]), late).unwrap();
    let activation = host.activate(&[plugin], EdgeSource::SERVICES).unwrap();
    assert_eq!(activation.outcomes[0].wait().state, ActivationState::Active);
    let released = host.release(plugin);
    let (told, heard) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = told.send(released.wait());
    });
    let settled = heard.recv_timeout(std::time::Duration::from_secs(10));
    assert_eq!(settled, Ok(Ok(true)), "the release never settled");
    host.destroy().wait();
    std::thread::sleep(std::time::Duration::from_millis(200));
}

/// A host of the standard's described library, and its library loaded.
fn described_and_loaded() -> (
    weivel_rust::host::Host,
    Arc<crate::Kept>,
    weivel_rust::host::LoadedLibrary,
) {
    let check =
        weivel_rust::standard::StandardDescriptions::load(shared_library("weivel_standard"))
            .expect("the standard's description checks load");
    let (host, kept) = kept_host(
        HostScheduler::NoClock,
        HostOptions {
            description_check: Some(Arc::new(check)),
            ..HostOptions::default()
        },
    );
    let loaded = host
        .load_library(
            &shared_library("weivel_standard_described"),
            &crate::libraries::described_declaration("described_walker_description"),
        )
        .expect("the described library loads");
    (host, kept, loaded)
}

#[test]
fn a_bound_definition_called_across_the_seam_while_its_holding_stands_after_the_unload_answers_as_in_process()
 {
    let _turn = crate::ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (host, kept, loaded) = described_and_loaded();
    let keeping = host
        .load_entry(&plugins_library(), Some("keeping_entry"))
        .expect("the keeping plugin loads");
    let plugin = host
        .register(manifest("keeper", &[], &["described-mirror"]), keeping)
        .unwrap();
    let activation = host.activate(&[plugin], EdgeSource::CAPABILITIES).unwrap();
    assert_eq!(
        activation.outcomes[0].wait().state,
        ActivationState::Active,
        "{:?}",
        kept.failures()
    );
    assert_eq!(host.unload_library(&loaded), Ok(true));
    drop(loaded);
    assert_eq!(host.release(plugin).wait(), Ok(true));
    let written = texts_written_by(&kept, "keeper");
    assert_eq!(written.len(), 1, "{written:?}");
    assert!(
        written[0].contains("I64(-5)"),
        "the holding keeps the binary mapped: {written:?}"
    );
    host.destroy().wait();
}

#[test]
fn a_stashed_bound_definition_called_after_its_plugin_and_library_are_gone_answers_unloaded() {
    let _turn = crate::ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (host, kept, loaded) = described_and_loaded();
    let stashing = host
        .load_entry(&plugins_library(), Some("stashing_entry"))
        .expect("the stashing plugin loads");
    let calling = host
        .load_entry(&plugins_library(), Some("stash_calling_entry"))
        .expect("the calling plugin loads");
    let stasher = host
        .register(manifest("stasher", &[], &["described-mirror"]), stashing)
        .unwrap();
    let activation = host.activate(&[stasher], EdgeSource::CAPABILITIES).unwrap();
    assert_eq!(
        activation.outcomes[0].wait().state,
        ActivationState::Active,
        "{:?}",
        kept.failures()
    );
    assert_eq!(host.unload_library(&loaded), Ok(true));
    drop(loaded);
    assert_eq!(host.release(stasher).wait(), Ok(true));
    let caller = host
        .register(manifest("caller", &[], &[]), calling)
        .unwrap();
    let activation = host.activate(&[caller], EdgeSource::SERVICES).unwrap();
    assert_eq!(
        activation.outcomes[0].wait().state,
        ActivationState::Active,
        "{:?}",
        kept.failures()
    );
    let written = texts_written_by(&kept, "caller");
    assert!(
        written.iter().any(|text| text.contains("Unloaded")),
        "{written:?}"
    );
    host.destroy().wait();
}

fn cursors_released(loaded: &weivel_rust::host::LoadedLibrary) -> u64 {
    let mirror = loaded
        .services()
        .into_iter()
        .find(|(id, _)| id == "described-mirror")
        .map(|(_, object)| object)
        .expect("the library registered its mirror");
    let bound = mirror
        .downcast_ref::<weivel_rust::host::BoundDefinition>()
        .expect("the mirror is bound");
    let counted = bound.call("count_releases", &weivel_rust::host::Inputs::new());
    let Ok(weivel_rust::host::DescribedAnswer::Answered(answers)) = counted else {
        panic!("count_releases answered {counted:?}");
    };
    match answers.value() {
        Some(weivel_rust::host::Value::Struct(counts)) => match counts.get("cursors") {
            Some(weivel_rust::host::Value::U64(cursors)) => *cursors,
            other => panic!("count_releases counted cursors as {other:?}"),
        },
        other => panic!("count_releases answered {other:?}"),
    }
}

#[test]
fn the_unload_releases_what_a_plugin_across_the_seam_got_through_the_library_while_it_is_still_mapped()
 {
    let _turn = crate::ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (keeping_host, _, keeping_loaded) = described_and_loaded();
    let (host, kept, loaded) = described_and_loaded();
    let cursoring = host
        .load_entry(&plugins_library(), Some("cursoring_entry"))
        .expect("the cursoring plugin loads");
    let plugin = host
        .register(manifest("walker", &[], &["described-walker"]), cursoring)
        .unwrap();
    let activation = host.activate(&[plugin], EdgeSource::CAPABILITIES).unwrap();
    assert_eq!(
        activation.outcomes[0].wait().state,
        ActivationState::Active,
        "{:?}",
        kept.failures()
    );
    let before = cursors_released(&keeping_loaded);
    assert_eq!(host.unload_library(&loaded), Ok(true));
    assert_eq!(cursors_released(&keeping_loaded), before + 1);
    host.destroy().wait();
    keeping_host.destroy().wait();
}

/// Watches `provider:methods` and calls its `ping` the moment it is registered, during that registration's own call,
/// which the provider makes from an action, outside any activation.
#[derive(Default)]
struct PingingAtOnce {
    answered: Arc<Mutex<Vec<String>>>,
    watching: Mutex<Option<weivel_rust::context::Watching>>,
}

struct Pinging(Arc<Mutex<Vec<String>>>);

impl weivel_rust::context::ServiceWatcher for Pinging {
    fn registered(&self, service: &Object) {
        let answered = weivel_rust::host::methods_of(service)
            .and_then(|methods| methods.call("ping", None))
            .map(|ended| format!("{ended:?}"))
            .unwrap_or_else(|| "no methods".to_owned());
        self.0.lock().unwrap().push(answered);
    }

    fn unregistered(&self) {}
}

impl Entry for PingingAtOnce {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let watching = context.watch("provider:methods", Arc::new(Pinging(self.answered.clone())));
        *self.watching.lock().unwrap() = watching.ok();
        Pending::settled(Ok(()))
    }
}

#[test]
fn a_method_called_back_during_its_registrations_own_call_reaches_the_plugins_object() {
    let (host, _) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let pinging = Arc::new(PingingAtOnce::default());
    let consumer = host
        .register(
            {
                let mut optional = manifest("pinger", &[], &["provider:methods"]);
                optional.services_consumed[0].optional = Some(true);
                optional
            },
            pinging.clone(),
        )
        .unwrap();
    let activation = host.activate(&[consumer], EdgeSource::SERVICES).unwrap();
    activation.outcomes[0].wait();
    let providing = host
        .load_entry(&plugins_library(), Some("methods_entry"))
        .expect("the providing plugin loads");
    let provider = host
        .register(
            manifest("provider", &["provider:methods"], &[]),
            providing.clone(),
        )
        .unwrap();
    let activation = host.activate(&[provider], EdgeSource::SERVICES).unwrap();
    assert_eq!(activation.outcomes[0].wait().state, ActivationState::Active);
    assert_eq!(providing.actions().call("register", None), Some(Ok(None)));
    assert_eq!(
        *pinging.answered.lock().unwrap(),
        vec![format!(
            "{:?}",
            Ok::<_, String>(Some(weivel_rust::host::Tree::Text("pong".to_owned())))
        )]
    );
    host.destroy().wait();
}
