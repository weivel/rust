//! The seam's host side: a plugin's library opened by path, its entry table taken from the symbol its caller names,
//! and the host's context table each of its activates is handed, every call through which finds its context by an id
//! in the host's own registry.

use std::collections::{BTreeMap, HashMap};
use std::ffi::{CString, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use super::abi::{
    ANSWER_FIRST_SHAPE, Answering, ENTRY_FIRST_SHAPE, EVENT_CAPABILITY_PROVIDED,
    EVENT_CAPABILITY_WITHDRAWN, EVENT_DEGRADED, EVENT_HOLDINGS_RELEASED, EVENT_REACH_REFUSED,
    EVENT_REVOKED, EVENT_WANT_SETTLED, EVENT_WATCH_REGISTERED, EVENT_WATCH_UNREGISTERED,
    LentManifest, SeamAnswer, SeamAnswered, SeamCapacity, SeamCompletion, SeamContext, SeamEntry,
    SeamEvent, SeamNumber, SeamText, SeamWantLimits, WANT_LIMITS_FIRST_SHAPE, carries,
    failure_kind_of, lend_answered, lend_numbers, lend_refusal, read_numbers, size_of_table,
};
use super::locked;
use crate::context::{
    CapabilityWatcher, CapacityAnswerer, Context, ForeignObject, Listening, Object, Pending,
    PluginEvent, Revocation, ServiceRegistration, ServiceWatcher, Settle, Want, WantLimits,
    Watching, address_of, pending,
};
use crate::core::binary::SharedLibrary;
use crate::core::{
    Answer, DescribedDefinition, Handle, LibraryBinary, LogLevel, Number, Refusal, panic_text,
};
use crate::host::description::BoundDefinition;
use crate::host::entry::{Entry, Finished};
use crate::host::state::Host;

/// The entry symbol a plugin's library exports when its caller names none.
pub const DEFAULT_ENTRY_SYMBOL: &str = "weivel_rust_entry";

const QUESTION_ATTRIBUTES: u32 = 1;
const QUESTION_SERVE: u32 = 2;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static CONTEXTS: Mutex<BTreeMap<u64, Arc<ContextBox>>> = Mutex::new(BTreeMap::new());
static COMPLETIONS: Mutex<BTreeMap<u64, CompletionRecord>> = Mutex::new(BTreeMap::new());

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// An activate or deactivate in flight, kept until it is finished or its context is forgotten.
struct CompletionRecord {
    settle: Settle<Finished>,
    context: u64,
}

/// A plugin's library, open while any entry loaded from it is.
pub(crate) struct PluginLibrary {
    _library: SharedLibrary,
}

/// A plugin's entry table, copied by value, told `free` once nothing of the host calls it any more.
struct EntryHandle {
    table: SeamEntry,
    _library: Arc<PluginLibrary>,
}

// SAFETY: the table's `data` is the plugin's, whose every function may be called from any thread, as the seam's
// entry table promises.
unsafe impl Send for EntryHandle {}
// SAFETY: as above.
unsafe impl Sync for EntryHandle {}

impl Drop for EntryHandle {
    fn drop(&mut self) {
        // SAFETY: nothing of the host holds the handle any more, so nothing calls the table after this.
        let _ = catch_unwind(|| unsafe { (self.table.free)(self.table.data) });
    }
}

impl EntryHandle {
    fn hear(&self, event: &SeamEvent) {
        // SAFETY: the event lives for the call; the plugin's side catches every panic.
        unsafe { (self.table.hear)(self.table.data, event) };
    }
}

/// What the host keeps of one plugin across the seam, found by its id from every call of its context table.
struct ContextBox {
    id: u64,
    context: Context,
    entry: Arc<EntryHandle>,
    registrations: Mutex<HashMap<u64, ServiceRegistration>>,
    wants: Mutex<HashMap<u64, Want>>,
    watches: Mutex<HashMap<u64, Watching>>,
    revocations: Mutex<HashMap<u64, Revocation>>,
    forwarding: Mutex<Option<Listening>>,
}

/// A plugin's entry across the seam, as the host's [`Entry`].
struct SeamEntryOf {
    entry: Arc<EntryHandle>,
    contexts: Mutex<HashMap<Handle, u64>>,
}

impl Host {
    /// Loads a plugin built as a library of its own, a Rust `cdylib` among them, and answers its entry, which the host
    /// registers as any other: the library's `symbol`, [`DEFAULT_ENTRY_SYMBOL`] when `None`, answers the plugin's
    /// entry table, and each activate is handed the host's context table of its plugin and its whole manifest, every
    /// call across both catching a panic so none unwinds through the seam. The library stays open while any entry
    /// loaded from it lives, and each is told to free its table once the host lets go of it. Answers
    /// [`Answer::Mistake`] for a path that names no library, a symbol it lacks, a symbol answering no table, or a
    /// table below its first shape; [`Answer::Destroyed`] once the destroy has been called.
    pub fn load_entry(&self, path: &Path, symbol: Option<&str>) -> Result<Arc<dyn Entry>, Answer> {
        if self.shared.is_destroy_called() {
            return Err(Answer::Destroyed);
        }
        let symbol = symbol.unwrap_or(DEFAULT_ENTRY_SYMBOL);
        let library = SharedLibrary::open(path).ok_or_else(|| {
            Answer::Mistake(format!(
                "{} names no library this process can load",
                path.display()
            ))
        })?;
        let name = CString::new(symbol)
            .map_err(|_| Answer::Mistake(format!("the entry symbol {symbol:?} holds a NUL")))?;
        let address = library.symbol(&name).ok_or_else(|| {
            Answer::Mistake(format!(
                "{} exports no entry symbol {symbol}",
                path.display()
            ))
        })?;
        // SAFETY: an entry symbol is `const SeamEntry *f(void)`, as `export_entry!` writes it.
        let table = unsafe {
            std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> *const SeamEntry>(address)(
            )
        };
        let table = entry_table_of(table, symbol)?;
        Ok(Arc::new(SeamEntryOf {
            entry: Arc::new(EntryHandle {
                table,
                _library: Arc::new(PluginLibrary { _library: library }),
            }),
            contexts: Mutex::new(HashMap::new()),
        }))
    }
}

/// Copies an entry table the plugin answered, refusing NULL and one below its first shape.
pub(super) fn entry_table_of(table: *const SeamEntry, symbol: &str) -> Result<SeamEntry, Answer> {
    if table.is_null() {
        return Err(Answer::Mistake(format!(
            "the entry symbol {symbol} answered no entry table"
        )));
    }
    // SAFETY: a non-NULL entry table begins with its `size`, read before anything past it.
    let size = unsafe { (*table).size };
    if !carries(size, ENTRY_FIRST_SHAPE) {
        return Err(Answer::Mistake(format!(
            "the entry table {symbol} answered is {size} bytes, below the {ENTRY_FIRST_SHAPE} of its first shape"
        )));
    }
    // SAFETY: the table reaches its first shape, which is every field this build reads.
    Ok(unsafe { std::ptr::read(table) })
}

fn box_of(id: u64) -> Option<Arc<ContextBox>> {
    locked(&CONTEXTS).get(&id).cloned()
}

fn completion(settle: Settle<Finished>, context: u64) -> SeamCompletion {
    let id = next_id();
    locked(&COMPLETIONS).insert(id, CompletionRecord { settle, context });
    SeamCompletion {
        size: size_of_table::<SeamCompletion>(),
        data: id,
        finish: finish_completion,
    }
}

fn settle_completion(id: u64, finished: Finished) {
    if let Some(record) = locked(&COMPLETIONS).remove(&id) {
        record.settle.settle(finished);
    }
}

impl SeamEntryOf {
    /// The box of `context`'s plugin, made at its first activate; what its last open made is let go of, since the
    /// core ended it.
    fn box_for(&self, context: &Context) -> Arc<ContextBox> {
        let mut contexts = locked(&self.contexts);
        let id = *contexts.entry(context.plugin()).or_insert_with(next_id);
        let boxed = locked(&CONTEXTS)
            .entry(id)
            .or_insert_with(|| {
                Arc::new(ContextBox {
                    id,
                    context: context.clone(),
                    entry: self.entry.clone(),
                    registrations: Mutex::new(HashMap::new()),
                    wants: Mutex::new(HashMap::new()),
                    watches: Mutex::new(HashMap::new()),
                    revocations: Mutex::new(HashMap::new()),
                    forwarding: Mutex::new(None),
                })
            })
            .clone();
        locked(&boxed.registrations).clear();
        locked(&boxed.watches).clear();
        locked(&boxed.revocations).clear();
        let forwarding = forward_events(&boxed);
        if let Some(previous) = locked(&boxed.forwarding).replace(forwarding) {
            previous.stop();
        }
        boxed
    }
}

fn context_table(boxed: &ContextBox) -> SeamContext {
    SeamContext {
        size: size_of_table::<SeamContext>(),
        data: boxed.id,
        plugin: boxed.context.plugin().0,
        write: context_write,
        report: context_report,
        get: context_get,
        ids: context_ids,
        want: context_want,
        withdraw: context_withdraw,
        watch: context_watch,
        stop_watch: context_stop_watch,
        register: context_register,
        unregister: context_unregister,
        release_holding: context_release_holding,
        provide: context_provide,
        implementations: context_implementations,
        dispatches_here: context_dispatches_here,
    }
}

impl Entry for SeamEntryOf {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let (activating, settle) = pending();
        let boxed = self.box_for(context);
        let table = context_table(&boxed);
        let manifest = LentManifest::of(context.manifest());
        let completion = completion(settle, boxed.id);
        let entry = &self.entry.table;
        // SAFETY: every table lives for the call; the plugin's side catches every panic.
        let taken = unsafe { (entry.activate)(entry.data, &table, &manifest.table(), &completion) };
        if taken != 0 {
            settle_completion(
                completion.data,
                Err("the plugin's entry table refused its activate".to_owned()),
            );
        }
        activating
    }

    fn deactivate(&self) -> Option<Pending<Finished>> {
        let (deactivating, settle) = pending();
        let completion = completion(settle, 0);
        let entry = &self.entry.table;
        // SAFETY: as for `activate`.
        let taken = unsafe { (entry.deactivate)(entry.data, &completion) };
        if taken != 0 {
            locked(&COMPLETIONS).remove(&completion.data);
            return None;
        }
        Some(deactivating)
    }

    fn forgotten(&self, context: &Context) {
        let Some(id) = locked(&self.contexts).remove(&context.plugin()) else {
            return;
        };
        let boxed = locked(&CONTEXTS).remove(&id);
        locked(&COMPLETIONS).retain(|_, record| record.context != id);
        if let Some(boxed) = boxed
            && let Some(forwarding) = locked(&boxed.forwarding).take()
        {
            forwarding.stop();
        }
        let entry = &self.entry.table;
        // SAFETY: the plugin's side catches every panic.
        unsafe { (entry.forget_context)(entry.data, id) };
    }
}

