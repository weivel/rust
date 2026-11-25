//! The one wrapper every call into the core goes through: the lock held across a call and its drain, the refusal a
//! call answers, the queue its events are dispatched from, the error sink, and the deadline hooks.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, MutexGuard};

use super::abi;
use super::answer::{Answer, RefusalStatus};
use super::event::{self, Event, EventPayload};
use super::marshal::Arena;
use super::table::{CoreBinary, CoreFunctions};
use super::values::{CoreOptions, Refusal};

/// What hears every event a core's wrapper dispatches, in the order the core queued them.
pub type Listener = Arc<dyn Fn(&Core, &Event) + Send + Sync>;

/// A step run on the dispatch queue, after every event drained before it.
pub type Step = Box<dyn FnOnce(&Core) + Send>;

/// What kind of failure the error sink is told of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// A listener panicked while it heard an event.
    Listener,
    /// A deferred step panicked.
    Step,
    /// The deadline hook's `follow` panicked.
    DeadlineHook,
    /// The deadline hook's clock panicked, so a due deadline was not handed to the core before the call.
    Clock,
    /// An event's texts could not all be read, so it is heard with fixed sentences in their place.
    EventIncomplete,
    /// The core answered the crate in a way only a defect of the crate explains.
    LayerDefect,
    /// The core did not take the time of a deadline that fell due, which stays due.
    DueAdvance,
}

/// One failure in code the wrapper called, which never reaches the caller of the call that set it off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// What failed.
    pub kind: FailureKind,
    /// What happened, in English.
    pub text: String,
    /// The sequence of the event concerned, when one is.
    pub sequence: Option<u64>,
}

/// Where failures in code the wrapper calls are told. A sink that panics is reported to standard error.
pub trait ErrorSink: Send + Sync {
    /// Hears one failure.
    fn report(&self, failure: &Failure);
}

/// The sink a core is given when its caller gives none: one line on standard error per failure.
struct StandardErrorSink;

impl ErrorSink for StandardErrorSink {
    fn report(&self, failure: &Failure) {
        eprintln!("weivel: {} (failure kind {:?})", failure.text, failure.kind);
    }
}

/// The clock and the follower a timer gives the wrapper: `now` is read before a call while a deadline is announced,
/// so a deadline that has fallen due reaches the core first, and `follow` is told each announcement once the lock is
/// let go, before any event of the call that drained it is dispatched.
#[derive(Clone)]
pub struct DeadlineHooks {
    /// The caller's time now, in its own milliseconds.
    pub now: Arc<dyn Fn() -> i64 + Send + Sync>,
    /// Hears the latest deadline, `None` when nothing waits on time.
    pub follow: Arc<dyn Fn(Option<i64>) + Send + Sync>,
}

/// What a core is wrapped with besides the core's own options.
#[derive(Clone, Default)]
pub struct Wrapping {
    /// The listener that hears the events the core's creation queued, and every one after.
    pub listener: Option<Listener>,
    /// Where failures are told; standard error when `None`.
    pub sink: Option<Arc<dyn ErrorSink>>,
}

/// A listener's place among a core's listeners, to remove it by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListenerRegistration(u64);

/// The deadline hooks' place in a core, to remove them by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HooksRegistration(u64);

/// Whether a call hands the core only what the crate built, so a malformed answer is the crate's defect, or carries
/// something its caller built, so a malformed answer is the caller's.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Parts {
    BuiltByTheLayer,
    CarriesTheCallers,
}

#[derive(Clone, Copy)]
pub(crate) struct CorePointer(pub(crate) *mut c_void);

// SAFETY: the core serialises every call on a lock of its own, from any thread.
unsafe impl Send for CorePointer {}

enum Queued {
    Event { event: Arc<Event>, fence: u64 },
    Step(Step),
}

struct InstalledHooks {
    registration: u64,
    hooks: DeadlineHooks,
}

