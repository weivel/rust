//! Activation: the plan, each plugin opened and its activate called in plan order, and each settled activate
//! verified or quarantined.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, Weak};

use super::entry::{Activation, ActivationState, Finished, Outcome};
use super::state::{Host, HostShared, InFlight, UnderWayMark};
use crate::context::{Context, Pending, Verification};
use crate::core::{
    Answer, EdgeSource, FailureKind, Handle, QuarantineReason, RefusalCode, RefusalStatus,
    panic_text,
};

impl Host {
    /// Activates `plugins` with the plan the core draws from `edge_source`, every service of the host's own counted as
    /// provided by the host: starts each in plan order without waiting for one activate before the next, and returns
    /// once every activate has been started, with the plan and each plugin's outcome to come. Where no dispatch runs,
    /// this thread starts the plan as the dispatcher, so what an activate's own calls queue is dispatched once every
    /// activate of the plan has been called. A plan the core refuses is answered as that refusal.
    pub fn activate(
        &self,
        plugins: &[Handle],
        edge_source: EdgeSource,
    ) -> Result<Activation, Answer> {
        if self.shared.is_destroy_called() {
            return Err(Answer::Destroyed);
        }
        let host_provided: Vec<String> = self
            .shared
            .records()
            .host_services
            .keys()
            .cloned()
            .collect();
        let plan = self.shared.core.plan_activation(
            plugins,
            edge_source,
            (!host_provided.is_empty()).then_some(host_provided.as_slice()),
        )?;
        let outcomes = Arc::new(Mutex::new(Vec::with_capacity(plan.order.len())));
        let starting = || {
            for plugin in &plan.order {
                let outcome = HostShared::start(&self.shared, *plugin);
                outcomes
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(outcome);
            }
        };
        let mut starting = Some(starting);
        if !self.shared.core.run_as_the_dispatcher(|| {
            if let Some(starting) = starting.take() {
                starting();
            }
        }) && let Some(starting) = starting.take()
        {
            starting();
        }
        let outcomes = std::mem::take(
            &mut *outcomes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        Ok(Activation { plan, outcomes })
    }

    /// Verifies `plugin`'s activation as a settled activate is verified: one the host holds open is quarantined in the
    /// same hold of the layer lock when the core refuses, and one it does not is answered as the core answers it.
    pub fn verify(&self, plugin: Handle) -> Result<Outcome, Answer> {
        let context = {
            let records = self.shared.records();
            if records.destroy.is_some() {
                return Err(Answer::Destroyed);
            }
            let held_open = records.opened.contains(&plugin);
            records
                .plugins
                .get(&plugin)
                .and_then(|record| record.context.clone())
                .filter(|_| held_open)
        };
        let Some(context) = context else {
            self.shared.core.verify_activation(plugin)?;
            return Ok(answered_outcome(plugin, ActivationState::Active));
        };
        let verification = self.shared.contexts.verify(&context)?;
        Ok(outcome_of(plugin, verification))
    }
}

fn outcome_of(plugin: Handle, verification: Verification) -> Outcome {
    match verification {
        Verification::Verified => answered_outcome(plugin, ActivationState::Active),
        Verification::Quarantined(refusal) => Outcome {
            plugin,
            state: ActivationState::Quarantined,
            refusal,
        },
    }
}

fn answered_outcome(plugin: Handle, state: ActivationState) -> Outcome {
    Outcome {
        plugin,
        state,
        refusal: None,
    }
}

fn answered(plugin: Handle, state: ActivationState) -> Pending<Outcome> {
    Pending::settled(answered_outcome(plugin, state))
}

impl HostShared {
    /// Opens one plugin of the plan and calls its activate, answering its outcome to come. The destroy waits for the
    /// open from here until it has an outcome.
    fn start(shared: &Arc<HostShared>, plugin: Handle) -> Pending<Outcome> {
        let (context, _opening) = {
            let mut records = shared.records();
            if records.destroy.is_some() {
                return answered(plugin, ActivationState::Released);
            }
            let opening = UnderWayMark::opening(shared, &mut records);
            let Some(record) = records.plugins.get_mut(&plugin) else {
                drop(records);
                return shared.failed(
                    plugin,
                    "the core planned a plugin the host holds no record of",
                );
            };
            let context = match &record.context {
                Some(context) => Ok(context.clone()),
                None => shared
                    .contexts
                    .make(plugin, record.manifest.clone())
                    .inspect(|made| record.context = Some(made.clone())),
            };
            (context, opening)
        };
        let context = match context {
            Ok(context) => context,
            Err(answer) => {
                return shared.failed(
                    plugin,
                    &format!("the host could not build a plugin's context: {answer}"),
                );
            }
        };
        HostShared::open(shared, plugin, context)
    }

    fn open(shared: &Arc<HostShared>, plugin: Handle, context: Context) -> Pending<Outcome> {
        let mut held_open = false;
        match shared.contexts.open(&context, || held_open = true) {
            Ok(()) => HostShared::activated(shared, plugin, context),
            Err(Answer::Refused {
                status: RefusalStatus::Refused,
                refusal,
            }) => Pending::settled(Outcome {
                plugin,
                state: ActivationState::Refused,
                refusal,
            }),
            Err(Answer::Destroyed) => answered(plugin, ActivationState::Released),
            Err(other) => {
                let detail = format!(
                    "a plugin's open failed for a reason other than the core's refusal: {other}"
                );
                if held_open {
                    let _ = shared.contexts.release(&context);
                    let _ = shared.contexts.forget(&context);
                }
                shared.failed(plugin, &detail)
            }
        }
    }

