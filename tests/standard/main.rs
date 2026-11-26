//! The `standard` module's tests: the manifest loader, the library declaration reader and the service ids against the
//! pinned standard, and the typed services against the kit's `stub-host`, which serves a scope of each.

#[path = "../support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use weivel_rust::context::Object;
use weivel_rust::host::{BoundDefinition, FunctionKind, Host, HostOptions, LoadedLibrary};
use weivel_rust::standard::{
    Config, HostOutcome, HostProvider, Log, LogLevel, NextTick, Paths, Scopes, Secrets, Setting,
    Standard, host_modes, plugin_states,
};

fn standard() -> Standard {
    Standard::load(support::shared_library("weivel_standard")).expect("the pinned standard loads")
}

fn stub_host_file(name: &str) -> Vec<u8> {
    let path = support::stub_host_library().with_file_name(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{} reads: {error}", path.display()))
}

#[test]
fn the_service_ids_are_the_ones_the_standard_publishes() {
    let ids = standard().service_ids().clone();
    let read = [
        ids.config,
        ids.secrets,
        ids.paths,
        ids.log,
        ids.scope,
        ids.host,
    ];
    assert_eq!(
        read,
        [
            "weivel.config",
            "weivel.secrets",
            "weivel.paths",
            "weivel.log",
            "weivel.scope",
            "weivel.host"
        ]
    );
}

#[test]
fn a_declaration_reads_with_its_descriptions_and_its_contracts() {
    let standard = standard();
    let declared = standard
        .read_library(&stub_host_file("library.json"))
        .expect("the stub host's declaration reads");
    assert_eq!(declared.import_name, "stub-host");
    let services: Vec<(&str, Option<&str>, FunctionKind)> = declared
        .services
        .iter()
        .map(|service| {
            (
                service.id.as_str(),
                service.description.as_deref(),
                service.functions[0].kind,
            )
        })
        .collect();
    assert_eq!(
        services,
        [
            (
                "weivel.host",
                Some("stub_host_description"),
                FunctionKind::Definition
            ),
            (
                "weivel.scope",
                Some("stub_host_scopes_description"),
                FunctionKind::Definition
            ),
        ]
    );
    let contracted = standard
        .read_library(&stub_host_file("library-declares-a-contract.json"))
        .expect("the declaration of a contract reads");
    let contracts: Vec<(&str, &str)> = contracted
        .contracts
        .iter()
        .map(|contract| (contract.capability.as_str(), contract.description.as_str()))
        .collect();
    assert_eq!(
        contracts,
        [("stub-host.tally", "stub_host_tally_description")]
    );
}

#[test]
fn a_declaration_the_rules_refuse_is_answered_at_its_field() {
    let refused = standard()
        .read_library(br#"{"importName": "x", "version": "1.0.0", "services": [{"id": "x.y", "functions": []}]}"#)
        .expect_err("a service of no function is refused");
    assert!(refused.path.starts_with("services[0]"), "{refused:?}");
}

#[test]
fn a_manifest_loads_and_one_the_schema_refuses_is_answered_at_its_field() {
    let standard = standard();
    let loaded = standard
        .load_manifest(
            br#"{"api": 1, "importName": "alpha", "language": "rust", "entry": "entry",
                "services": {"consumes": ["weivel.log", {"id": "beta:feed", "optional": true}], "provides": ["alpha:accounts"]}}"#,
        )
        .expect("the manifest loads");
    assert_eq!(loaded.language, "rust");
    let manifest = loaded.manifest;
    assert_eq!(manifest.import_name, "alpha");
    assert_eq!(manifest.entry.as_deref(), Some("entry"));
    let consumed: Vec<(&str, Option<bool>)> = manifest
        .services_consumed
        .iter()
        .map(|want| (want.id.as_str(), want.optional))
        .collect();
    assert_eq!(consumed, [("weivel.log", None), ("beta:feed", Some(true))]);
    assert_eq!(manifest.services_provided[0].id, "alpha:accounts");
    let refused = standard
        .load_manifest(br#"{"api": "one", "importName": "alpha", "language": "rust"}"#)
        .expect_err("an api that is not a number is refused");
    assert_eq!(refused.path, "api", "{refused:?}");
}

#[test]
fn a_manifest_naming_no_language_or_one_outside_the_grammar_is_refused_at_its_field() {
    let standard = standard();
    let missing = standard
        .load_manifest(br#"{"api": 1, "importName": "alpha"}"#)
        .expect_err("a manifest without a language is refused");
    assert_eq!(missing.path, "language", "{missing:?}");
    let spaced = standard
        .load_manifest(br#"{"api": 1, "importName": "alpha", "language": "Type Script"}"#)
        .expect_err("a language outside the grammar is refused");
    assert_eq!(spaced.path, "language", "{spaced:?}");
    let named = standard
        .load_manifest(br#"{"api": 1, "importName": "alpha", "language": "c"}"#)
        .expect("a manifest naming another implementation loads");
    assert_eq!(named.language, "c");
}

fn run_directory() -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("weivel-rust-standard-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the run directory is made");
    std::fs::write(
        directory.join("stub-host.json"),
        r#"{"mode": "serve", "ticks": [40], "exit": 3, "plugins": [{"name": "alpha", "directory": "alpha",
            "manifest": "alpha/manifest.json", "entry": "alpha/entry", "scope": true,
            "settings": {"greeting": "hello", "count": 3, "on": true}}]}"#,
    )
    .expect("stub-host.json is written");
    directory
}

fn stub_host(host: &Host, standard: &Standard) -> LoadedLibrary {
    let declared = standard
        .read_library(&stub_host_file("library.json"))
        .expect("the stub host's declaration reads");
    let ids = standard.service_ids();
    host.load_library_withholding(
        &support::stub_host_library(),
        &declared,
        &[ids.host.as_str(), ids.scope.as_str()],
    )
    .expect("the stub host loads")
}

fn object(bound: &Option<BoundDefinition>) -> Object {
    Arc::new(bound.clone().expect("the scope answers the definition"))
}

fn record(directory: &Path) -> Vec<String> {
    std::fs::read_to_string(directory.join("record.txt"))
        .expect("the record reads")
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_typed_services_reach_a_scope_the_stub_host_opens() {
    let standard = standard();
    let host = Host::create(
        &support::core_binary(),
        HostOptions {
            description_check: Some(Arc::new(standard.descriptions().clone())),
            ..HostOptions::default()
        },
    )
    .expect("the host is created");
    let library = stub_host(&host, &standard);
    let ids = standard.service_ids();
    let provider = HostProvider::of(
        library
            .described(&ids.host)
            .expect("weivel.host is described"),
    );
    let directory = run_directory();
    let plan = provider
        .start(&[directory.to_string_lossy().into_owned(), "token".to_owned()])
        .expect("start answers the plan");
    assert_eq!(plan.mode, host_modes::SERVE);
    assert_eq!(plan.plugins.len(), 1);
    assert_eq!(plan.action_name, None);
    let scope = Scopes::of(
        library
            .described(&ids.scope)
            .expect("weivel.scope is described"),
    )
    .open(&plan.plugins[0].manifest)
    .expect("the scope opens")
    .expect("the scope answers definitions");

    let config = Config::of(&object(&scope.config)).expect("the config is bound");
    assert_eq!(
        config.get("greeting"),
        Ok(Some(Setting::Text("hello".to_owned())))
    );
    assert_eq!(config.get("count"), Ok(Some(Setting::Number(3.0))));
    assert_eq!(config.get("on"), Ok(Some(Setting::Boolean(true))));
    assert_eq!(config.get("seen"), Ok(None));
    config
        .set("seen", &Setting::Text("yes".to_owned()))
        .expect("a setting is stored");
    assert_eq!(
        config.get("seen"),
        Ok(Some(Setting::Text("yes".to_owned())))
    );
    let mut every = config.all().expect("every setting is answered");
    every.sort_by(|first, second| first.0.cmp(&second.0));
    assert_eq!(
        every,
        [
            ("count".to_owned(), Setting::Number(3.0)),
            ("greeting".to_owned(), Setting::Text("hello".to_owned())),
            ("on".to_owned(), Setting::Boolean(true)),
            ("seen".to_owned(), Setting::Text("yes".to_owned())),
        ]
    );
    let no_file = config.settings_file();
    assert!(
        matches!(&no_file, Ok(None))
            || no_file
                .as_ref()
                .is_err_and(|refused| refused.is_callers_mistake()),
        "{no_file:?}"
    );
    let not_offered = config.get_tree("greeting");
    assert!(
        not_offered
            .as_ref()
            .is_err_and(|refused| refused.is_callers_mistake()),
        "the stub host's config offers no get_tree, which is appended past its first shape: {not_offered:?}"
    );

    let paths = Paths::of(&object(&scope.paths)).expect("the paths are bound");
    let plugin = paths.plugin().expect("the plugin's directory is answered");
    assert!(plugin.ends_with("alpha"), "{plugin}");
    assert!(
        paths
            .cache()
            .expect("the cache is answered")
            .ends_with("cache")
    );
    assert!(
        paths
            .config()
            .expect("the config directory is answered")
            .ends_with("config")
    );

    let secrets = Secrets::of(&object(&scope.secrets)).expect("the secrets are bound");
    assert_eq!(secrets.get("token"), Ok(None));
    secrets
        .set("token", b"\x00\xffkept")
        .expect("a secret is stored");
    assert_eq!(secrets.get("token"), Ok(Some(b"\x00\xffkept".to_vec())));
    secrets.remove("token").expect("a secret is removed");
    assert_eq!(secrets.get("token"), Ok(None));

    let log = Log::of(&object(&scope.log)).expect("the log is bound");
    log.write(LogLevel::Info, "hello", None)
        .expect("an entry is written");

    scope.release();
    provider
        .started(&[HostOutcome {
            manifest: plan.plugins[0].manifest.clone(),
            state: plugin_states::ACTIVATED,
            detail: None,
        }])
        .expect("started answers OK");
    assert!(matches!(provider.tick(1_000), Ok(NextTick::At(1_040))));
    assert_eq!(provider.tick(1_040), Ok(NextTick::Never));
    assert_eq!(provider.stop(), Ok(3));
    host.destroy().wait();
    assert_eq!(
        record(&directory),
        [
            "start \"token\"",
            "open alpha scoped",
            "set alpha \"seen\" \"yes\"",
            "log alpha info \"hello\"",
            "started 1",
            "outcome alpha activated no-detail",
            "tick 1 first",
            "tick 2 on-time",
            "stop",
        ]
    );
    let _ = std::fs::remove_dir_all(&directory);
}