/// Everything the layer lock guards. It is held across a call and its drain and never while anything of the
/// caller's runs. `dispatching` is true exactly while one thread delivers the queue.
///
/// `deadline` is the latest announcement drained, and `deadline_serial` counts every reason to tell `follow` again.
/// `following` is true while one thread calls `follow`, which goes on until `followed_serial` has caught up. Every
/// event drained along with an announcement is fenced with the serial it left, and the dispatch stops at the first
/// event whose fence `followed_serial` has not reached.
struct State {
    core: Option<CorePointer>,
    listeners: Vec<(u64, Listener)>,
    last_registration: u64,
    queue: VecDeque<Queued>,
    dispatching: bool,
    sequence: u64,
    deadline: Option<i64>,
    hooks: Option<InstalledHooks>,
    following: bool,
    deadline_serial: u64,
    followed_serial: u64,
}

struct Shared {
    binary: CoreBinary,
    sink: Arc<dyn ErrorSink>,
    state: Mutex<State>,
}

/// A core, created through a loaded [`CoreBinary`] and reached only through its wrapper. Cloning it shares the core.
#[derive(Clone)]
pub struct Core {
    shared: Arc<Shared>,
}

impl fmt::Debug for Core {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Core")
    }
}

type Guard<'a> = MutexGuard<'a, State>;

struct FollowingTurn {
    follow: Arc<dyn Fn(Option<i64>) + Send + Sync>,
    deadline: Option<i64>,
    serial: u64,
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic with no text".to_owned())
}

fn failure(kind: FailureKind, text: impl Into<String>, sequence: Option<u64>) -> Failure {
    Failure {
        kind,
        text: text.into(),
        sequence,
    }
}

fn defect(notes: &mut Vec<Failure>, text: String) -> Answer {
    notes.push(failure(FailureKind::LayerDefect, text.clone(), None));
    Answer::Defect(text)
}

impl State {
    fn first_is_fenced(&self) -> bool {
        matches!(self.queue.front(), Some(Queued::Event { fence, .. }) if *fence > self.followed_serial)
    }

    fn claim_the_dispatch(&mut self) -> bool {
        if self.queue.is_empty() || self.dispatching || self.first_is_fenced() {
            return false;
        }
        self.dispatching = true;
        true
    }

    fn take_following_turn(&mut self) -> Option<FollowingTurn> {
        if self.following || self.followed_serial == self.deadline_serial {
            return None;
        }
        let Some(installed) = &self.hooks else {
            self.followed_serial = self.deadline_serial;
            return None;
        };
        self.following = true;
        Some(FollowingTurn {
            follow: installed.hooks.follow.clone(),
            deadline: self.deadline,
            serial: self.deadline_serial,
        })
    }

    fn fence_from(&mut self, first_drained: usize) {
        let serial = self.deadline_serial;
        for queued in self.queue.iter_mut().skip(first_drained) {
            if let Queued::Event { fence, .. } = queued {
                *fence = serial;
            }
        }
    }

    fn next_listener(&self, after: u64) -> Option<(u64, Listener)> {
        self.core?;
        self.listeners
            .iter()
            .find(|(registration, _)| *registration > after)
            .cloned()
    }

    fn add_listener(&mut self, listener: Listener) -> ListenerRegistration {
        self.last_registration += 1;
        self.listeners.push((self.last_registration, listener));
        ListenerRegistration(self.last_registration)
    }
}

impl Core {
    /// Creates a core through `binary`, then drains and dispatches what its creation queued, as a call does, so only
    /// `wrapping`'s listener hears it.
    pub fn create(
        binary: &CoreBinary,
        options: &CoreOptions,
        wrapping: Wrapping,
    ) -> Result<Core, Answer> {
        let mut arena = Arena::default();
        let options = arena.core_options(options)?;
        let shared = Arc::new(Shared {
            binary: binary.clone(),
            sink: wrapping.sink.unwrap_or_else(|| Arc::new(StandardErrorSink)),
            state: Mutex::new(State {
                core: None,
                listeners: Vec::new(),
                last_registration: 0,
                queue: VecDeque::new(),
                dispatching: false,
                sequence: 0,
                deadline: None,
                hooks: None,
                following: false,
                deadline_serial: 0,
                followed_serial: 0,
            }),
        });
        let core = Core { shared };
        core.start(&options, wrapping.listener)?;
        Ok(core)
    }

