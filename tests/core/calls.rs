//! Every call of the core made once through the wrapper against the real core, and what each refuses.

use weivel_rust::core::{
    Answer, Capacity, Core, CoreOptions, EdgeSource, Handle, HandleCondition, Manifest,
    QuarantineReason, RefusalCode, RefusalStatus, ServiceOffer, ServiceWant, WantOptions,
};

use crate::support::recorded_core;

fn provider_manifest() -> Manifest {
    Manifest {
        import_name: "p".to_owned(),
        api: 1,
        services_provided: vec![ServiceOffer {
            id: "p:a".to_owned(),
            attributes: Vec::new(),
        }],
        capabilities_provided: vec!["cap.one".to_owned()],
        ..Manifest::default()
    }
}

fn consumer_manifest() -> Manifest {
    Manifest {
        import_name: "c".to_owned(),
        api: 1,
        services_consumed: vec![ServiceWant {
            id: "p:a".to_owned(),
            ..ServiceWant::default()
        }],
        capabilities_consumed: vec!["cap.one".to_owned()],
        ..Manifest::default()
    }
}

fn refusal_code(answer: Answer) -> RefusalCode {
    match answer {
        Answer::Refused {
            status: RefusalStatus::Refused,
            refusal: Some(refusal),
        } => refusal.code,
        other => panic!("expected the core's refusal, answered {other:?}"),
    }
}

fn plans_an_activation(core: &Core, provider: Handle, consumer: Handle) {
    let plan = core
        .plan_activation(&[consumer, provider], EdgeSource::SERVICES, None)
        .unwrap();
    assert_eq!(plan.order, vec![provider, consumer]);
    assert_eq!(plan.edges.len(), 1);
    assert_eq!(
        (plan.edges[0].provider, plan.edges[0].consumer),
        (provider, consumer)
    );
    assert!(plan.cycles.is_empty() && plan.refusals.is_empty());
}

fn opens_a_plugin_with_its_payload(core: &Core, plugin: Handle, token: u64) {
    core.open_plugin(plugin, token).unwrap();
    assert_eq!(core.plugin_payload(plugin).unwrap(), token);
}

fn provides_and_consumes_a_capability(core: &Core, provider: Handle, consumer: Handle) {
    let watch = core.watch_capability(consumer, "cap.one").unwrap();
    assert_eq!(core.handle_state(watch).unwrap(), HandleCondition::WATCHING);
    let implementation = core.provide_capability(provider, "cap.one").unwrap();
    core.verify_activation(provider).unwrap();
    let found = core.all_capabilities(consumer, "cap.one").unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(
        (found[0].implementation, found[0].provider),
        (implementation, provider)
    );
}

fn registers_and_consumes_a_service(core: &Core, provider: Handle, consumer: Handle) {
    assert_eq!(core.get_service(consumer, "p:a"), Err(Answer::Absent));
    let watch = core.watch_service(consumer, "p:a").unwrap();
    let registration = core.register_service(provider, "p:a").unwrap();
    assert_eq!(core.get_service(consumer, "p:a").unwrap(), registration);
    let want = core
        .want_service(consumer, "p:a", &WantOptions::default())
        .unwrap();
    assert_eq!(core.service_ids(consumer).unwrap(), vec!["p:a".to_owned()]);
    assert_eq!(core.unbound_services(consumer).unwrap().len(), 1);
    assert_eq!(
        core.report_capacity(want, registration, &Capacity::default(), 0),
        Err(Answer::Absent)
    );
    core.unregister(watch).unwrap();
    core.unregister(registration).unwrap();
}

fn reads_the_ledger(core: &Core, provider: Handle) {
    let snapshot = core.ledger().unwrap();
    assert_eq!(snapshot.rows.len(), 2);
    let row = core.ledger_entry(provider).unwrap();
    assert_eq!(row.import_name, "p");
    assert!(row.verified);
    assert_eq!(row.capabilities_provided, vec!["cap.one".to_owned()]);
}

fn tells_the_time(core: &Core) {
    assert_eq!(core.next_deadline(), Err(Answer::Absent));
    core.advance(1000).unwrap();
}