struct LentEvent {
    kind: u32,
    token: u64,
    address: usize,
    description: usize,
    registration: u64,
    awaits_release: bool,
    reach_kind: u32,
    id: String,
    refusal: Option<Refusal>,
}

impl LentEvent {
    fn of(kind: u32) -> LentEvent {
        LentEvent {
            kind,
            token: 0,
            address: 0,
            description: 0,
            registration: 0,
            awaits_release: false,
            reach_kind: 0,
            id: String::new(),
            refusal: None,
        }
    }

    fn tell(self, boxed: &ContextBox, settlement: Option<&SeamAnswered>) {
        boxed.entry.hear(&SeamEvent {
            size: size_of_table::<SeamEvent>(),
            kind: self.kind,
            context: boxed.id,
            token: self.token,
            address: self.address,
            registration: self.registration,
            awaits_release: u32::from(self.awaits_release),
            reach_kind: self.reach_kind,
            id: SeamText::of(&self.id),
            refusal: lend_refusal(self.refusal.as_ref()),
            settlement: settlement
                .map_or(std::ptr::null(), |settled| settled as *const SeamAnswered),
            description: self.description,
        });
    }
}

/// Forwards what the core reports about the plugin to its side of the seam, keeping a revocation awaiting its release
/// until the plugin lets go of it.
fn forward_events(boxed: &Arc<ContextBox>) -> Listening {
    let id = boxed.id;
    boxed.context.listen(move |event| {
        let Some(boxed) = box_of(id) else {
            return;
        };
        let told = match event {
            PluginEvent::Revoked(revocation) => {
                if revocation.awaits_release {
                    locked(&boxed.revocations)
                        .insert(revocation.registration.0, revocation.clone());
                }
                LentEvent {
                    address: address_of(&revocation.object),
                    description: description_of(&revocation.object),
                    registration: revocation.registration.0,
                    awaits_release: revocation.awaits_release,
                    ..LentEvent::of(EVENT_REVOKED)
                }
            }
            PluginEvent::HoldingsReleased { id, service } => LentEvent {
                address: address_of(service),
                description: description_of(service),
                id: id.clone(),
                ..LentEvent::of(EVENT_HOLDINGS_RELEASED)
            },
            PluginEvent::ReachRefused { kind, id, refusal } => LentEvent {
                reach_kind: kind.0,
                id: id.clone(),
                refusal: Some(refusal.clone()),
                ..LentEvent::of(EVENT_REACH_REFUSED)
            },
            PluginEvent::Degraded {
                id,
                service,
                unmet_constraint,
            } => LentEvent {
                address: address_of(service),
                description: description_of(service),
                id: id.clone(),
                refusal: Some(unmet_constraint.clone()),
                ..LentEvent::of(EVENT_DEGRADED)
            },
        };
        told.tell(&boxed, None);
    })
}