    fn start(&self, options: &abi::CoreOptions, listener: Option<Listener>) -> Result<(), Answer> {
        let mut pointer = std::ptr::null_mut();
        let mut waiting = 0u32;
        // SAFETY: `options` and the texts it points into live until the call returns.
        let status =
            unsafe { (self.functions().weivel_core_create)(options, &mut pointer, &mut waiting) };
        match status {
            abi::STATUS_OK => {}
            abi::STATUS_OUT_OF_MEMORY => {
                return Err(Answer::Refused {
                    status: RefusalStatus::OutOfMemory,
                    refusal: None,
                });
            }
            _ => {
                return Err(Answer::Refused {
                    status: RefusalStatus::Malformed,
                    refusal: None,
                });
            }
        }
        let mut notes = Vec::new();
        let mut state = self.lock();
        state.core = Some(CorePointer(pointer));
        if let Some(listener) = listener {
            state.add_listener(listener);
        }
        self.drain(&mut state, waiting, &mut notes);
        self.leave(state, notes);
        Ok(())
    }

    pub(crate) fn functions(&self) -> &CoreFunctions {
        &self.shared.binary.loaded.functions
    }

    fn lock(&self) -> Guard<'_> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn enter(&self) -> Result<Guard<'_>, Answer> {
        let state = self.lock();
        if state.core.is_none() {
            return Err(Answer::Destroyed);
        }
        Ok(state)
    }

