//! Release: the deactivate while the plugin is still whole, the core's release, its events dispatched to the plugin,
//! its entries ended, its forget, and its record dropped.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Weak};

use super::entry::{ActivationState, Finished, Outcome};
use super::state::{Host, HostShared, Records};
use crate::context::{Context, Pending, Settle, pending};
use crate::core::{Answer, FailureKind, Handle, panic_text};

/// What a release found to do once it began.
struct ReleaseStart {
    plugin: Handle,
    context: Context,
    deactivate: bool,
    settle: Settle<Result<bool, Answer>>,
}

impl Host {
    /// Releases a plugin the host opened, as the contract's release orders it. One whose activate is still running is
    /// released once that call has returned, and one whose settled activate is being verified once that has ended; a
    /// release that finds its activate in flight answers its outcome released as the release begins. The result
    /// settles once the release has run: true; false, doing nothing, for a plugin the host has not opened or has
    /// forgotten; [`Answer::Destroyed`] once the destroy has been called.
    pub fn release(&self, plugin: Handle) -> Pending<Result<bool, Answer>> {
        if self.shared.is_destroy_called() {
            return Pending::settled(Err(Answer::Destroyed));
        }
        HostShared::release_record(&self.shared, plugin)
    }
}

/// Takes what a release that may begin now needs out of the records, the plugin's outcome in flight among it.
fn take_the_start(
    records: &mut Records,
    plugin: Handle,
) -> Option<(ReleaseStart, Option<Settle<Outcome>>)> {
    let record = records.plugins.get_mut(&plugin)?;
    if record.is_busy() {
        return None;
    }
    let settle = record.waiting_release.take()?;
    let context = record.context.clone()?;
    let in_flight = record.in_flight.take().and_then(|run| run.settle);
    let start = ReleaseStart {
        plugin,
        context,
        deactivate: record.activate_called,
        settle,
    };
    records.opened.retain(|opened| *opened != plugin);
    Some((start, in_flight))
}

impl HostShared {
    /// Asks for the release of `plugin`, or answers the one under way. It begins at once unless the plugin's activate
    /// is running or settling, and then once neither is.
    pub(super) fn release_record(
        shared: &Arc<HostShared>,
        plugin: Handle,
    ) -> Pending<Result<bool, Answer>> {
        let (released, settle) = pending();
        {
            let mut records = shared.records();
            if !records.opened.contains(&plugin) {
                return Pending::settled(Ok(false));
            }
            let Some(record) = records.plugins.get_mut(&plugin) else {
                return Pending::settled(Ok(false));
            };
            if let Some(under_way) = &record.release {
                return under_way.clone();
            }
            if record.context.is_none() {
                return Pending::settled(Ok(false));
            }
            record.release = Some(released.clone());
            record.waiting_release = Some(settle);
            records.under_way.releases += 1;
        }
        HostShared::begin_a_waiting_release(shared, plugin);
        released
    }

    /// Begins the release asked for `plugin` when one waits and its activate neither runs nor settles.
    pub(super) fn begin_a_waiting_release(shared: &Arc<HostShared>, plugin: Handle) {
        let Some((start, in_flight)) = take_the_start(&mut shared.records(), plugin) else {
            return;
        };
        if let Some(in_flight) = in_flight {
            in_flight.settle(Outcome {
                plugin,
                state: ActivationState::Released,
                refusal: None,
            });
        }
        HostShared::deactivate_then_release(shared, start);
    }

    fn deactivate_then_release(shared: &Arc<HostShared>, start: ReleaseStart) {
        let entry = shared
            .records()
            .plugins
            .get(&start.plugin)
            .map(|record| record.entry.clone());
        let deactivating = match entry.filter(|_| start.deactivate) {
            None => None,
            Some(entry) => match catch_unwind(AssertUnwindSafe(|| entry.deactivate())) {
                Ok(deactivating) => deactivating,
                Err(payload) => {
                    shared.report_deactivate(&panic_text(&*payload));
                    None
                }
            },
        };
        let Some(deactivating) = deactivating else {
            shared.release_in_the_core(start);
            return;
        };
        let releasing: Weak<HostShared> = Arc::downgrade(shared);
        let mut start = Some(start);
        deactivating.on_settled(move |finished: &Finished| {
            let Some(shared) = releasing.upgrade() else {
                return;
            };
            if let Err(failure) = finished {
                shared.report_deactivate(failure);
            }
            if let Some(start) = start.take() {
                shared.release_in_the_core(start);
            }
        });
    }

    fn report_deactivate(&self, failure: &str) {
        self.report(
            FailureKind::PluginDeactivate,
            format!("a plugin's deactivate failed, and its release went on: {failure}"),
        );
    }

    /// Releases the plugin in the core, its entries ending once the release's events are dispatched, then forgets it
    /// and drops its record behind those events, so a release made inside a dispatch defers them too.
    fn release_in_the_core(self: &Arc<HostShared>, start: ReleaseStart) {
        if let Err(answer) = self.contexts.release(&start.context)
            && answer != Answer::Destroyed
        {
            self.report(
                FailureKind::LayerDefect,
                format!("the core did not release a plugin the host held open: {answer}"),
            );
        }
        let shared = self.clone();
        self.run_deferred(move || {
            let _ = shared.contexts.forget(&start.context);
            shared.records().plugins.remove(&start.plugin);
            start.settle.settle(Ok(true));
            let mut records = shared.records();
            records.under_way.releases = records.under_way.releases.saturating_sub(1);
            HostShared::end_the_destroy_when_quiet(&shared, records);
        });
    }
}