/// Tells the plugin of a watched change, by the token it chose for the watch.
struct ForwardingWatcher {
    context: u64,
    token: u64,
}

impl ForwardingWatcher {
    fn tell(&self, kind: u32, object: Option<&Object>) {
        if let Some(boxed) = box_of(self.context) {
            LentEvent {
                token: self.token,
                address: object.map_or(0, address_of),
                description: object.map_or(0, description_of),
                ..LentEvent::of(kind)
            }
            .tell(&boxed, None);
        }
    }
}

impl ServiceWatcher for ForwardingWatcher {
    fn registered(&self, service: &Object) {
        self.tell(EVENT_WATCH_REGISTERED, Some(service));
    }

    fn unregistered(&self) {
        self.tell(EVENT_WATCH_UNREGISTERED, None);
    }
}

impl CapabilityWatcher for ForwardingWatcher {
    fn provided(&self, implementation: &Object) {
        self.tell(EVENT_CAPABILITY_PROVIDED, Some(implementation));
    }

    fn withdrawn(&self, implementation: &Object) {
        self.tell(EVENT_CAPABILITY_WITHDRAWN, Some(implementation));
    }
}

/// Asks the plugin's own capacity answerer, by the id it chose for it.
struct ForwardingAnswerer {
    context: u64,
    answerer: u64,
}

