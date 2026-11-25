//! C libraries as host services beyond what the layer vectors witness: descriptions checked by the standard, a
//! library of contracts alone, ids the host holds already, and bytes services after an unload.

use std::sync::{Arc, Mutex};

use weivel_rust::context::{ForeignObject, Object};
use weivel_rust::core::Answer;
use weivel_rust::host::{
    FunctionKind, Host, HostOptions, HostScheduler, LibraryAnswer, LibraryContract,
    LibraryDeclaration, LibraryFunction, LibraryService, LibraryServiceDeclaration,
};
use weivel_rust::standard::StandardDescriptions;

use crate::kept_host;
use crate::support::shared_library;

/// A process maps a library once however often it is opened, so these tests take turns with each other.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn function(name: &str, symbol: &str, kind: FunctionKind) -> LibraryFunction {
    LibraryFunction {
        name: name.to_owned(),
        symbol: symbol.to_owned(),
        kind,
        context: 0,
    }
}

fn definition_service(
    id: &str,
    symbol: &str,
    description: Option<&str>,
) -> LibraryServiceDeclaration {
    LibraryServiceDeclaration {
        id: id.to_owned(),
        functions: vec![function("definition", symbol, FunctionKind::Definition)],
        description: description.map(str::to_owned),
    }
}

fn declaration(import_name: &str, services: Vec<LibraryServiceDeclaration>) -> LibraryDeclaration {
    LibraryDeclaration {
        import_name: import_name.to_owned(),
        version: "1.0.0".to_owned(),
        release: None,
        services,
        contracts: Vec::new(),
    }
}

fn described_host() -> Host {
    let check = StandardDescriptions::load(shared_library("weivel_standard"))
        .expect("the standard's description checks load");
    kept_host(
        HostScheduler::NoClock,
        HostOptions {
            description_check: Some(Arc::new(check)),
            ..HostOptions::default()
        },
    )
    .0
}

fn described_declaration(walker_description: &str) -> LibraryDeclaration {
    declaration(
        "described",
        vec![
            definition_service(
                "described-mirror",
                "described_mirror_definition",
                Some("described_mirror_description"),
            ),
            definition_service(
                "described-walker",
                "described_walker_definition",
                Some(walker_description),
            ),
        ],
    )
}

#[test]
fn a_definition_whose_description_the_standard_passes_loads() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let host = described_host();
    let loaded = host
        .load_library(
            &shared_library("weivel_standard_described"),
            &described_declaration("described_walker_description"),
        )
        .expect("the described library loads");
    let services = loaded.services();
    assert!(
        services
            .iter()
            .all(|(_, object)| object.is::<ForeignObject>())
    );
    host.destroy().wait();
}

fn refused_load(declaration: &LibraryDeclaration) -> String {
    let host = described_host();
    let loaded = host.load_library(&shared_library("weivel_standard_described"), declaration);
    host.destroy().wait();
    match loaded {
        Err(Answer::Mistake(detail)) => detail,
        other => panic!("the load answered {other:?}"),
    }
}

#[test]
fn a_description_the_standards_check_refuses_fails_the_load() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let detail = refused_load(&described_declaration(
        "described_broken_definition_description",
    ));
    assert!(
        detail.contains("weivel_standard_description_check refused"),
        "{detail}"
    );
}

#[test]
fn a_description_answered_by_another_definition_fails_the_load() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let detail = refused_load(&declaration(
        "described",
        vec![definition_service(
            "described-mirror",
            "described_mirror_definition",
            Some("described_mirror_without_halve_description"),
        )],
    ));
    assert!(
        detail.contains("weivel_standard_description_answers refused"),
        "{detail}"
    );
}

#[test]
fn a_description_on_a_host_given_no_check_fails_the_load() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (host, _) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let loaded = host.load_library(
        &shared_library("weivel_standard_described"),
        &described_declaration("described_walker_description"),
    );
    assert!(matches!(loaded, Err(Answer::Mistake(_))), "{loaded:?}");
    host.destroy().wait();
}

#[test]
fn a_library_of_contracts_alone_loads_and_registers_nothing() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (host, _) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let mut contracts_only = declaration("described", Vec::new());
    contracts_only.contracts = vec![LibraryContract {
        capability: "described.mirroring".to_owned(),
        description: "described_mirror_description".to_owned(),
    }];
    let loaded = host
        .load_library(
            &shared_library("weivel_standard_described"),
            &contracts_only,
        )
        .expect("a library of contracts alone loads");
    assert!(loaded.services().is_empty());
    assert_eq!(loaded.declaration().contracts, contracts_only.contracts);
    assert_eq!(host.unload_library(&loaded), Ok(true));
    host.destroy().wait();
}

fn echo_bytes() -> LibraryDeclaration {
    LibraryDeclaration {
        release: Some("echo_release".to_owned()),
        ..declaration(
            "echo",
            vec![LibraryServiceDeclaration {
                id: "echo-bytes".to_owned(),
                functions: vec![function("echo", "echo_echo", FunctionKind::Bytes)],
                description: None,
            }],
        )
    }
}

#[test]
fn a_bytes_service_answers_in_place_and_nothing_once_unloaded() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (host, _) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let loaded = host
        .load_library(&shared_library("echo"), &echo_bytes())
        .expect("the echo library loads");
    let (_, object) = loaded.services().remove(0);
    let service = object
        .downcast_ref::<LibraryService>()
        .expect("a bytes service");
    match service.call("echo", b"kept") {
        LibraryAnswer::Returned(bytes) => assert_eq!(&*bytes, b"kept"),
        other => panic!("the call answered {other:?}"),
    }
    assert!(matches!(
        service.call("absent", b""),
        LibraryAnswer::NoSuchFunction
    ));
    assert_eq!(host.unload_library(&loaded), Ok(true));
    assert_eq!(host.unload_library(&loaded), Ok(false));
    assert!(matches!(
        service.call("echo", b"late"),
        LibraryAnswer::Unloaded
    ));
    host.destroy().wait();
}

#[test]
fn an_id_the_host_holds_already_refuses_the_load_and_the_host_service() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (host, _) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let own: Object = Arc::new(());
    let service = host.register_host_service("echo-bytes", own).unwrap();
    let refused = host.load_library(&shared_library("echo"), &echo_bytes());
    assert!(matches!(refused, Err(Answer::Mistake(_))), "{refused:?}");
    service.unregister().unwrap();
    let _loaded = host
        .load_library(&shared_library("echo"), &echo_bytes())
        .expect("the id is free again");
    let clashing = host.register_host_service("echo-bytes", Arc::new(()));
    assert!(matches!(clashing, Err(Answer::Mistake(_))), "{clashing:?}");
    host.destroy().wait();
}

#[test]
fn a_library_another_host_loaded_is_not_unloaded() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (loading, _) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let (other, _) = kept_host(HostScheduler::NoClock, HostOptions::default());
    let loaded = loading
        .load_library(&shared_library("echo"), &echo_bytes())
        .unwrap();
    assert!(matches!(
        other.unload_library(&loaded),
        Err(Answer::Mistake(_))
    ));
    loading.destroy().wait();
    other.destroy().wait();
}