fn ends_a_plugin(core: &Core, provider: Handle, consumer: Handle) {
    let reason = QuarantineReason {
        detail: "its activation failed".to_owned(),
        fix: "fix the plugin".to_owned(),
        ..Default::default()
    };
    core.quarantine_plugin(consumer, Some(&reason)).unwrap();
    let refused = core.forget_plugin(provider).unwrap_err();
    let Answer::Refused {
        refusal: Some(refusal),
        ..
    } = refused
    else {
        panic!("a forget of an open plugin is refused")
    };
    assert_eq!(refusal.code, RefusalCode::PLUGIN_STATE);
    assert_eq!(refusal.subject.as_deref(), Some("p"));
    core.release_plugin(provider).unwrap();
    core.forget_plugin(provider).unwrap();
    assert_eq!(
        refusal_code(core.release_plugin(provider).unwrap_err()),
        RefusalCode::STALE_HANDLE
    );
}

#[test]
fn makes_every_call_of_the_core_through_the_wrapper() {
    let (core, _heard, told) = recorded_core(&CoreOptions::default());
    let support = core.supports(&provider_manifest()).unwrap();
    assert!(support.supported);
    assert_eq!(support.refusal.code, RefusalCode::NONE);
    let provider = core.register_manifest(&provider_manifest()).unwrap();
    let consumer = core.register_manifest(&consumer_manifest()).unwrap();
    plans_an_activation(&core, provider, consumer);
    opens_a_plugin_with_its_payload(&core, provider, 41);
    opens_a_plugin_with_its_payload(&core, consumer, 42);
    provides_and_consumes_a_capability(&core, provider, consumer);
    registers_and_consumes_a_service(&core, provider, consumer);
    reads_the_ledger(&core, provider);
    tells_the_time(&core);
    ends_a_plugin(&core, provider, consumer);
    assert!(told.all().is_empty(), "the sink was told {:?}", told.all());
}

#[test]
fn a_text_holding_a_nul_is_the_callers_mistake_and_reaches_no_core() {
    let (core, _heard, told) = recorded_core(&CoreOptions::default());
    let plugin = core.register_manifest(&provider_manifest()).unwrap();
    let drained = core.last_sequence();
    assert!(matches!(
        core.register_service(plugin, "p:\0a"),
        Err(Answer::Mistake(_))
    ));
    let mut manifest = consumer_manifest();
    manifest.permissions.push("read\0write".to_owned());
    assert!(matches!(
        core.register_manifest(&manifest),
        Err(Answer::Mistake(_))
    ));
    assert_eq!(
        core.last_sequence(),
        drained,
        "a mistaken call drained nothing"
    );
    assert!(told.all().is_empty());
}

#[test]
fn a_manifest_the_core_reads_as_malformed_is_the_callers_and_told_to_no_sink() {
    let (core, _heard, told) = recorded_core(&CoreOptions::default());
    let answered = core.register_manifest(&Manifest {
        import_name: String::new(),
        api: 1,
        ..Manifest::default()
    });
    let Err(Answer::Refused { status, refusal }) = answered else {
        panic!("an empty import name is refused: {answered:?}")
    };
    assert!(
        matches!(status, RefusalStatus::Malformed | RefusalStatus::Refused),
        "{status:?}"
    );
    assert!(refusal.is_some(), "the core queued a refusal for it");
    assert!(told.all().is_empty());
}

#[test]
fn creation_options_with_a_nul_are_the_callers_mistake() {
    let options = CoreOptions {
        applications: vec!["a\0b".to_owned()],
        ..CoreOptions::default()
    };
    let created = Core::create(&crate::support::core_binary(), &options, Default::default());
    assert!(matches!(created, Err(Answer::Mistake(_))));
}

#[test]
fn creation_options_the_core_reads_as_malformed_are_refused() {
    let options = CoreOptions {
        initial_event_capacity: u32::MAX,
        ..CoreOptions::default()
    };
    let created = Core::create(&crate::support::core_binary(), &options, Default::default());
    assert!(
        matches!(
            created,
            Err(Answer::Refused {
                status: RefusalStatus::Malformed,
                refusal: None
            })
        ),
        "{created:?}"
    );
}