    fn failed(&self, plugin: Handle, detail: &str) -> Pending<Outcome> {
        self.report(FailureKind::LayerDefect, detail.to_owned());
        answered(plugin, ActivationState::Failed)
    }

    /// Takes a plugin the core opened as activating and calls its activate, unless the destroy began meanwhile: it is
    /// then released at once without its activate being called. A release asked for while the activate runs waits for
    /// it to return, so no deactivate runs before its activate.
    fn activated(shared: &Arc<HostShared>, plugin: Handle, context: Context) -> Pending<Outcome> {
        let (outcome, settle) = shared.settled_by_its_dispatch();
        let (serial, entry) = {
            let mut records = shared.records();
            let destroying = records.destroy.is_some();
            records.last_serial += 1;
            let serial = records.last_serial;
            records.opened.retain(|opened| *opened != plugin);
            records.opened.push(plugin);
            let Some(record) = records.plugins.get_mut(&plugin) else {
                return answered(plugin, ActivationState::Failed);
            };
            record.activate_called = !destroying;
            if destroying {
                drop(records);
                HostShared::release_record(shared, plugin);
                return answered(plugin, ActivationState::Released);
            }
            record.in_flight = Some(InFlight {
                serial,
                settle: Some(settle),
            });
            record.activate_running = true;
            record.activating_thread = Some(std::thread::current().id());
            (serial, record.entry.clone())
        };
        let activating = catch_unwind(AssertUnwindSafe(|| entry.activate(&context)));
        match activating {
            Err(payload) => {
                HostShared::settle(shared, plugin, serial, Err(panic_text(&*payload)));
            }
            Ok(activating) => {
                let settling: Weak<HostShared> = Arc::downgrade(shared);
                activating.on_settled(move |finished| {
                    if let Some(shared) = settling.upgrade() {
                        HostShared::settle(&shared, plugin, serial, finished.clone());
                    }
                });
            }
        }
        if let Some(record) = shared.records().plugins.get_mut(&plugin) {
            record.activate_running = false;
            record.activating_thread = None;
        }
        HostShared::begin_a_waiting_release(shared, plugin);
        outcome
    }

    /// Verifies or quarantines a plugin whose activate settled, unless it was released or the destroy called since,
    /// and answers its outcome. A release asked for meanwhile waits for it, and the destroy waits for it too.
    fn settle(shared: &Arc<HostShared>, plugin: Handle, serial: u64, finished: Finished) {
        let (settle, context, _settling) = {
            let mut records = shared.records();
            if records.destroy.is_some() {
                return;
            }
            let Some(record) = records.plugins.get_mut(&plugin) else {
                return;
            };
            let Some(in_flight) = record.in_flight.as_mut().filter(|run| run.serial == serial)
            else {
                return;
            };
            let Some(settle) = in_flight.settle.take() else {
                return;
            };
            record.settling = true;
            let context = record.context.clone();
            let settling = UnderWayMark::settling(shared, &mut records);
            (settle, context, settling)
        };
        let outcome = match (context, finished) {
            (None, _) => answered_outcome(plugin, ActivationState::Failed),
            (Some(context), Ok(())) => shared.verified(plugin, &context),
            (Some(context), Err(failure)) => {
                let reason = QuarantineReason {
                    code: RefusalCode::NONE,
                    detail: format!("its activate failed: {}", failure.replace('\0', "\\0")),
                    fix: "make the plugin's activate complete normally".to_owned(),
                };
                answered_outcome(plugin, shared.quarantined(&context, &reason))
            }
        };
        if let Some(record) = shared.records().plugins.get_mut(&plugin) {
            record.settling = false;
            if record
                .in_flight
                .as_ref()
                .is_some_and(|run| run.serial == serial)
            {
                record.in_flight = None;
            }
        }
        settle.settle(outcome);
        HostShared::begin_a_waiting_release(shared, plugin);
    }

    fn verified(&self, plugin: Handle, context: &Context) -> Outcome {
        match self.contexts.verify(context) {
            Ok(verification) => outcome_of(plugin, verification),
            Err(answer) => {
                let reason = QuarantineReason {
                    code: RefusalCode::NONE,
                    detail: format!("its activation could not be verified: {answer}"),
                    fix: "release it and activate it again".to_owned(),
                };
                answered_outcome(plugin, self.quarantined(context, &reason))
            }
        }
    }

    fn quarantined(&self, context: &Context, reason: &QuarantineReason) -> ActivationState {
        match self.contexts.quarantine(context, Some(reason)) {
            Ok(()) => ActivationState::Quarantined,
            Err(answer) => {
                self.report(
                    FailureKind::LayerDefect,
                    format!("the core did not quarantine a plugin the host holds open: {answer}"),
                );
                ActivationState::Failed
            }
        }
    }
}
