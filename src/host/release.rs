//! Release: the deactivate while the plugin is still whole, the core's release, its events dispatched to the plugin,
//! its entries ended, its forget, and its record dropped.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Weak};

use super::entry::{ActivationState, Finished, Outcome};
use super::state::{Host, HostShared};
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
    /// released the same way, its outcome answered released as the release begins. The result settles once the
    /// release has run: true; false, doing nothing, for a plugin the host has not opened or has forgotten;
    /// [`Answer::Destroyed`] once the destroy has been called.
    pub fn release(&self, plugin: Handle) -> Pending<Result<bool, Answer>> {
        if self.shared.is_destroy_called() {
            return Pending::settled(Err(Answer::Destroyed));
        }
        HostShared::release_record(&self.shared, plugin)
    }
}

impl HostShared {
    /// Begins the release of `plugin`, or answers the one under way.
    pub(super) fn release_record(
        shared: &Arc<HostShared>,
        plugin: Handle,
    ) -> Pending<Result<bool, Answer>> {
        let (released, settle) = pending();
        let (start, in_flight) = {
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
            let Some(context) = record.context.clone() else {
                return Pending::settled(Ok(false));
            };
            record.release = Some(released.clone());
            let in_flight = record.in_flight.take().and_then(|run| run.settle);
            let start = ReleaseStart {
                plugin,
                context,
                deactivate: record.activate_called,
                settle,
            };
            records.opened.retain(|opened| *opened != plugin);
            (start, in_flight)
        };
        if let Some(in_flight) = in_flight {
            in_flight.settle(Outcome {
                plugin,
                state: ActivationState::Released,
                refusal: None,
            });
        }
        HostShared::deactivate_then_release(shared, start);
        released
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
        });
    }
}
