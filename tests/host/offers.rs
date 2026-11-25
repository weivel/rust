//! Services the host offers each plugin on its own, the object a plugin registered answered by name, and a provide
//! held to the contract a loaded library publishes, in process.

use std::sync::Arc;

use weivel_rust::context::Object;
use weivel_rust::core::{Answer, EdgeSource, Handle, Manifest, ServiceWant};
use weivel_rust::host::{
    Host, HostOptions, HostScheduler, LibraryContract, LibraryDeclaration, Methods,
};

use crate::support::shared_library;
use crate::{Held, ONE_AT_A_TIME, Settles, kept_host};

fn consuming(name: &str, id: &str) -> Manifest {
    Manifest {
        import_name: name.to_owned(),
        services_consumed: vec![ServiceWant {
            id: id.to_owned(),
            provider: None,
            optional: Some(true),
            degradable: None,
            constraints: Vec::new(),
        }],
        ..Manifest::default()
    }
}

fn activated(host: &Host, names: &[&str], id: &str) -> Vec<Handle> {
    let plugins: Vec<Handle> = names
        .iter()
        .map(|name| {
            host.register(consuming(name, id), Arc::new(Settles))
                .expect("the plugin registers")
        })
        .collect();
    let activation = host
        .activate(&plugins, EdgeSource::SERVICES)
        .expect("the plan is drawn");
    for outcome in &activation.outcomes {
        outcome.wait();
    }
    plugins
}

fn got(host: &Host, plugin: Handle, id: &str) -> Result<Object, Answer> {
    host.context(plugin)
        .expect("the plugin has its context")
        .get(id)
}

#[test]
fn a_service_offered_to_one_plugin_reaches_it_alone_until_it_is_withdrawn() {
    let (host, _) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let plugins = activated(&host, &["alpha", "beta"], "shared.config");
    let (alpha, beta) = (plugins[0], plugins[1]);
    let offered: Object = Arc::new(7_u32);
    let offer = host
        .offer_to_plugin(alpha, "shared.config", offered.clone())
        .expect("the service is offered");
    let reached = got(&host, alpha, "shared.config").expect("alpha reaches its own");
    assert!(Arc::ptr_eq(&reached, &offered));
    assert_eq!(
        got(&host, beta, "shared.config").err(),
        Some(Answer::Absent)
    );
    assert!(matches!(
        host.offer_to_plugin(alpha, "shared.config", offered.clone()),
        Err(Answer::Mistake(_))
    ));
    assert!(matches!(
        host.register_host_service("shared.config", offered),
        Err(Answer::Mistake(_))
    ));
    assert_eq!(offer.withdraw(), Ok(true));
    assert_eq!(offer.withdraw(), Ok(false));
    assert!(got(&host, alpha, "shared.config").is_err());
    host.destroy().wait();
}

#[test]
fn the_object_a_plugin_registered_is_answered_to_the_host_by_its_plugin_alone() {
    let (host, _) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let plugins = activated(&host, &["alpha", "beta"], "alpha:accounts");
    let methods: Object =
        Arc::new(Methods::new().with("login", |arguments| Ok(arguments.cloned())));
    let _registration = host
        .context(plugins[0])
        .expect("alpha has its context")
        .register("alpha:accounts", methods.clone())
        .expect("alpha registers its service");
    let answered = host
        .service_registered_by(plugins[0], "alpha:accounts")
        .expect("alpha's registration is answered");
    assert!(Arc::ptr_eq(&answered, &methods));
    assert!(
        host.service_registered_by(plugins[1], "alpha:accounts")
            .is_none()
    );
    host.destroy().wait();
    assert!(
        host.service_registered_by(plugins[0], "alpha:accounts")
            .is_none()
    );
}

fn contract_of(capability: &str) -> LibraryDeclaration {
    LibraryDeclaration {
        import_name: "described".to_owned(),
        version: "1.0.0".to_owned(),
        release: None,
        services: Vec::new(),
        contracts: vec![LibraryContract {
            capability: capability.to_owned(),
            description: "described_mirror_description".to_owned(),
        }],
    }
}

#[test]
fn a_provide_short_of_a_published_contract_is_refused_until_the_contract_is_unloaded() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (host, _) = kept_host(
        HostScheduler::NoClock,
        HostOptions {
            description_check: Some(Arc::new(crate::support::standard_descriptions())),
            ..HostOptions::default()
        },
    );
    let held = Arc::new(Held::default());
    let plugin = host
        .register(
            Manifest {
                import_name: "alpha".to_owned(),
                capabilities_provided: vec!["described.mirroring".to_owned()],
                ..Manifest::default()
            },
            held.clone(),
        )
        .expect("the plugin registers");
    let library = host
        .load_library(
            &shared_library("weivel_standard_described"),
            &contract_of("described.mirroring"),
        )
        .expect("the contract loads");
    assert!(matches!(
        host.load_library(
            &shared_library("weivel_standard_described"),
            &contract_of("described.mirroring")
        ),
        Err(Answer::Mistake(_))
    ));
    let reading = host
        .load_entry(
            &shared_library("weivel_test_plugins"),
            Some("contract_enums_entry"),
        )
        .expect("the reading plugin loads");
    let reader = host
        .register(
            Manifest {
                import_name: "reader".to_owned(),
                ..Manifest::default()
            },
            reading.clone(),
        )
        .expect("the reading plugin registers");
    let activation = host
        .activate(&[plugin, reader], EdgeSource::CAPABILITIES)
        .expect("the plan is drawn");
    let context = host.context(plugin).expect("the plugin has its context");
    let dark = context
        .capability_enums("described.mirroring")
        .and_then(|enums| enums.number("described_shade", "DESCRIBED_SHADE_DARK"))
        .expect("the contract's enums are published");
    activation.outcomes[1].wait();
    assert_eq!(
        reading.actions().call("shade", None),
        Some(Ok(Some(weivel_rust::host::Tree::Number(f64::from(dark))))),
        "a plugin across the seam reads the same enums"
    );
    let refused = context.provide("described.mirroring", Arc::new(Methods::new()));
    assert!(
        matches!(&refused, Err(Answer::ContractMismatch { capability, .. }) if capability == "described.mirroring"),
        "{refused:?}"
    );
    assert_eq!(host.unload_library(&library), Ok(true));
    assert!(context.capability_enums("described.mirroring").is_none());
    assert_eq!(
        context.provide("described.mirroring", Arc::new(Methods::new())),
        Ok(())
    );
    if let Some(settle) = held.settle.lock().unwrap().take() {
        settle.settle(Ok(()));
    }
    activation.outcomes[0].wait();
    host.destroy().wait();
}