    /// Tells the sink, and standard error when the sink itself panics.
    pub(crate) fn report(&self, told: &Failure) {
        let sink = &self.shared.sink;
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| sink.report(told))) {
            eprintln!(
                "weivel: the error sink panicked ({}) and so was not told: {}",
                panic_text(&*payload),
                told.text
            );
        }
    }

    fn tell(&self, notes: Vec<Failure>) {
        for told in &notes {
            self.report(told);
        }
    }

    /// Makes one call of the core through the wrapper: a due deadline handed over first, the call, its drain, and the
    /// answer read from its status and the refusal it queued.
    pub(crate) fn invoke(
        &self,
        parts: Parts,
        call: impl FnOnce(&CoreFunctions, *mut c_void, &mut u32) -> i32,
    ) -> Result<(), Answer> {
        let state = self.enter()?;
        let state = self.hand_over_a_due_deadline(state)?;
        self.invoke_entered(state, parts, call)
    }

    /// As [`Core::invoke`], for the one call that hands the core the time itself, so it checks no deadline first.
    pub(crate) fn invoke_handing_the_time(
        &self,
        call: impl FnOnce(&CoreFunctions, *mut c_void, &mut u32) -> i32,
    ) -> Result<(), Answer> {
        let state = self.enter()?;
        self.invoke_entered(state, Parts::CarriesTheCallers, call)
    }

    fn invoke_entered(
        &self,
        mut state: Guard<'_>,
        parts: Parts,
        call: impl FnOnce(&CoreFunctions, *mut c_void, &mut u32) -> i32,
    ) -> Result<(), Answer> {
        let pointer = state.core.expect("an entered core is not destroyed").0;
        let mut notes = Vec::new();
        let mut waiting = 0u32;
        let status = call(self.functions(), pointer, &mut waiting);
        let own_refusal = self.drain(&mut state, waiting, &mut notes);
        let answer = answer_of(status, parts, own_refusal, &mut notes);
        self.leave(state, notes);
        answer
    }

    /// Drains a call's events, fencing them all when one is a deadline announcement, and answers the first refusal
    /// among them, which is the call's own.
    fn drain(
        &self,
        state: &mut State,
        mut waiting: u32,
        notes: &mut Vec<Failure>,
    ) -> Option<Refusal> {
        let first_drained = state.queue.len();
        let serial = state.deadline_serial;
        let mut own_refusal = None;
        while waiting > 0 && self.drain_one(state, &mut waiting, notes, &mut own_refusal) {}
        if state.deadline_serial != serial {
            state.fence_from(first_drained);
        }
        own_refusal
    }

    /// Drains one event: copied, recorded when it is a deadline, and released whatever the copy did.
    fn drain_one(
        &self,
        state: &mut State,
        waiting: &mut u32,
        notes: &mut Vec<Failure>,
        own_refusal: &mut Option<Refusal>,
    ) -> bool {
        let Some(CorePointer(pointer)) = state.core else {
            return false;
        };
        let mut buffer = abi::Event::empty();
        // SAFETY: `pointer` is a live core and `buffer` an event of the crate's size.
        let status = unsafe { (self.functions().weivel_next_event)(pointer, &mut buffer, waiting) };
        if status == abi::STATUS_NONE {
            return false;
        }
        if status != abi::STATUS_OK {
            defect(notes, "the core refused the layer's event struct, so the rest of its queue was not drained".to_owned());
            return false;
        }
        state.sequence += 1;
        // SAFETY: the event was just filled and is released only below.
        let copied = unsafe { event::copy(&buffer, state.sequence) };
        // SAFETY: the event was filled by the core and is released exactly once.
        if unsafe { (self.functions().weivel_event_release)(&mut buffer) } != abi::STATUS_OK {
            defect(
                notes,
                "the core refused to release an event the layer drained".to_owned(),
            );
        }
        if let Some(copied) = copied {
            self.keep(state, copied, notes, own_refusal);
        }
        true
    }

    fn keep(
        &self,
        state: &mut State,
        copied: Event,
        notes: &mut Vec<Failure>,
        own_refusal: &mut Option<Refusal>,
    ) {
        if let EventPayload::Deadline(deadline) = copied.payload {
            state.deadline = deadline;
            state.deadline_serial += 1;
        }
        if copied.incomplete {
            notes.push(failure(
                FailureKind::EventIncomplete,
                "an event's texts could not all be read, so it is heard with fixed sentences in their place",
                Some(copied.sequence),
            ));
        }
        if own_refusal.is_none() {
            *own_refusal = copied.own_refusal().cloned();
        }
        state.queue.push_back(Queued::Event {
            event: Arc::new(copied),
            fence: 0,
        });
    }

    /// Tells `follow` of a deadline this hold drained, lets the lock go, tells the sink what the call met, and
    /// dispatches unless a dispatch already runs.
    fn leave<'a>(&'a self, mut state: Guard<'a>, notes: Vec<Failure>) {
        if let Some(turn) = state.take_following_turn() {
            state = self.follow_the_deadline(state, turn);
        }
        let claimed = state.claim_the_dispatch();
        drop(state);
        self.tell(notes);
        if claimed {
            self.dispatch();
        }
    }

    /// Tells `follow` the latest announcement outside the lock, again for as long as a newer one was drained
    /// meanwhile; only the thread holding the turn is here, so two calls of `follow` never overlap.
    fn follow_the_deadline<'a>(
        &'a self,
        mut state: Guard<'a>,
        mut turn: FollowingTurn,
    ) -> Guard<'a> {
        loop {
            drop(state);
            let follow = &turn.follow;
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| follow(turn.deadline))) {
                self.report(&failure(
                    FailureKind::DeadlineHook,
                    format!(
                        "the deadline hook panicked for an announced deadline: {}",
                        panic_text(&*payload)
                    ),
                    None,
                ));
            }
            state = self.lock();
            state.followed_serial = turn.serial;
            state.following = false;
            match state.take_following_turn() {
                Some(next) => turn = next,
                None => return state,
            }
        }
    }

    fn take_next(&self) -> Option<Queued> {
        let mut state = self.lock();
        let next = if state.first_is_fenced() {
            None
        } else {
            state.queue.pop_front()
        };
        if next.is_none() {
            state.dispatching = false;
        }
        next
    }

    /// Delivers the queue until it is empty or reaches a fence. Only the thread that claimed the dispatch is here.
    fn dispatch(&self) {
        while let Some(queued) = self.take_next() {
            match queued {
                Queued::Step(step) => self.run_step(step),
                Queued::Event { event, .. } => self.hear(&event),
            }
        }
    }

    fn run_step(&self, step: Step) {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| step(self))) {
            self.report(&failure(
                FailureKind::Step,
                format!("a deferred step panicked: {}", panic_text(&*payload)),
                None,
            ));
        }
    }

    /// Hands an event to each listener in turn, finding the next under the lock each time, so one removed meanwhile
    /// hears no more and one added meanwhile hears this event.
    fn hear(&self, event: &Event) {
        let mut heard = 0;
        loop {
            let next = self.lock().next_listener(heard);
            let Some((registration, listener)) = next else {
                return;
            };
            heard = registration;
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| listener(self, event))) {
                self.report(&failure(
                    FailureKind::Listener,
                    format!(
                        "a listener panicked for an event: {}",
                        panic_text(&*payload)
                    ),
                    Some(event.sequence),
                ));
            }
        }
    }

    /// Hands the core a deadline that has fallen due: the clock read outside the lock while a deadline is announced,
    /// and when it has reached the deadline an advance drained and dispatched as any call, before the call asked for.
    fn hand_over_a_due_deadline<'a>(&'a self, state: Guard<'a>) -> Result<Guard<'a>, Answer> {
        let now = match (&state.hooks, state.deadline) {
            (Some(installed), Some(_)) => installed.hooks.now.clone(),
            _ => return Ok(state),
        };
        drop(state);
        let read = catch_unwind(AssertUnwindSafe(|| now()));
        let mut state = self.enter()?;
        let now = match read {
            Ok(now) => now,
            Err(payload) => {
                drop(state);
                self.report(&failure(
                    FailureKind::Clock,
                    format!(
                        "the deadline hook's clock panicked, so a due deadline waits: {}",
                        panic_text(&*payload)
                    ),
                    None,
                ));
                return self.enter();
            }
        };
        let mut notes = Vec::new();
        if !self.advance_when_due(&mut state, now, &mut notes) {
            return Ok(state);
        }
        self.leave(state, notes);
        self.enter()
    }

    fn advance_when_due(&self, state: &mut State, now: i64, notes: &mut Vec<Failure>) -> bool {
        let (Some(at), Some(CorePointer(pointer))) = (state.deadline, state.core) else {
            return false;
        };
        if now < at {
            return false;
        }
        let mut waiting = 0u32;
        // SAFETY: `pointer` is a live core.
        let status = unsafe { (self.functions().weivel_advance)(pointer, now, &mut waiting) };
        note_an_advance_not_taken(status, notes);
        self.drain(state, waiting, notes);
        true
    }

    /// Adds a listener, which hears every event dispatched from now on, after the listeners added before it.
    pub fn add_listener(&self, listener: Listener) -> Result<ListenerRegistration, Answer> {
        let mut state = self.enter()?;
        Ok(state.add_listener(listener))
    }

    /// Removes a listener; one being called now finishes, and hears no more.
    pub fn remove_listener(&self, registration: ListenerRegistration) -> Result<(), Answer> {
        let mut state = self.enter()?;
        let index = state
            .listeners
            .iter()
            .position(|(held, _)| *held == registration.0)
            .ok_or_else(|| {
                Answer::Mistake("no listener of this core has that registration".to_owned())
            })?;
        let removed = state.listeners.remove(index);
        drop(state);
        drop(removed);
        Ok(())
    }

    /// Puts a step at the end of the dispatch queue, after every event drained before it, run by the thread that
    /// dispatches.
    pub fn defer(&self, step: impl FnOnce(&Core) + Send + 'static) -> Result<(), Answer> {
        let mut state = self.enter()?;
        state.queue.push_back(Queued::Step(Box::new(step)));
        self.leave(state, Vec::new());
        Ok(())
    }

    /// Installs the deadline hooks, which `follow` hears the latest announcement through at once. A core holds one
    /// set; installing a second is the caller's mistake.
    pub fn install_deadline_hooks(
        &self,
        hooks: DeadlineHooks,
    ) -> Result<HooksRegistration, Answer> {
        let mut state = self.enter()?;
        if state.hooks.is_some() {
            return Err(Answer::Mistake(
                "this core already has deadline hooks".to_owned(),
            ));
        }
        state.last_registration += 1;
        let registration = state.last_registration;
        state.hooks = Some(InstalledHooks {
            registration,
            hooks,
        });
        state.deadline_serial += u64::from(state.deadline_serial != 0);
        self.leave(state, Vec::new());
        Ok(HooksRegistration(registration))
    }

    /// Removes the deadline hooks `registration` names; answers [`Answer::Absent`] when they are not the ones installed.
    pub fn remove_deadline_hooks(&self, registration: HooksRegistration) -> Result<(), Answer> {
        let mut state = self.enter()?;
        if state.hooks.as_ref().map(|installed| installed.registration) != Some(registration.0) {
            return Err(Answer::Absent);
        }
        let removed = state.hooks.take();
        state.deadline_serial += u64::from(state.deadline_serial != 0);
        self.leave(state, Vec::new());
        drop(removed);
        Ok(())
    }

    /// Destroys the core: what still waits to be dispatched is dropped, and every call after answers
    /// [`Answer::Destroyed`]. Destroying it again does nothing.
    pub fn destroy(&self) {
        let mut state = self.lock();
        let dropped = take_everything(&mut state);
        if let Some(CorePointer(pointer)) = state.core.take() {
            // SAFETY: `pointer` is a live core, destroyed exactly once since it is taken out under the lock.
            unsafe {
                (self.functions().weivel_core_destroy)(pointer);
            }
        }
        drop(state);
        drop(dropped);
    }

    /// Whether the core is destroyed.
    pub fn is_destroyed(&self) -> bool {
        self.lock().core.is_none()
    }

    /// How many events this core's wrapper has drained.
    pub fn last_sequence(&self) -> u64 {
        self.lock().sequence
    }
}

