//! A definition an answer carried, a scope's config, held across the plugin seam no longer than that answer.

use std::path::PathBuf;
use std::sync::Arc;

use weivel_rust::context::Object;
use weivel_rust::core::{EdgeSource, Manifest, ServiceWant};
use weivel_rust::host::{ActivationState, HostOptions, HostScheduler, Tree};
use weivel_rust::standard::{Config, HostProvider, Scopes, Setting, Standard};

use crate::kept_host;
use crate::support::{shared_library, stub_host_library};

const ENDED: &str = "carried by an answer that has ended";

fn run_directory() -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("weivel-rust-carried-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the run directory is made");
    std::fs::write(
        directory.join("stub-host.json"),
        r#"{"mode": "serve", "plugins": [{"name": "alpha", "directory": "alpha",
            "manifest": "alpha/manifest.json", "entry": "alpha/entry", "scope": true,
            "settings": {"greeting": "hello"}}]}"#,
    )
    .expect("stub-host.json is written");
    directory
}

#[test]
fn a_scopes_definition_kept_across_the_seam_is_refused_once_the_scope_is_released_as_in_process() {
    let standard = Standard::load(shared_library("weivel_standard")).expect("the standard loads");
    let (host, _) = kept_host(
        HostScheduler::NoClock,
        HostOptions {
            description_check: Some(Arc::new(standard.descriptions().clone())),
            ..HostOptions::default()
        },
    );
    let declared = standard
        .read_library(&std::fs::read(stub_host_library().with_file_name("library.json")).unwrap())
        .expect("the stub host's declaration reads");
    let ids = standard.service_ids();
    let library = host
        .load_library_withholding(&stub_host_library(), &declared, &[&ids.host, &ids.scope])
        .expect("the stub host loads");
    let provider = HostProvider::of(library.described(&ids.host).unwrap());
    let directory = run_directory();
    let plan = provider
        .start(&[directory.to_string_lossy().into_owned()])
        .expect("start answers");
    let scope = Scopes::of(library.described(&ids.scope).unwrap())
        .open(&plan.plugins[0].manifest)
        .expect("the scope opens")
        .expect("the scope answers definitions");
    let config: Object = Arc::new(scope.config.clone().expect("the scope answers a config"));

    let entry = host
        .load_entry(
            &shared_library("weivel_test_plugins"),
            Some("config_keeping_entry"),
        )
        .expect("the plugin loads");
    let manifest = Manifest {
        import_name: "alpha".to_owned(),
        services_consumed: vec![ServiceWant {
            id: "weivel.config".to_owned(),
            provider: None,
            optional: Some(true),
            degradable: None,
            constraints: Vec::new(),
        }],
        ..Manifest::default()
    };
    let plugin = host.register(manifest, entry.clone()).unwrap();
    let _offer = host
        .offer_to_plugin(plugin, &ids.config, config.clone())
        .expect("the config is offered");
    let activation = host.activate(&[plugin], EdgeSource::SERVICES).unwrap();
    assert_eq!(activation.outcomes[0].wait().state, ActivationState::Active);
    let greeting = format!("{:?}", Some(Setting::Text("hello".to_owned())));
    assert_eq!(
        entry.actions().call("greeting", None),
        Some(Ok(Some(Tree::Text(greeting))))
    );

    scope.release();
    let in_process = Config::of(&config).unwrap().get("greeting");
    assert!(
        in_process
            .as_ref()
            .is_err_and(|refused| refused.is_callers_mistake()
                && refused
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains(ENDED))),
        "{in_process:?}"
    );
    let across = entry.actions().call("greeting", None);
    assert!(
        matches!(&across, Some(Err(failure)) if failure.contains(ENDED) && failure.contains("CallersMistake")),
        "{across:?}"
    );
    provider.started(&[]).ok();
    provider.stop().expect("stop answers");
    host.destroy().wait();
    let _ = std::fs::remove_dir_all(&directory);
}