#[derive(Default)]
struct CapacityTaken {
    serves: bool,
    attributes: Vec<Number>,
}

unsafe extern "C" fn take_capacity(
    data: *mut c_void,
    serves: u32,
    attributes: *const SeamNumber,
    count: usize,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `data` is the asker's own, for the length of the call; the attributes are lent for it too.
        let taken = unsafe { &mut *(data as *mut CapacityTaken) };
        taken.serves = serves != 0;
        // SAFETY: as above.
        taken.attributes = unsafe { read_numbers(attributes, count) };
    }));
}

impl ForwardingAnswerer {
    fn ask(&self, question: u32, constraints: &[Number]) -> CapacityTaken {
        let mut taken = CapacityTaken::default();
        let Some(boxed) = box_of(self.context) else {
            return taken;
        };
        let lent = lend_numbers(constraints);
        let answer = SeamCapacity {
            size: size_of_table::<SeamCapacity>(),
            data: &mut taken as *mut CapacityTaken as *mut c_void,
            take: take_capacity,
        };
        let entry = &boxed.entry.table;
        // SAFETY: everything lent lives for the call; the plugin's side catches every panic.
        unsafe {
            (entry.capacity)(
                entry.data,
                self.context,
                self.answerer,
                question,
                lent.as_ptr(),
                lent.len(),
                &answer,
            )
        };
        taken
    }
}

