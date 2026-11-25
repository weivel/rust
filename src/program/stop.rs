//! A run's stop request, and the clock the program reads.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch, as the program's clock reads; 0 for a clock set before it.
pub(super) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// How a host program is asked to end a run: once requested, it stays requested. Cloning it shares it, so the one
/// who started a run and the run itself hold the same request.
#[derive(Clone, Default)]
pub struct StopRequest {
    shared: Arc<(Mutex<bool>, Condvar)>,
}

impl std::fmt::Debug for StopRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "StopRequest(requested: {})", self.is_requested())
    }
}

impl StopRequest {
    /// A request not yet made.
    pub fn new() -> StopRequest {
        StopRequest::default()
    }

    fn requested(&self) -> MutexGuard<'_, bool> {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Requests the stop; a later request changes nothing.
    pub fn request(&self) {
        *self.requested() = true;
        self.shared.1.notify_all();
    }

    /// Whether the stop is requested.
    pub fn is_requested(&self) -> bool {
        *self.requested()
    }

    /// Waits until the stop is requested.
    pub(super) fn wait(&self) {
        let mut requested = self.requested();
        while !*requested {
            requested = self
                .shared
                .1
                .wait(requested)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    /// Waits until the clock reads `at` or later, reading it again after every wait, or until the stop is requested.
    pub(super) fn wait_until(&self, at: i64) {
        let mut requested = self.requested();
        loop {
            let left = at.saturating_sub(now_millis());
            if *requested || left <= 0 {
                return;
            }
            requested = self
                .shared
                .1
                .wait_timeout(requested, Duration::from_millis(left as u64))
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }
}
