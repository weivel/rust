//! Activation: the plan, each plugin opened and its activate called in plan order, and each settled activate
//! verified or quarantined.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, Weak};

use super::entry::{Activation, ActivationState, Finished, Outcome};
use super::state::{Host, HostShared, InFlight};
use crate::context::{Context, Pending, Verification, pending};
use crate::core::{
    Answer, EdgeSource, FailureKind, Handle, QuarantineReason, RefusalCode, RefusalStatus,
    panic_text,
};

impl Host {
    /// Activates `plugins` with the plan the core draws from `edge_source`: starts each in plan order without waiting
    /// for one activate before the next, and returns once every activate has been started, with the plan and each
    /// plugin's outcome to come. Where no dispatch runs, this thread starts the plan as the dispatcher, so what an
    /// activate's own calls queue is dispatched once every activate of the plan has been called. A plan the core
    /// refuses is answered as that refusal.
    pub fn activate(
        &self,
        plugins: &[Handle],
        edge_source: EdgeSource,
    ) -> Result<Activation, Answer> {
        if self.shared.is_destroy_called() {
            return Err(Answer::Destroyed);
        }
        let plan = self
            .shared
            .core
            .plan_activation(plugins, edge_source, None)?;
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
            return Ok(active(plugin));
        };
        let verification = self.shared.contexts.verify(&context)?;
        Ok(outcome_of(plugin, verification))
    }
}

fn active(plugin: Handle) -> Outcome {
    Outcome {
        plugin,
        state: ActivationState::Active,
        refusal: None,
    }
}

fn outcome_of(plugin: Handle, verification: Verification) -> Outcome {
    match verification {
        Verification::Verified => active(plugin),
        Verification::Quarantined(refusal) => Outcome {
            plugin,
            state: ActivationState::Quarantined,
            refusal,
        },
    }
}

fn answered(plugin: Handle, state: ActivationState) -> Pending<Outcome> {
    Pending::settled(Outcome {
        plugin,
        state,
        refusal: None,
    })
}

impl HostShared {
    /// Opens one plugin of the plan and calls its activate, answering its outcome to come.
    fn start(shared: &Arc<HostShared>, plugin: Handle) -> Pending<Outcome> {
        let context = {
            let mut records = shared.records();
            if records.destroy.is_some() {
                return answered(plugin, ActivationState::Released);
            }
            let Some(record) = records.plugins.get_mut(&plugin) else {
                drop(records);
                return shared.failed(
                    plugin,
                    "the core planned a plugin the host holds no record of",
                );
            };
            match &record.context {
                Some(context) => Ok(context.clone()),
                None => shared
                    .contexts
                    .make(plugin, record.manifest.clone())
                    .inspect(|made| record.context = Some(made.clone())),
            }
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
    /// then released at once without its activate being called.
    fn activated(shared: &Arc<HostShared>, plugin: Handle, context: Context) -> Pending<Outcome> {
        let (outcome, settle) = pending();
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
            if destroying {
                drop(records);
                HostShared::release_record(shared, plugin);
                return answered(plugin, ActivationState::Released);
            }
            record.in_flight = Some(InFlight {
                serial,
                settle: Some(settle),
            });
            record.activate_called = true;
            (serial, record.entry.clone())
        };
        let activating = catch_unwind(AssertUnwindSafe(|| entry.activate(&context)));
        match activating {
            Err(payload) => {
                shared.settle(plugin, serial, Err(panic_text(&*payload)));
            }
            Ok(activating) => {
                let settling: Weak<HostShared> = Arc::downgrade(shared);
                activating.on_settled(move |finished| {
                    if let Some(shared) = settling.upgrade() {
                        shared.settle(plugin, serial, finished.clone());
                    }
                });
            }
        }
        outcome
    }

    /// Verifies or quarantines a plugin whose activate settled, unless it was released or the destroy called since,
    /// and answers its outcome.
    fn settle(&self, plugin: Handle, serial: u64, finished: Finished) {
        let (settle, context) = {
            let mut records = self.records();
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
            (settle, record.context.clone())
        };
        let outcome = match (context, finished) {
            (None, _) => answered_outcome(plugin, ActivationState::Failed),
            (Some(context), Ok(())) => self.verified(plugin, &context),
            (Some(context), Err(failure)) => {
                let reason = QuarantineReason {
                    code: RefusalCode::NONE,
                    detail: format!("its activate failed: {}", failure.replace('\0', "\\0")),
                    fix: "make the plugin's activate complete normally".to_owned(),
                };
                answered_outcome(plugin, self.quarantined(&context, &reason))
            }
        };
        if let Some(record) = self.records().plugins.get_mut(&plugin)
            && record
                .in_flight
                .as_ref()
                .is_some_and(|run| run.serial == serial)
        {
            record.in_flight = None;
        }
        settle.settle(outcome);
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

fn answered_outcome(plugin: Handle, state: ActivationState) -> Outcome {
    Outcome {
        plugin,
        state,
        refusal: None,
    }
}