impl CapacityAnswerer for ForwardingAnswerer {
    fn attributes(&self) -> Vec<Number> {
        self.ask(QUESTION_ATTRIBUTES, &[]).attributes
    }

    fn serves(&self, constraints: &[Number]) -> bool {
        self.ask(QUESTION_SERVE, constraints).serves
    }
}

unsafe extern "C" fn finish_completion(data: u64, failed: u32, text: SeamText) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the text is lent for the call.
        let finished = if failed == 0 {
            Ok(())
        } else {
            Err(unsafe { text.read() })
        };
        settle_completion(data, finished);
    }));
}

/// Answers a call of the context table to the caller's answer: a call handed no answer it can take is not made, since
/// it could not be told what came of it.
fn answered(
    data: u64,
    answer: *const SeamAnswer,
    call: impl FnOnce(&ContextBox) -> Result<Answering, Answer>,
) {
    if answer.is_null() {
        return;
    }
    // SAFETY: a non-NULL answer begins with its `size`.
    if !carries(unsafe { (*answer).size }, ANSWER_FIRST_SHAPE) {
        return;
    }
    // SAFETY: the answer reaches its first shape, and is the caller's for the length of the call.
    let answer = unsafe { &*answer };
    let outcome = catch_unwind(AssertUnwindSafe(|| match box_of(data) {
        Some(boxed) => call(&boxed),
        None => Err(Answer::Destroyed),
    }))
    .unwrap_or_else(|payload| {
        Err(Answer::Defect(format!(
            "the host's side of the plugin seam panicked: {}",
            panic_text(&*payload)
        )))
    });
    // SAFETY: the answer's `take` is the caller's, called once with what this call answered.
    lend_answered(&outcome, |lent| unsafe { (answer.take)(answer.data, lent) });
}

/// The description of each definition a library the host loaded registered, by its definition's address, kept while
/// that library's binary is, so a plugin across the seam binds the very definition an in-process plugin is handed.
static DESCRIBED: Mutex<BTreeMap<usize, (usize, Weak<LibraryBinary>)>> =
    Mutex::new(BTreeMap::new());

/// Records the description `described` pairs with its definition, while `binary` stays open.
pub(crate) fn remember_described(described: &DescribedDefinition, binary: &Arc<LibraryBinary>) {
    let mut kept = locked(&DESCRIBED);
    kept.retain(|_, (_, library)| library.strong_count() > 0);
    kept.insert(
        described.definition_address(),
        (described.description_address(), Arc::downgrade(binary)),
    );
}

/// The description of `object` when a description bound it, 0 otherwise.
fn description_of(object: &Object) -> usize {
    let Some(bound) = object.downcast_ref::<BoundDefinition>() else {
        return 0;
    };
    locked(&DESCRIBED)
        .get(&bound.address())
        .filter(|(_, library)| library.strong_count() > 0)
        .map_or(0, |(description, _)| *description)
}

/// An object as it crosses: its address, and its description where one bound it.
fn crossing(object: &Object) -> Result<Answering, Answer> {
    Ok(Answering {
        number: address_of(object) as u64,
        description: description_of(object),
        ..Answering::default()
    })
}

fn done() -> Result<Answering, Answer> {
    Ok(Answering::default())
}

fn number(number: u64) -> Result<Answering, Answer> {
    Ok(Answering {
        number,
        ..Answering::default()
    })
}

fn foreign(address: usize) -> Object {
    Arc::new(ForeignObject::at(address))
}

