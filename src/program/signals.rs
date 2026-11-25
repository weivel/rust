//! The platform's terminate signals as a run's stop request: SIGINT and SIGTERM on POSIX systems, SIGINT and SIGBREAK
//! on Windows, each heard by a handler declared by hand that only sets a flag, which a thread of the program's reads.
//! POSIX signals are hooked with `signal`, not `sigaction`: `struct sigaction` is laid out differently on Linux and on
//! macOS, so a hand declaration of it would be one per system, while `signal` takes only the number and the handler,
//! and installs a handler that stays installed on both.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::stop::StopRequest;

/// Set by the handler, the one thing a signal handler may safely do.
static TERMINATE_ASKED: AtomicBool = AtomicBool::new(false);

const POLL: Duration = Duration::from_millis(20);

#[cfg(unix)]
mod platform {
    use std::ffi::c_int;

    const SIGINT: c_int = 2;
    const SIGTERM: c_int = 15;
    const SIG_ERR: usize = usize::MAX;

    unsafe extern "C" {
        /// `signal` of the C library std links on every POSIX system, its handler passed as an address.
        fn signal(signal_number: c_int, handler: usize) -> usize;
    }

    pub(super) extern "C" fn heard(_: c_int) {
        super::TERMINATE_ASKED.store(true, super::Ordering::SeqCst);
    }

    /// The signal a handler hears as a stop request, for a test to call the handler with.
    #[cfg(test)]
    pub(super) const STOPPING: c_int = SIGINT;

    #[cfg(test)]
    pub(super) fn hear(signal_number: c_int) -> bool {
        heard(signal_number);
        true
    }

    /// Hooks SIGINT and SIGTERM, answering whether both were hooked.
    pub(super) fn listen() -> bool {
        [SIGINT, SIGTERM].into_iter().all(|signal_number| {
            // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
            unsafe { signal(signal_number, heard as extern "C" fn(c_int) as usize) != SIG_ERR }
        })
    }
}

#[cfg(windows)]
mod platform {
    const CTRL_C_EVENT: u32 = 0;
    const CTRL_BREAK_EVENT: u32 = 1;

    type Handler = unsafe extern "system" fn(u32) -> i32;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<Handler>, add: i32) -> i32;
    }

    /// The console events a handler hears as a stop request, for a test to call the handler with.
    #[cfg(test)]
    pub(super) const STOPPING: u32 = CTRL_BREAK_EVENT;

    /// Calls the handler as the console would, answering whether it took the event.
    #[cfg(test)]
    pub(super) fn hear(kind: u32) -> bool {
        // SAFETY: the handler only stores to an atomic.
        unsafe { heard(kind) != 0 }
    }

    /// A console event the handler leaves to the next handler.
    #[cfg(test)]
    pub(super) const CLOSING: u32 = 2;

    unsafe extern "system" fn heard(kind: u32) -> i32 {
        if kind == CTRL_C_EVENT || kind == CTRL_BREAK_EVENT {
            super::TERMINATE_ASKED.store(true, super::Ordering::SeqCst);
            return 1;
        }
        0
    }

    /// Hooks Ctrl+C and Ctrl+Break, SIGINT and SIGBREAK, answering whether they were hooked.
    pub(super) fn listen() -> bool {
        // SAFETY: the handler only stores to an atomic, and stays valid for the life of the process.
        unsafe { SetConsoleCtrlHandler(Some(heard), 1) != 0 }
    }
}

/// Forwards the platform's terminate signals to a run's stop request until it is dropped.
pub(super) struct Forwarding {
    done: Arc<AtomicBool>,
}

impl Drop for Forwarding {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
    }
}

/// Makes the platform's terminate signals `stop`'s request while the answer lives; `None` when they could not be
/// hooked.
pub(super) fn forward_terminate_signals(stop: StopRequest) -> Option<Forwarding> {
    if !platform::listen() {
        return None;
    }
    let done = Arc::new(AtomicBool::new(false));
    let watching = done.clone();
    std::thread::Builder::new()
        .name("weivel-host-signals".to_owned())
        .spawn(move || {
            while !watching.load(Ordering::SeqCst) {
                if TERMINATE_ASKED.load(Ordering::SeqCst) {
                    stop.request();
                    return;
                }
                std::thread::sleep(POLL);
            }
        })
        .ok()?;
    Some(Forwarding { done })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminate_signal_heard_by_the_handler_becomes_the_runs_stop_request() {
        let stop = StopRequest::new();
        let forwarding = forward_terminate_signals(stop.clone()).expect("the signals are hooked");
        #[cfg(windows)]
        assert!(
            !platform::hear(platform::CLOSING),
            "closing the console is left to the next handler"
        );
        assert!(!stop.is_requested());
        assert!(platform::hear(platform::STOPPING));
        let asked = std::time::Instant::now();
        while !stop.is_requested() {
            assert!(
                asked.elapsed() < Duration::from_secs(5),
                "the stop was never requested"
            );
            std::thread::sleep(POLL);
        }
        drop(forwarding);
        TERMINATE_ASKED.store(false, Ordering::SeqCst);
    }
}
