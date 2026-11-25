//! The seam's host side: a plugin's library opened by path, its entry table taken from the symbol its caller names,
//! and the host's context table each of its activates is handed. What the host keeps of the seam lives in the host's
//! own [`SeamRegistry`]; a context or a completion crosses as an id that resolves to it only while the host keeps it.

use std::collections::{BTreeMap, HashMap};
use std::ffi::{CString, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use super::abi::{
    ANSWER_FIRST_SHAPE, Answering, ENTRY_CALL_END, ENTRY_FIRST_SHAPE, EVENT_CAPABILITY_PROVIDED,
    EVENT_CAPABILITY_WITHDRAWN, EVENT_DEGRADED, EVENT_HOLDINGS_RELEASED, EVENT_LIBRARY_UNLOADING,
    EVENT_REACH_REFUSED, EVENT_REVOKED, EVENT_WANT_SETTLED, EVENT_WATCH_REGISTERED,
    EVENT_WATCH_UNREGISTERED, LentManifest, SeamAnswer, SeamAnswered, SeamCapacity, SeamCompletion,
    SeamContext, SeamEntry, SeamEvent, SeamNumber, SeamText, SeamTexts, SeamWantLimits,
    WANT_LIMITS_FIRST_SHAPE, answered_by, carries, copy_table, failure_kind_of, lend_answered,
    lend_numbers, lend_refusal, read_numbers, read_texts, size_of_table,
};
use super::locked;
use super::trees::{encode, result_of};
use crate::context::{
    CapabilityWatcher, CapacityAnswerer, Context, ForeignObject, Listening, Object, Pending,
    PluginEvent, Revocation, ServiceRegistration, ServiceWatcher, Settle, Want, WantLimits,
    Watching, address_of, pending,
};
use crate::core::binary::SharedLibrary;
use crate::core::{
    Answer, DescribedDefinition, Handle, LibraryBinary, LogLevel, Number, Refusal, panic_text,
};
use crate::host::description::{BoundDefinition, Tree};
use crate::host::entry::{Entry, Finished};
use crate::host::methods::{MethodResult, Methods};
use crate::host::state::Host;

/// The entry symbol a plugin's library exports when its caller names none.
pub const DEFAULT_ENTRY_SYMBOL: &str = "weivel_rust_entry";

const QUESTION_ATTRIBUTES: u32 = 1;
const QUESTION_SERVE: u32 = 2;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Every context and completion id any host handed across, resolving only while that host keeps what it names.
static CONTEXT_IDS: Mutex<BTreeMap<u64, Weak<ContextBox>>> = Mutex::new(BTreeMap::new());
static COMPLETION_IDS: Mutex<BTreeMap<u64, Weak<CompletionRecord>>> = Mutex::new(BTreeMap::new());

/// How many calls a plugin made into a host's side of the seam are running now, which a plugin's library is never
/// closed under, since each returns into that library's code.
static CALLS_FROM_PLUGINS: AtomicUsize = AtomicUsize::new(0);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Counts one call a plugin made into the host's side while it runs.
struct CallFromAPlugin;

impl CallFromAPlugin {
    fn enter() -> CallFromAPlugin {
        CALLS_FROM_PLUGINS.fetch_add(1, Ordering::AcqRel);
        CallFromAPlugin
    }
}

impl Drop for CallFromAPlugin {
    fn drop(&mut self) {
        CALLS_FROM_PLUGINS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// An activate or deactivate in flight, kept until it is finished or its context is forgotten.
struct CompletionRecord {
    id: u64,
    settle: Mutex<Option<Settle<Finished>>>,
    context: u64,
    registry: Weak<SeamRegistry>,
}

/// A library the host loaded whose described definitions crossed the seam, held mapped for each call a plugin makes
/// through its own binding of one.
struct MappingRecord {
    binary: Weak<LibraryBinary>,
    holds: Vec<Arc<LibraryBinary>>,
}

/// What one host keeps of the seam: each plugin's context box, each completion in flight, and each described
/// definition with the library it lies in. Dropped with the host, which ends every id that names any of it.
#[derive(Default)]
pub(crate) struct SeamRegistry {
    contexts: Mutex<BTreeMap<u64, Arc<ContextBox>>>,
    completions: Mutex<BTreeMap<u64, Arc<CompletionRecord>>>,
    described: Mutex<BTreeMap<usize, (usize, u64)>>,
    mappings: Mutex<BTreeMap<u64, MappingRecord>>,
}

impl SeamRegistry {
    /// Records the description `described` pairs with its definition, in `binary`, so a plugin across the seam binds
    /// the very definition an in-process plugin is handed.
    pub(crate) fn remember_described(
        &self,
        described: &DescribedDefinition,
        binary: &Arc<LibraryBinary>,
    ) {
        let mapping = self.mapping_of(binary);
        let mut kept = locked(&self.described);
        kept.retain(|_, (_, known)| locked(&self.mappings).contains_key(known));
        kept.insert(
            described.definition_address(),
            (described.description_address(), mapping),
        );
    }

    /// The id of the mapping of `binary`, made at the first definition of it that crosses.
    fn mapping_of(&self, binary: &Arc<LibraryBinary>) -> u64 {
        let mut mappings = locked(&self.mappings);
        mappings.retain(|_, record| record.binary.strong_count() > 0);
        let known = mappings
            .iter()
            .find(|(_, record)| std::ptr::eq(record.binary.as_ptr(), Arc::as_ptr(binary)))
            .map(|(id, _)| *id);
        known.unwrap_or_else(|| {
            let id = next_id();
            mappings.insert(
                id,
                MappingRecord {
                    binary: Arc::downgrade(binary),
                    holds: Vec::new(),
                },
            );
            id
        })
    }

    /// The description of `object` and the id of the library it lies in, when a description bound it; zeros otherwise.
    /// A definition an answer carried, as a scope's are, crosses with the nested description it was bound from and
    /// the library its carrier lies in.
    fn description_of(&self, object: &Object) -> (usize, u64) {
        let Some(bound) = object.downcast_ref::<BoundDefinition>() else {
            return (0, 0);
        };
        if let Some(known) = locked(&self.described).get(&bound.address()).copied() {
            return known;
        }
        match bound.library() {
            Some(binary) => (bound.description_address(), self.mapping_of(&binary)),
            None => (0, 0),
        }
    }

    fn hold(&self, mapping: u64) -> bool {
        let mut mappings = locked(&self.mappings);
        let Some(record) = mappings.get_mut(&mapping) else {
            return false;
        };
        let Some(binary) = record.binary.upgrade() else {
            return false;
        };
        record.holds.push(binary);
        true
    }

    fn unhold(&self, mapping: u64) {
        let released = locked(&self.mappings)
            .get_mut(&mapping)
            .and_then(|record| record.holds.pop());
        drop(released);
    }

    /// Tells every plugin across the seam that `binary` is about to close to calls, while it is still mapped, so each
    /// releases what its own bindings of the library's definitions got.
    pub(crate) fn library_unloading(&self, binary: &Arc<LibraryBinary>) {
        let mapping = locked(&self.mappings)
            .iter()
            .find(|(_, record)| std::ptr::eq(record.binary.as_ptr(), Arc::as_ptr(binary)))
            .map(|(id, _)| *id);
        let Some(mapping) = mapping else {
            return;
        };
        let boxes: Vec<Arc<ContextBox>> = locked(&self.contexts).values().cloned().collect();
        for boxed in boxes {
            LentEvent {
                mapping,
                ..LentEvent::of(EVENT_LIBRARY_UNLOADING)
            }
            .tell(&boxed, None);
        }
    }

    fn completion(self: &Arc<Self>, settle: Settle<Finished>, context: u64) -> SeamCompletion {
        let id = next_id();
        let record = Arc::new(CompletionRecord {
            id,
            settle: Mutex::new(Some(settle)),
            context,
            registry: Arc::downgrade(self),
        });
        locked(&COMPLETION_IDS).insert(id, Arc::downgrade(&record));
        locked(&self.completions).insert(id, record);
        SeamCompletion {
            size: size_of_table::<SeamCompletion>(),
            data: id,
            finish: finish_completion,
        }
    }

    fn forget_completion(&self, id: u64) {
        locked(&COMPLETION_IDS).remove(&id);
        let forgotten = locked(&self.completions).remove(&id);
        drop(forgotten);
    }

    /// How many contexts and completions it keeps now: what a test reads to prove each is let go of at its end.
    #[cfg(test)]
    pub(super) fn kept_counts(&self) -> (usize, usize) {
        (
            locked(&self.contexts).len(),
            locked(&self.completions).len(),
        )
    }
}

/// Settles a completion once: the record is taken out under its lock, and settled outside every lock of the seam's.
fn settle_completion(id: u64, finished: Finished) {
    let record = locked(&COMPLETION_IDS)
        .remove(&id)
        .and_then(|record| record.upgrade());
    let Some(record) = record else {
        return;
    };
    let settle = locked(&record.settle).take();
    if let Some(registry) = record.registry.upgrade() {
        let removed = locked(&registry.completions).remove(&record.id);
        drop(removed);
    }
    if let Some(settle) = settle {
        settle.settle(finished);
    }
}

/// A plugin's library, open while any entry loaded from it is.
pub(crate) struct PluginLibrary {
    _library: SharedLibrary,
}

/// A plugin's entry table, copied by value, told `free` once nothing of the host calls it any more.
struct EntryHandle {
    table: SeamEntry,
    library: Option<Arc<PluginLibrary>>,
}

// SAFETY: the table's `data` is the plugin's, whose every function may be called from any thread, as the seam's
// entry table promises.
unsafe impl Send for EntryHandle {}
// SAFETY: as above.
unsafe impl Sync for EntryHandle {}

/// An entry the host let go of, freed and its library dropped on the seam's own thread once no call a plugin made
/// into the host is running, so neither runs inside a call that will return into the library's code.
struct Retired {
    table: SeamEntry,
    library: Option<Arc<PluginLibrary>>,
}

// SAFETY: as for `EntryHandle`.
unsafe impl Send for Retired {}

fn retire(retired: Retired) {
    static RETIRING: OnceLock<Mutex<Sender<Retired>>> = OnceLock::new();
    let sender = RETIRING.get_or_init(|| {
        let (sender, receiving) = channel::<Retired>();
        let started = std::thread::Builder::new()
            .name("weivel-seam-retire".to_owned())
            .spawn(move || {
                for retired in receiving {
                    free_once_no_plugin_calls_in(retired);
                }
            });
        if started.is_err() {
            eprintln!("weivel: the seam's retiring thread could not be started, so retired entries stay loaded");
        }
        Mutex::new(sender)
    });
    if let Err(unsent) = locked(sender).send(retired) {
        std::mem::forget(unsent.0);
    }
}

fn free_once_no_plugin_calls_in(retired: Retired) {
    while CALLS_FROM_PLUGINS.load(Ordering::Acquire) > 0 {
        std::thread::sleep(Duration::from_millis(1));
    }
    // SAFETY: nothing of the host holds the entry any more, so nothing calls the table after this.
    let _ = catch_unwind(|| unsafe { (retired.table.free)(retired.table.data) });
    drop(retired.library);
}

impl Drop for EntryHandle {
    fn drop(&mut self) {
        retire(Retired {
            table: self.table,
            library: self.library.take(),
        });
    }
}

impl EntryHandle {
    fn hear(&self, event: &SeamEvent) {
        // SAFETY: the event lives for the call; the plugin's side catches every panic.
        unsafe { (self.table.hear)(self.table.data, event) };
    }

    /// Whether the plugin's table carries the calls of methods by name, appended after its first shape.
    fn calls_methods(&self) -> bool {
        carries(self.table.size, ENTRY_CALL_END)
    }

    /// The names of the entry's actions, none for a table that carries no calls of methods.
    fn action_names(&self) -> Vec<String> {
        let Some(action_names) = self.table.action_names.filter(|_| self.calls_methods()) else {
            return Vec::new();
        };
        // SAFETY: the answer lives for the call; the plugin's side catches every panic.
        answered_by(|answer| unsafe { action_names(self.table.data, answer) })
            .map(|answered| answered.texts)
            .unwrap_or_default()
    }

    /// Calls the method `name` of the plugin's object at `address`, through the context `context`, or of the entry's
    /// actions for address 0.
    fn call(
        &self,
        context: u64,
        address: usize,
        name: &str,
        arguments: Option<&Tree>,
    ) -> MethodResult {
        let Some(call) = self.table.call.filter(|_| self.calls_methods()) else {
            return Err(format!(
                "the plugin's entry table carries no calls of methods, so {name} cannot be called"
            ));
        };
        let bytes = arguments.map(encode).unwrap_or_default();
        // SAFETY: the name, the bytes and the answer live for the call; the plugin's side catches every panic.
        let answered = answered_by(|answer| unsafe {
            call(
                self.table.data,
                context,
                address,
                SeamText::of(name),
                bytes.as_ptr(),
                bytes.len(),
                u32::from(arguments.is_some()),
                answer,
            )
        });
        result_of(answered, name)
    }
}

/// Methods named by the plugin, each calling back across the seam to the plugin's object at `address`.
fn methods_across(
    entry: &Arc<EntryHandle>,
    context: u64,
    address: usize,
    names: Vec<String>,
) -> Methods {
    names.into_iter().fold(Methods::new(), |methods, name| {
        let entry = entry.clone();
        let called = name.clone();
        methods.with(name, move |arguments| {
            entry.call(context, address, &called, arguments)
        })
    })
}

/// What the host keeps of one plugin across the seam, found by its id from every call of its context table.
struct ContextBox {
    id: u64,
    context: Context,
    entry: Arc<EntryHandle>,
    registry: Weak<SeamRegistry>,
    registrations: Mutex<HashMap<u64, ServiceRegistration>>,
    wants: Mutex<HashMap<u64, Want>>,
    watches: Mutex<HashMap<u64, Option<Watching>>>,
    revocations: Mutex<HashMap<u64, Revocation>>,
    forwarding: Mutex<Option<Listening>>,
    /// The methods the plugin said each of its objects offers, by the object's address.
    offered_methods: Mutex<HashMap<usize, Vec<String>>>,
}

impl ContextBox {
    /// The host's object for the plugin's object at `address`: its address, and the methods the plugin said it
    /// offers, each calling back across the seam.
    fn foreign(&self, address: usize) -> Object {
        match locked(&self.offered_methods).get(&address) {
            Some(names) => Arc::new(ForeignObject::offering(
                address,
                methods_across(&self.entry, self.id, address, names.clone()),
            )),
            None => foreign(address),
        }
    }
}

impl ContextBox {
    fn description_of(&self, object: &Object) -> (usize, u64) {
        self.registry
            .upgrade()
            .map_or((0, 0), |registry| registry.description_of(object))
    }
}

/// A plugin's entry across the seam, as the host's [`Entry`].
struct SeamEntryOf {
    entry: Arc<EntryHandle>,
    registry: Weak<SeamRegistry>,
    contexts: Mutex<HashMap<Handle, u64>>,
}

impl Host {
    /// Loads a plugin built as a library of its own, a Rust `cdylib` among them, and answers its entry, which the host
    /// registers as any other: the library's `symbol`, [`DEFAULT_ENTRY_SYMBOL`] when `None`, answers the plugin's
    /// entry table, and each activate is handed the host's context table of its plugin and its whole manifest, every
    /// call across both catching a panic so none unwinds through the seam. Once the host lets go of an entry, the
    /// seam's own thread tells it to free its table and closes its library when no other entry of it is loaded, after
    /// every call the plugin made into the host has returned. Answers [`Answer::Mistake`] for a path that names no
    /// library, a symbol it lacks, a symbol answering no table, or a table below its first shape;
    /// [`Answer::Destroyed`] once the destroy has been called.
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
                library: Some(Arc::new(PluginLibrary { _library: library })),
            }),
            registry: Arc::downgrade(&self.shared.seam),
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
    // SAFETY: the table reaches its first shape, and every field past it is an optional function.
    Ok(unsafe { copy_table(table) })
}

fn box_of(id: u64) -> Option<Arc<ContextBox>> {
    locked(&CONTEXT_IDS).get(&id).and_then(Weak::upgrade)
}

impl SeamEntryOf {
    fn registry(&self) -> Option<Arc<SeamRegistry>> {
        self.registry.upgrade()
    }

    /// The box of `context`'s plugin, made at its first activate; what its last open made is let go of, since the
    /// core ended it.
    fn box_for(&self, registry: &Arc<SeamRegistry>, context: &Context) -> Arc<ContextBox> {
        let mut contexts = locked(&self.contexts);
        let id = *contexts.entry(context.plugin()).or_insert_with(next_id);
        let boxed = locked(&registry.contexts)
            .entry(id)
            .or_insert_with(|| {
                Arc::new(ContextBox {
                    id,
                    context: context.clone(),
                    entry: self.entry.clone(),
                    registry: Arc::downgrade(registry),
                    registrations: Mutex::new(HashMap::new()),
                    wants: Mutex::new(HashMap::new()),
                    watches: Mutex::new(HashMap::new()),
                    revocations: Mutex::new(HashMap::new()),
                    forwarding: Mutex::new(None),
                    offered_methods: Mutex::new(HashMap::new()),
                })
            })
            .clone();
        locked(&CONTEXT_IDS).insert(id, Arc::downgrade(&boxed));
        let ended = (
            std::mem::take(&mut *locked(&boxed.registrations)),
            std::mem::take(&mut *locked(&boxed.wants)),
            std::mem::take(&mut *locked(&boxed.watches)),
            std::mem::take(&mut *locked(&boxed.revocations)),
            std::mem::take(&mut *locked(&boxed.offered_methods)),
        );
        drop(ended);
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
        hold: Some(context_hold),
        unhold: Some(context_unhold),
        offer_methods: Some(context_offer_methods),
    }
}

impl Entry for SeamEntryOf {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let (activating, settle) = pending();
        let Some(registry) = self.registry() else {
            return Pending::settled(Err("the host is gone".to_owned()));
        };
        let boxed = self.box_for(&registry, context);
        let table = context_table(&boxed);
        let manifest = LentManifest::of(context.manifest());
        let completion = registry.completion(settle, boxed.id);
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
        let registry = self.registry()?;
        let completion = registry.completion(settle, 0);
        let entry = &self.entry.table;
        // SAFETY: as for `activate`.
        let taken = unsafe { (entry.deactivate)(entry.data, &completion) };
        if taken != 0 {
            registry.forget_completion(completion.data);
            return None;
        }
        Some(deactivating)
    }

    fn actions(&self) -> Methods {
        let names = self.entry.action_names();
        methods_across(&self.entry, 0, 0, names)
    }

    fn forgotten(&self, context: &Context) {
        let Some(id) = locked(&self.contexts).remove(&context.plugin()) else {
            return;
        };
        locked(&CONTEXT_IDS).remove(&id);
        if let Some(registry) = self.registry() {
            let boxed = locked(&registry.contexts).remove(&id);
            let ended: Vec<u64> = locked(&registry.completions)
                .values()
                .filter(|record| record.context == id)
                .map(|record| record.id)
                .collect();
            for completion in ended {
                registry.forget_completion(completion);
            }
            if let Some(boxed) = boxed
                && let Some(forwarding) = locked(&boxed.forwarding).take()
            {
                forwarding.stop();
            }
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
    mapping: u64,
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
            mapping: 0,
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
            mapping: self.mapping,
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
                    registration: revocation.registration.0,
                    awaits_release: revocation.awaits_release,
                    ..crossed_event(EVENT_REVOKED, &boxed, &revocation.object)
                }
            }
            PluginEvent::HoldingsReleased { id, service } => LentEvent {
                id: id.clone(),
                ..crossed_event(EVENT_HOLDINGS_RELEASED, &boxed, service)
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
                id: id.clone(),
                refusal: Some(unmet_constraint.clone()),
                ..crossed_event(EVENT_DEGRADED, &boxed, service)
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
                ..object.map_or(LentEvent::of(kind), |object| {
                    crossed_event(kind, &boxed, object)
                })
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
    let _call = CallFromAPlugin::enter();
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
    let _call = CallFromAPlugin::enter();
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
    let _call = CallFromAPlugin::enter();
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

/// An object as it crosses: its address, and where a description bound it, that description and its library.
fn crossing(boxed: &ContextBox, object: &Object) -> Result<Answering, Answer> {
    let (description, mapping) = boxed.description_of(object);
    Ok(Answering {
        number: address_of(object) as u64,
        description,
        mapping,
        ..Answering::default()
    })
}

/// The parts of an event that carry `object` across.
fn crossed_event(kind: u32, boxed: &ContextBox, object: &Object) -> LentEvent {
    let (description, mapping) = boxed.description_of(object);
    LentEvent {
        address: address_of(object),
        description,
        mapping,
        ..LentEvent::of(kind)
    }
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
    let _call = CallFromAPlugin::enter();
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
    let _call = CallFromAPlugin::enter();
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
    let _call = CallFromAPlugin::enter();
    catch_unwind(|| box_of(data).is_some_and(|boxed| boxed.context.dispatches_here()))
        .map(u32::from)
        .unwrap_or(0)
}

unsafe extern "C" fn context_get(data: u64, id: SeamText, answer: *const SeamAnswer) {
    // SAFETY: the id is lent for the call.
    answered(data, answer, |boxed| {
        crossing(boxed, &boxed.context.get(&unsafe { text(id) })?)
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
    let answered = settlement
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|object| crossing(&boxed, object));
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
        locked(&boxed.watches).insert(token, None);
        let watched = if capability != 0 {
            boxed.context.watch_capability(&id, watcher)
        } else {
            boxed.context.watch(&id, watcher)
        };
        let watching = match watched {
            Ok(watching) => watching,
            Err(refused) => {
                locked(&boxed.watches).remove(&token);
                return Err(refused);
            }
        };
        let stopped_meanwhile = {
            let mut watches = locked(&boxed.watches);
            match watches.get_mut(&token) {
                Some(slot) => {
                    *slot = Some(watching);
                    None
                }
                None => Some(watching),
            }
        };
        if let Some(watching) = stopped_meanwhile {
            watching.stop()?;
        }
        done()
    });
}

unsafe extern "C" fn context_stop_watch(data: u64, token: u64, answer: *const SeamAnswer) {
    answered(data, answer, |boxed| {
        let watching = locked(&boxed.watches)
            .remove(&token)
            .ok_or(Answer::Absent)?;
        if let Some(watching) = watching {
            watching.stop()?;
        }
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
            boxed.foreign(address),
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
            .provide(&unsafe { text(id) }, boxed.foreign(address))?;
        done()
    });
}

unsafe extern "C" fn context_offer_methods(
    data: u64,
    address: usize,
    names: SeamTexts,
    answer: *const SeamAnswer,
) {
    answered(data, answer, |boxed| {
        // SAFETY: the names are lent for the call.
        let names = unsafe { read_texts(names) };
        locked(&boxed.offered_methods).insert(address, names);
        done()
    });
}

unsafe extern "C" fn context_implementations(data: u64, id: SeamText, answer: *const SeamAnswer) {
    answered(data, answer, |boxed| {
        // SAFETY: the id is lent for the call.
        let implementations = boxed.context.implementations(&unsafe { text(id) })?;
        Ok(Answering {
            addresses: implementations.iter().map(address_of).collect(),
            descriptions: implementations
                .iter()
                .map(|object| boxed.description_of(object).0)
                .collect(),
            mappings: implementations
                .iter()
                .map(|object| boxed.description_of(object).1)
                .collect(),
            ..Answering::default()
        })
    });
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

unsafe extern "C" fn context_hold(data: u64, mapping: u64) -> u32 {
    let _call = CallFromAPlugin::enter();
    catch_unwind(|| {
        box_of(data)
            .and_then(|boxed| boxed.registry.upgrade())
            .is_some_and(|registry| registry.hold(mapping))
    })
    .map(u32::from)
    .unwrap_or(0)
}

unsafe extern "C" fn context_unhold(data: u64, mapping: u64) {
    let _call = CallFromAPlugin::enter();
    let _ = catch_unwind(|| {
        if let Some(registry) = box_of(data).and_then(|boxed| boxed.registry.upgrade()) {
            registry.unhold(mapping);
        }
    });
}