/// What a destroy drops, taken out under the lock and dropped after it, since a dropped closure may itself call in.
struct Dropped {
    _queue: VecDeque<Queued>,
    _listeners: Vec<(u64, Listener)>,
    _hooks: Option<InstalledHooks>,
}

fn take_everything(state: &mut State) -> Dropped {
    state.deadline = None;
    Dropped {
        _queue: std::mem::take(&mut state.queue),
        _listeners: std::mem::take(&mut state.listeners),
        _hooks: state.hooks.take(),
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(CorePointer(pointer)) = state.core.take() {
            // SAFETY: the last reference to the core is going, so nothing else can call it.
            unsafe {
                (self.binary.loaded.functions.weivel_core_destroy)(pointer);
            }
        }
    }
}

fn answer_of(
    status: i32,
    parts: Parts,
    own_refusal: Option<Refusal>,
    notes: &mut Vec<Failure>,
) -> Result<(), Answer> {
    match status {
        abi::STATUS_OK => Ok(()),
        abi::STATUS_NONE => Err(Answer::Absent),
        abi::STATUS_REFUSED => Err(Answer::Refused {
            status: RefusalStatus::Refused,
            refusal: own_refusal,
        }),
        abi::STATUS_OUT_OF_MEMORY => Err(Answer::Refused {
            status: RefusalStatus::OutOfMemory,
            refusal: own_refusal,
        }),
        abi::STATUS_MALFORMED if parts == Parts::CarriesTheCallers => Err(Answer::Refused {
            status: RefusalStatus::Malformed,
            refusal: own_refusal,
        }),
        abi::STATUS_MALFORMED => {
            let detail = own_refusal.map_or_else(
                || "the core queued no refusal saying why".to_owned(),
                |refusal| refusal.detail,
            );
            Err(defect(
                notes,
                format!("the core called a call the layer built malformed: {detail}"),
            ))
        }
        unknown => Err(defect(
            notes,
            format!("the core answered a status the layer does not know: {unknown}"),
        )),
    }
}

fn note_an_advance_not_taken(status: i32, notes: &mut Vec<Failure>) {
    match status {
        abi::STATUS_OK => {}
        abi::STATUS_MALFORMED => {
            defect(
                notes,
                "the core called the advance made for a due deadline malformed".to_owned(),
            );
        }
        _ => notes.push(failure(
            FailureKind::DueAdvance,
            "the core did not take the time of a deadline that fell due, which stays due",
            None,
        )),
    }
}
