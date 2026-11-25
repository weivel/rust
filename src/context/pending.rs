//! A result that comes later, on `std` alone: awaited as a [`Future`], waited for with [`Pending::wait`], or handed
//! to a callback, so any runtime and plain threads all use it.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::task::{Context as TaskContext, Poll, Waker};

type Callback<T> = Box<dyn FnOnce(&T) + Send>;

const WAITED_FOR_GOOD: &str = "a result was waited for on a thread that runs what settles it, so it would never settle; hand it a callback with on_settled instead";

enum Slot<T> {
    Waiting {
        callbacks: Vec<Callback<T>>,
        wakers: Vec<Waker>,
    },
    Settled(Arc<T>),
}

/// Whether waiting for a result on this thread now would wait for good, because what settles it cannot run until the
/// wait returns.
pub(crate) type WouldWaitForGood = Arc<dyn Fn() -> bool + Send + Sync>;

struct Shared<T> {
    slot: Mutex<Slot<T>>,
    settled: Condvar,
    would_wait_for_good: Option<WouldWaitForGood>,
}

impl<T> Shared<T> {
    fn lock(&self) -> MutexGuard<'_, Slot<T>> {
        self.slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A result that settles once. Cloning it shares the result.
pub struct Pending<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Clone for Pending<T> {
    fn clone(&self) -> Self {
        Pending {
            shared: self.shared.clone(),
        }
    }
}

impl<T> std::fmt::Debug for Pending<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(if self.is_settled() {
            "Pending(settled)"
        } else {
            "Pending(waiting)"
        })
    }
}

/// What settles a [`Pending`], once. Dropping it unused leaves the result waiting for good.
pub struct Settle<T> {
    shared: Arc<Shared<T>>,
}

/// A result to come and what settles it.
pub fn pending<T>() -> (Pending<T>, Settle<T>) {
    pending_unless(None)
}

/// A result to come that one of the crate's own steps settles, and what tells a wait that it would wait for good.
pub(crate) fn pending_settled_by<T>(
    would_wait_for_good: WouldWaitForGood,
) -> (Pending<T>, Settle<T>) {
    pending_unless(Some(would_wait_for_good))
}

fn pending_unless<T>(would_wait_for_good: Option<WouldWaitForGood>) -> (Pending<T>, Settle<T>) {
    let shared = Arc::new(Shared {
        slot: Mutex::new(Slot::Waiting {
            callbacks: Vec::new(),
            wakers: Vec::new(),
        }),
        settled: Condvar::new(),
        would_wait_for_good,
    });
    (
        Pending {
            shared: shared.clone(),
        },
        Settle { shared },
    )
}

impl<T> Pending<T> {
    /// A result settled already.
    pub fn settled(value: T) -> Pending<T> {
        let (result, settle) = pending();
        settle.settle(value);
        result
    }

    /// Whether the result has settled.
    pub fn is_settled(&self) -> bool {
        matches!(*self.shared.lock(), Slot::Settled(_))
    }

    /// Calls `callback` with the result once it settles, on the thread that settles it, or at once on this thread
    /// when it has settled already.
    pub fn on_settled(&self, callback: impl FnOnce(&T) + Send + 'static) {
        let mut slot = self.shared.lock();
        match &mut *slot {
            Slot::Waiting { callbacks, .. } => callbacks.push(Box::new(callback)),
            Slot::Settled(value) => {
                let value = value.clone();
                drop(slot);
                callback(&value);
            }
        }
    }
}

impl<T: Clone> Pending<T> {
    /// The result, once it has settled; `None` until then.
    pub fn value(&self) -> Option<T> {
        match &*self.shared.lock() {
            Slot::Settled(value) => Some(T::clone(value)),
            Slot::Waiting { .. } => None,
        }
    }

    /// Blocks this thread until the result settles.
    ///
    /// # Panics
    ///
    /// When the result is one of the crate's and is not settled, while what settles it cannot run until this wait
    /// returns: on a thread dispatching the core whose dispatch settles it, or for a release asked from inside the
    /// released plugin's own activate, which the release waits for. Such a caller hands the result a callback.
    /// A result the caller made with [`pending`] carries no such knowledge and is waited for as asked.
    pub fn wait(&self) -> T {
        let mut slot = self.shared.lock();
        loop {
            if let Slot::Settled(value) = &*slot {
                return T::clone(value);
            }
            if self
                .shared
                .would_wait_for_good
                .as_ref()
                .is_some_and(|would_wait_for_good| would_wait_for_good())
            {
                drop(slot);
                panic!("{WAITED_FOR_GOOD}");
            }
            slot = self
                .shared
                .settled
                .wait(slot)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

impl<T: Clone> Future for Pending<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, task: &mut TaskContext<'_>) -> Poll<T> {
        let mut slot = self.shared.lock();
        match &mut *slot {
            Slot::Settled(value) => Poll::Ready(T::clone(value)),
            Slot::Waiting { wakers, .. } => {
                if !wakers.iter().any(|waker| waker.will_wake(task.waker())) {
                    wakers.push(task.waker().clone());
                }
                Poll::Pending
            }
        }
    }
}

impl<T> Settle<T> {
    /// Settles the result with `value`, then calls every callback waiting on it, on this thread.
    pub fn settle(self, value: T) {
        let value = Arc::new(value);
        let waiting = std::mem::replace(&mut *self.shared.lock(), Slot::Settled(value.clone()));
        self.shared.settled.notify_all();
        let Slot::Waiting { callbacks, wakers } = waiting else {
            return;
        };
        wakers.into_iter().for_each(Waker::wake);
        for callback in callbacks {
            callback(&value);
        }
    }
}
