//! Drives a `want` vector on the suite's own time: the consumer asks at 0, the provider registers at
//! `registerAtMillis`, and time runs to the horizon. The scheduler runs every task due on the way to the registration
//! unless `schedulerRunsBeforeRegistering` is false, when only the clock moves.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use super::observations::{Observer, kit_refusal};
use super::shapes::{self, DriverFailure, Members};
use super::world::{DriverWorld, InFlight, manifest};
use crate::context::{Object, WantLimits};
use crate::core::{Answer, CoreOptions, EdgeSource, RefusalCode};
use crate::standard::Json;
use crate::timer::Scheduler;

const CONSUMER: &str = "consumer";
const PROVIDER: &str = "provider";

/// A want vector's `given`, every part checked.
struct Given {
    service: String,
    register_at: Option<i64>,
    timeout: Option<i64>,
    horizon: i64,
    scheduler_runs_before_registering: bool,
}

impl Given {
    fn of(given: &Members) -> Result<Given, DriverFailure> {
        let place = "the vector's given";
        shapes::refuse_unknown_keys(
            place,
            given,
            &[
                "service",
                "registerAtMillis",
                "timeoutMillis",
                "horizonMillis",
                "schedulerRunsBeforeRegistering",
            ],
        )?;
        let service_place = format!("{place}'s service");
        let service = shapes::object_at(place, given, "service")?;
        shapes::refuse_unknown_keys(&service_place, service, &["id"])?;
        Ok(Given {
            service: shapes::text(&service_place, service, "id")?,
            register_at: shapes::number_or_null(place, given, "registerAtMillis")?,
            timeout: shapes::number_or_null(place, given, "timeoutMillis")?,
            horizon: shapes::number(place, given, "horizonMillis")?,
            scheduler_runs_before_registering: shapes::flag(
                place,
                given,
                "schedulerRunsBeforeRegistering",
                true,
            )?,
        })
    }
}

/// A want vector's core options: its service is well known, so the provider registers it under its own name.
pub(crate) fn core_options(given: &Members) -> Result<CoreOptions, DriverFailure> {
    Ok(CoreOptions {
        well_known_services: vec![Given::of(given)?.service],
        ..CoreOptions::default()
    })
}

#[derive(Default)]
struct Settled {
    served: bool,
    timed_out: bool,
    at_millis: Option<i64>,
    refusal: Option<Json>,
    unexpected: Option<String>,
}

pub(crate) struct WantRun {
    settled: Arc<Mutex<Settled>>,
}

fn locked(settled: &Mutex<Settled>) -> MutexGuard<'_, Settled> {
    settled
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn drive(given: &Members, world: &mut DriverWorld) -> Result<WantRun, DriverFailure> {
    let given = Given::of(given)?;
    let service = std::slice::from_ref(&given.service);
    world.register(
        manifest(CONSUMER, &[], service, &[], &[]),
        Arc::new(InFlight),
    )?;
    world.register(
        manifest(PROVIDER, service, &[], &[], &[]),
        Arc::new(InFlight),
    )?;
    world.open_in_flight(&world.names(), EdgeSource::SERVICES)?;
    let limits = given
        .timeout
        .map_or_else(WantLimits::unbounded, WantLimits::timeout);
    let run = WantRun {
        settled: Arc::new(Mutex::new(Settled::default())),
    };
    let settled = run.settled.clone();
    let time = world.time.clone();
    world
        .context(CONSUMER)?
        .want(&given.service, limits)
        .settlement()
        .on_settled(move |settlement| record(&mut locked(&settled), time.now(), settlement));
    if let Some(at) = given.register_at {
        register_at(&given, at, world)?;
    }
    world.time.run_until(given.horizon);
    if let Some(unexpected) = locked(&run.settled).unexpected.clone() {
        return Err(unexpected);
    }
    Ok(run)
}

fn record(settled: &mut Settled, now: i64, settlement: &Result<Object, Answer>) {
    settled.at_millis = Some(now);
    match settlement {
        Ok(_) => settled.served = true,
        Err(Answer::Refused {
            refusal: Some(refusal),
            ..
        }) => {
            settled.timed_out = refusal.code == RefusalCode::TIMED_OUT;
            settled.refusal = Some(kit_refusal(refusal));
        }
        Err(other) => {
            settled.unexpected = Some(format!(
                "the want settled with {other}, neither served nor refused"
            ));
        }
    }
}

fn register_at(given: &Given, at: i64, world: &DriverWorld) -> Result<(), DriverFailure> {
    if given.scheduler_runs_before_registering {
        world.time.run_until(at);
    } else {
        world.time.move_clock_to(at);
    }
    let service: Object = Arc::new(());
    world
        .context(PROVIDER)?
        .register(&given.service, service)
        .map(drop)
        .map_err(|answer| format!("the provider's registration answered {answer}"))
}

impl WantRun {
    pub(crate) fn observers<'a>(
        &'a self,
        world: &'a DriverWorld,
    ) -> BTreeMap<&'static str, Observer<'a>> {
        let mut observers: BTreeMap<&'static str, Observer<'a>> = BTreeMap::new();
        observers.insert(
            "settled",
            Box::new(|_| Ok(Json::Bool(locked(&self.settled).served))),
        );
        observers.insert(
            "timedOut",
            Box::new(|_| Ok(Json::Bool(locked(&self.settled).timed_out))),
        );
        observers.insert(
            "atMillis",
            Box::new(|_| {
                Ok(locked(&self.settled)
                    .at_millis
                    .map_or(Json::Null, |at| Json::Number(at as f64)))
            }),
        );
        observers.insert(
            "refusal",
            Box::new(|_| Ok(locked(&self.settled).refusal.clone().unwrap_or(Json::Null))),
        );
        observers.insert("timer", Box::new(move |_| Ok(world.timer_moves())));
        observers.insert(
            "fired",
            Box::new(move |_| {
                Ok(Json::Array(
                    world
                        .time
                        .fired()
                        .into_iter()
                        .map(|at| Json::Number(at as f64))
                        .collect(),
                ))
            }),
        );
        observers
    }
}