/// # Safety
/// The text is lent for the length of the call.
unsafe fn text(text: SeamText) -> String {
    // SAFETY: as the caller promised.
    unsafe { text.read() }
}

unsafe extern "C" fn context_write(data: u64, level: u32, written: SeamText) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(boxed) = box_of(data) {
            // SAFETY: the text is lent for the call.
            boxed
                .context
                .log()
                .write(LogLevel(level), unsafe { text(written) });
        }
    }));
}

unsafe extern "C" fn context_report(data: u64, kind: u32, reported: SeamText) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(boxed) = box_of(data) {
            // SAFETY: the text is lent for the call.
            boxed
                .context
                .report(failure_kind_of(kind), unsafe { text(reported) });
        }
    }));
}

unsafe extern "C" fn context_dispatches_here(data: u64) -> u32 {
    catch_unwind(|| box_of(data).is_some_and(|boxed| boxed.context.dispatches_here()))
        .map(u32::from)
        .unwrap_or(0)
}

unsafe extern "C" fn context_get(data: u64, id: SeamText, answer: *const SeamAnswer) {
    // SAFETY: the id is lent for the call.
    answered(data, answer, |boxed| {
        crossing(&boxed.context.get(&unsafe { text(id) })?)
    });
}

unsafe extern "C" fn context_ids(data: u64, answer: *const SeamAnswer) {
    answered(data, answer, |boxed| {
        Ok(Answering {
            texts: boxed.context.ids()?,
            ..Answering::default()
        })
    });
}

/// # Safety
/// The limits are lent for the length of the call, or NULL for none.
unsafe fn limits_of(limits: *const SeamWantLimits) -> Result<WantLimits, Answer> {
    if limits.is_null() {
        return Ok(WantLimits::unbounded());
    }
    // SAFETY: as the caller promised; the `size` is read before anything past it.
    let limits = unsafe { &*limits };
    if !carries(limits.size, WANT_LIMITS_FIRST_SHAPE) {
        return Err(Answer::Mistake(
            "the want's limits are below their first shape".to_owned(),
        ));
    }
    let limit = |present: u32, value: i64| (present != 0).then_some(value);
    Ok(WantLimits {
        timeout_millis: limit(limits.has_timeout, limits.timeout_millis),
        budget_millis: limit(limits.has_budget, limits.budget_millis),
        poll_millis: limit(limits.has_poll, limits.poll_millis),
    })
}

fn tell_settlement(context: u64, token: u64, settlement: &Result<Object, Answer>) {
    let Some(boxed) = box_of(context) else {
        return;
    };
    locked(&boxed.wants).remove(&token);
    let answered = settlement.as_ref().map_err(Clone::clone).and_then(crossing);
    lend_answered(&answered, |lent| {
        LentEvent {
            token,
            ..LentEvent::of(EVENT_WANT_SETTLED)
        }
        .tell(&boxed, Some(lent))
    });
}

unsafe extern "C" fn context_want(
    data: u64,
    id: SeamText,
    limits: *const SeamWantLimits,
    token: u64,
    answer: *const SeamAnswer,
) {
    answered(data, answer, |boxed| {
        // SAFETY: the limits and the id are lent for the call.
        let limits = unsafe { limits_of(limits)? };
        // SAFETY: as above.
        let want = boxed.context.want(&unsafe { text(id) }, limits);
        let settlement = want.settlement();
        locked(&boxed.wants).insert(token, want);
        let context = boxed.id;
        settlement.on_settled(move |settled| tell_settlement(context, token, settled));
        done()
    });
}

unsafe extern "C" fn context_withdraw(data: u64, token: u64, answer: *const SeamAnswer) {
    answered(data, answer, |boxed| {
        let want = locked(&boxed.wants).remove(&token).ok_or(Answer::Absent)?;
        want.withdraw()?;
        done()
    });
}

