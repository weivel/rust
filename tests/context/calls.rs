//! A context's calls: the provider's own object answered, an absence that is no refusal, a want settled with the
//! provider's object, a want needing a clock refused as a mistake, and a forgotten plugin's context answering
//! detached.

use std::sync::Arc;

use weivel_rust::context::{Context, Object, Pending, WantLimits, pending};
use weivel_rust::core::{Answer, EdgeSource, Handle, Manifest, ServiceOffer, ServiceWant};
use weivel_rust::host::{Entry, Finished, Host, HostOptions, HostScheduler};

use crate::support;

struct InFlight;

impl Entry for InFlight {
    fn activate(&self, _: &Context) -> Pending<Finished> {
        pending().0
    }
}

const SERVICE: &str = "calendar:events";

fn opened_pair() -> (Host, Context, Context, Handle) {
    let host = Host::create(
        &support::core_binary(),
        HostOptions {
            scheduler: HostScheduler::NoClock,
            ..HostOptions::default()
        },
    )
    .unwrap();
    let provider = host
        .register(
            Manifest {
                import_name: "calendar".to_owned(),
                services_provided: vec![ServiceOffer {
                    id: SERVICE.to_owned(),
                    attributes: Vec::new(),
                }],
                ..Manifest::default()
            },
            Arc::new(InFlight),
        )
        .unwrap();
    let consumer = host
        .register(
            Manifest {
                import_name: "agenda".to_owned(),
                services_consumed: vec![ServiceWant {
                    id: SERVICE.to_owned(),
                    provider: None,
                    optional: Some(true),
                    degradable: Some(false),
                    constraints: Vec::new(),
                }],
                ..Manifest::default()
            },
            Arc::new(InFlight),
        )
        .unwrap();
    host.activate(&[provider, consumer], EdgeSource::SERVICES)
        .unwrap();
    let providing = host.context(provider).unwrap();
    let consuming = host.context(consumer).unwrap();
    (host, providing, consuming, provider)
}

#[test]
fn a_get_answers_the_providers_own_object_and_an_absence_before() {
    let (host, provider, consumer, _) = opened_pair();
    assert_eq!(consumer.get(SERVICE).err(), Some(Answer::Absent));
    let service: Object = Arc::new(7u32);
    provider.register(SERVICE, service.clone()).unwrap();
    let got = consumer.get(SERVICE).unwrap();
    assert!(Arc::ptr_eq(&got, &service));
    host.destroy().wait();
}

#[test]
fn a_want_of_a_registered_service_settles_with_the_providers_own_object() {
    let (host, provider, consumer, _) = opened_pair();
    let service: Object = Arc::new("served");
    provider.register(SERVICE, service.clone()).unwrap();
    let want = consumer.want(SERVICE, WantLimits::unbounded());
    let settled = want
        .settlement()
        .value()
        .expect("a want made outside any dispatch is settled by the dispatch its call ends with");
    assert!(Arc::ptr_eq(&settled.unwrap(), &service));
    host.destroy().wait();
}

#[test]
fn a_want_that_needs_the_time_on_a_host_with_no_clock_is_the_callers_mistake() {
    let (host, _, consumer, _) = opened_pair();
    let settled = consumer
        .want(SERVICE, WantLimits::timeout(1000))
        .settlement()
        .wait();
    assert!(matches!(settled, Err(Answer::Mistake(_))), "{settled:?}");
    host.destroy().wait();
}

#[test]
fn a_released_plugins_context_answers_detached() {
    let (host, provider, _, provider_handle) = opened_pair();
    assert_eq!(host.release(provider_handle).wait(), Ok(true));
    assert!(provider.is_detached());
    let service: Object = Arc::new(());
    assert_eq!(
        provider.register(SERVICE, service).err(),
        Some(Answer::Detached)
    );
    assert!(host.manifest(provider_handle).is_none());
    host.destroy().wait();
}