unsafe extern "C" fn context_watch(
    data: u64,
    id: SeamText,
    capability: u32,
    token: u64,
    answer: *const SeamAnswer,
) {
    answered(data, answer, |boxed| {
        // SAFETY: the id is lent for the call.
        let id = unsafe { text(id) };
        let watcher = Arc::new(ForwardingWatcher {
            context: boxed.id,
            token,
        });
        let watching = if capability != 0 {
            boxed.context.watch_capability(&id, watcher)?
        } else {
            boxed.context.watch(&id, watcher)?
        };
        locked(&boxed.watches).insert(token, watching);
        done()
    });
}

unsafe extern "C" fn context_stop_watch(data: u64, token: u64, answer: *const SeamAnswer) {
    answered(data, answer, |boxed| {
        let watching = locked(&boxed.watches)
            .remove(&token)
            .ok_or(Answer::Absent)?;
        watching.stop()?;
        done()
    });
}

unsafe extern "C" fn context_register(
    data: u64,
    id: SeamText,
    address: usize,
    awaiting_releases: u32,
    answerer: u64,
    answer: *const SeamAnswer,
) {
    answered(data, answer, |boxed| {
        // SAFETY: the id is lent for the call.
        let id = unsafe { text(id) };
        let answerer: Option<Arc<dyn CapacityAnswerer>> = (answerer != 0).then(|| {
            Arc::new(ForwardingAnswerer {
                context: boxed.id,
                answerer,
            }) as Arc<dyn CapacityAnswerer>
        });
        let registered = boxed.context.register_with(
            &id,
            foreign(address),
            crate::context::Registering {
                answerer,
                awaiting: (awaiting_releases != 0).then_some(None),
                ..crate::context::Registering::default()
            },
        )?;
        let handle = registered.handle().0;
        locked(&boxed.registrations).insert(handle, registered);
        number(handle)
    });
}

unsafe extern "C" fn context_unregister(data: u64, registration: u64, answer: *const SeamAnswer) {
    answered(data, answer, |boxed| {
        let registered = locked(&boxed.registrations)
            .remove(&registration)
            .ok_or_else(|| {
                Answer::Mistake(
                    "the plugin unregistered a registration it does not hold".to_owned(),
                )
            })?;
        registered.unregister()?;
        done()
    });
}

unsafe extern "C" fn context_release_holding(
    data: u64,
    registration: u64,
    answer: *const SeamAnswer,
) {
    answered(data, answer, |boxed| {
        let revocation = locked(&boxed.revocations).remove(&registration);
        let let_go = match revocation {
            Some(revocation) => boxed.context.release_holding(&revocation)?,
            None => false,
        };
        number(u64::from(let_go))
    });
}

unsafe extern "C" fn context_provide(
    data: u64,
    id: SeamText,
    address: usize,
    answer: *const SeamAnswer,
) {
    answered(data, answer, |boxed| {
        // SAFETY: the id is lent for the call.
        boxed
            .context
            .provide(&unsafe { text(id) }, foreign(address))?;
        done()
    });
}

unsafe extern "C" fn context_implementations(data: u64, id: SeamText, answer: *const SeamAnswer) {
    answered(data, answer, |boxed| {
        // SAFETY: the id is lent for the call.
        let implementations = boxed.context.implementations(&unsafe { text(id) })?;
        Ok(Answering {
            addresses: implementations.iter().map(address_of).collect(),
            descriptions: implementations.iter().map(description_of).collect(),
            ..Answering::default()
        })
    });
}

/// How many contexts and completions the host's side of the seam keeps now: what a test reads to prove each is let
/// go of at its end.
#[cfg(test)]
pub(super) fn kept_counts() -> (usize, usize) {
    (locked(&CONTEXTS).len(), locked(&COMPLETIONS).len())
}

/// Calls the table's get with no answer, as a plugin handing NULL would.
///
/// # Safety
/// Nothing: the call reads no answer.
#[cfg(test)]
pub(super) unsafe fn call_get_without_an_answer() {
    // SAFETY: the id lives for the call, and a NULL answer is what is under test.
    unsafe { context_get(1, SeamText::of("x"), std::ptr::null()) };
}
