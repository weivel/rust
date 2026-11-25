//! The seam's plugin side: a plugin's [`Entry`] behind the entry table its library exports, and the host's context
//! table behind the same safe [`Context`] an in-process plugin is handed.

use std::collections::HashMap;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::abi::{
    ANSWER_FIRST_SHAPE, Answering, CAPACITY_FIRST_SHAPE, COMPLETION_FIRST_SHAPE,
    CONTEXT_FIRST_SHAPE, CONTEXT_HOLD_END, CONTEXT_OFFER_METHODS_END, EVENT_CAPABILITY_PROVIDED,
    EVENT_CAPABILITY_WITHDRAWN, EVENT_DEGRADED, EVENT_DESCRIPTION_END, EVENT_FIRST_SHAPE,
    EVENT_HOLDINGS_RELEASED, EVENT_LIBRARY_UNLOADING, EVENT_MAPPING_END, EVENT_REACH_REFUSED,
    EVENT_REVOKED, EVENT_WANT_SETTLED, EVENT_WATCH_REGISTERED, EVENT_WATCH_UNREGISTERED,
    SeamAnswer, SeamAnswered, SeamCapacity, SeamCompletion, SeamContext, SeamEntry, SeamEvent,
    SeamManifest, SeamNumber, SeamText, SeamTexts, SeamWantLimits, carries, copy_table,
    failure_kind_code, lend_answered, lend_numbers, lend_texts, read_answered, read_manifest,
    read_numbers, read_refusal, size_of_table,
};
use super::locked;
use super::trees::{answering_of, decode};
use crate::context::{
    CapabilityWatcher, CapacityAnswerer, Context, ForeignObject, Object, PluginEvent, Revocation,
    SeamCalls, ServiceWatcher, WantLimits, WantRecord, address_of,
};
use crate::core::{Answer, FailureKind, Handle, LogLevel, ReachKind, panic_text};
use crate::host::description::{BoundDefinition, Holdings, Mapping, RemoteMapping, Tree};
use crate::host::entry::{Entry, Finished};
use crate::host::methods::{MethodResult, methods_of};

enum Watcher {
    Service(Arc<dyn ServiceWatcher>),
    Capability(Arc<dyn CapabilityWatcher>),
}

/// What the plugin's side keeps of the calls made through one context: its own objects the host holds only the
/// addresses of, and the wants, watchers and answerers events and questions come back to.
#[derive(Default)]
struct Kept {
    last_token: u64,
    /// Each object with its registration once the host answered it, its answerer, and the token it was kept by.
    objects: Vec<(Option<Handle>, Option<u64>, Object, u64)>,
    wants: HashMap<u64, Arc<WantRecord>>,
    watchers: HashMap<u64, Watcher>,
    answerers: HashMap<u64, Arc<dyn CapacityAnswerer>>,
    bound: HashMap<(usize, u64), Object>,
    holdings: HashMap<u64, Arc<Holdings>>,
}

impl Kept {
    fn token(&mut self) -> u64 {
        self.last_token += 1;
        self.last_token
    }
}

/// The host's context table, copied, as the plugin's side calls it; closed once the host let go of it.
pub(super) struct HostTable {
    table: SeamContext,
    closed: Arc<AtomicBool>,
    kept: Mutex<Kept>,
}

unsafe extern "C" fn take_answered(data: *mut c_void, answered: *const SeamAnswered) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `data` is the caller's slot for the length of the call, and the answer is lent for it.
        let slot = unsafe { &mut *(data as *mut Option<Result<Answering, Answer>>) };
        *slot = Some(if answered.is_null() {
            Err(Answer::Defect("the host answered nothing".to_owned()))
        } else {
            // SAFETY: as above.
            unsafe { read_answered(&*answered) }
        });
    }));
}

impl HostTable {
    pub(super) fn of(table: SeamContext) -> HostTable {
        HostTable {
            table,
            closed: Arc::new(AtomicBool::new(false)),
            kept: Mutex::new(Kept::default()),
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let kept = std::mem::take(&mut *locked(&self.kept));
        for want in kept.wants.into_values() {
            want.settle(Err(Answer::Detached));
        }
    }

    /// Makes one call of the table, answering what the host handed its answer, or [`Answer::Destroyed`] once the
    /// host let go of the table.
    fn call(
        &self,
        call: impl FnOnce(&SeamContext, *const SeamAnswer),
    ) -> Result<Answering, Answer> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Answer::Destroyed);
        }
        let mut slot: Option<Result<Answering, Answer>> = None;
        let answer = SeamAnswer {
            size: size_of_table::<SeamAnswer>(),
            data: &mut slot as *mut Option<Result<Answering, Answer>> as *mut c_void,
            take: take_answered,
        };
        call(&self.table, &answer);
        slot.unwrap_or_else(|| Err(Answer::Defect("the host answered nothing".to_owned())))
    }

    /// Tells the host which methods its object `object` offers by name, before it crosses, where the host's table
    /// carries that call; an object offering none, or a host that cannot hear it, is told nothing.
    fn offer_methods_of(&self, object: &Object) -> Result<(), Answer> {
        let Some(methods) = methods_of(object) else {
            return Ok(());
        };
        let Some(offer_methods) = self
            .table
            .offer_methods
            .filter(|_| carries(self.table.size, CONTEXT_OFFER_METHODS_END))
        else {
            return Ok(());
        };
        let names = methods.names();
        let lent = lend_texts(&names);
        let address = address_of(object);
        // SAFETY: every argument lives for the call.
        self.call(|table, answer| unsafe {
            offer_methods(
                table.data,
                address,
                SeamTexts {
                    texts: lent.as_ptr(),
                    count: lent.len(),
                },
                answer,
            )
        })
        .map(|_| ())
    }

    /// Keeps this side's own object before it crosses, so a call back to it during the very call that hands it across,
    /// as a consumer's during that call's dispatch, finds it; answers the token it is kept by.
    fn keep(&self, registration: Option<Handle>, answerer: Option<u64>, object: Object) -> u64 {
        let mut kept = locked(&self.kept);
        let token = kept.token();
        kept.objects.push((registration, answerer, object, token));
        token
    }

    /// Lets go of an object kept for a call the host refused, and of its answerer.
    fn let_go(&self, kept_by: u64, answerer: Option<u64>) {
        let mut kept = locked(&self.kept);
        kept.objects.retain(|entry| entry.3 != kept_by);
        if let Some(answerer) = answerer {
            kept.answerers.remove(&answerer);
        }
    }

    /// Calls the method `name` of this side's own object at `address`, one it registered or provided.
    fn call_method(
        &self,
        address: usize,
        name: &str,
        arguments: Option<&Tree>,
    ) -> Option<MethodResult> {
        let object = locked(&self.kept)
            .objects
            .iter()
            .find(|(_, _, object, _)| address_of(object) == address)
            .map(|(_, _, object, _)| object.clone())?;
        methods_of(&object)?.call(name, arguments)
    }

    /// The object at `address` as this side holds it: its own, for one it handed across; the definition its
    /// description binds, bound once by this side's own binding over the host's mapping of its library, so a call or a
    /// release through it holds the binary mapped through the host and answers unloaded once it has closed; otherwise
    /// the bare address.
    fn object_at(&self, crossed: Crossed) -> Object {
        let mut kept = locked(&self.kept);
        if let Some((_, _, object, _)) = kept
            .objects
            .iter()
            .find(|(_, _, object, _)| address_of(object) == crossed.address)
        {
            return object.clone();
        }
        let bare = || -> Object { Arc::new(ForeignObject::at(crossed.address)) };
        if crossed.description == 0
            || crossed.mapping == 0
            || !carries(self.table.size, CONTEXT_HOLD_END)
        {
            return bare();
        }
        if let Some(bound) = kept.bound.get(&(crossed.address, crossed.mapping)) {
            return bound.clone();
        }
        let mapping = Arc::new(HostMapping {
            table: self.table,
            closed: self.closed.clone(),
            mapping: crossed.mapping,
        });
        let Some(mapped) = Mapping::Remote(mapping.clone()).hold() else {
            return bare();
        };
        let holdings = kept
            .holdings
            .entry(crossed.mapping)
            .or_insert_with(Holdings::of_a_library)
            .clone();
        // SAFETY: the host hands a description only beside the definition it bound from it, both answered by a library
        // the standard's check passed, and `mapped` holds that library mapped across the binding.
        let bound = unsafe {
            BoundDefinition::bind_remote(
                crossed.address,
                crossed.description,
                Mapping::Remote(mapping),
                holdings,
            )
        };
        drop(mapped);
        let object: Object = match bound {
            Ok(bound) => Arc::new(bound),
            Err(_) => bare(),
        };
        kept.bound
            .insert((crossed.address, crossed.mapping), object.clone());
        object
    }

    /// Releases everything this side's bindings got from the library `mapping` names, while the host still holds it
    /// mapped, as the host's unload does for its own.
    fn library_unloading(&self, mapping: u64) {
        let holdings = {
            let mut kept = locked(&self.kept);
            kept.bound.retain(|(_, of), _| *of != mapping);
            kept.holdings.remove(&mapping)
        };
        if let Some(holdings) = holdings {
            holdings.release_all();
        }
    }
}

/// An object as the host handed it across.
#[derive(Clone, Copy)]
struct Crossed {
    address: usize,
    description: usize,
    mapping: u64,
}

impl Crossed {
    fn answered(answered: &Answering) -> Crossed {
        Crossed {
            address: answered.number as usize,
            description: answered.description,
            mapping: answered.mapping,
        }
    }

    fn of_event(event: &SeamEvent) -> Crossed {
        let reaches = |end: usize| carries(event.size, end);
        Crossed {
            address: event.address,
            description: if reaches(EVENT_DESCRIPTION_END) {
                event.description
            } else {
                0
            },
            mapping: if reaches(EVENT_MAPPING_END) {
                event.mapping
            } else {
                0
            },
        }
    }
}

/// The host's mapping of one library, held through its context table for each call or release a binding on this
/// side makes; nothing once the host let go of the table.
struct HostMapping {
    table: SeamContext,
    closed: Arc<AtomicBool>,
    mapping: u64,
}

impl RemoteMapping for HostMapping {
    fn hold(&self) -> bool {
        // SAFETY: the call takes nothing but ids; the host answers 0 for a table or library it let go of.
        !self.closed.load(Ordering::Acquire)
            && self
                .table
                .hold
                .is_some_and(|hold| unsafe { hold(self.table.data, self.mapping) } != 0)
    }

    fn unhold(&self) {
        if let Some(unhold) = self.table.unhold {
            // SAFETY: as above.
            unsafe { unhold(self.table.data, self.mapping) };
        }
    }
}

fn lent_limits(limits: &WantLimits) -> SeamWantLimits {
    let part = |limit: Option<i64>| (u32::from(limit.is_some()), limit.unwrap_or(0));
    let (has_timeout, timeout_millis) = part(limits.timeout_millis);
    let (has_budget, budget_millis) = part(limits.budget_millis);
    let (has_poll, poll_millis) = part(limits.poll_millis);
    SeamWantLimits {
        size: size_of_table::<SeamWantLimits>(),
        has_timeout,
        has_budget,
        has_poll,
        timeout_millis,
        budget_millis,
        poll_millis,
    }
}

impl SeamCalls for HostTable {
    fn get(&self, id: &str) -> Result<Object, Answer> {
        // SAFETY: every argument lives for the call.
        let got = self
            .call(|table, answer| unsafe { (table.get)(table.data, SeamText::of(id), answer) })?;
        Ok(self.object_at(Crossed::answered(&got)))
    }

    fn ids(&self) -> Result<Vec<String>, Answer> {
        // SAFETY: as above.
        Ok(self
            .call(|table, answer| unsafe { (table.ids)(table.data, answer) })?
            .texts)
    }

    fn want(
        &self,
        id: &str,
        limits: &WantLimits,
        record: Arc<WantRecord>,
    ) -> Result<Handle, Answer> {
        let token = {
            let mut kept = locked(&self.kept);
            let token = kept.token();
            kept.wants.insert(token, record);
            token
        };
        let limits = lent_limits(limits);
        // SAFETY: as above.
        let made = self.call(|table, answer| unsafe {
            (table.want)(table.data, SeamText::of(id), &limits, token, answer)
        });
        if made.is_err() {
            locked(&self.kept).wants.remove(&token);
        }
        made.map(|_| Handle(token))
    }

    fn withdraw(&self, want: Handle) -> Result<(), Answer> {
        // SAFETY: as above.
        self.call(|table, answer| unsafe { (table.withdraw)(table.data, want.0, answer) })?;
        locked(&self.kept).wants.remove(&want.0);
        Ok(())
    }

    fn watch(&self, id: &str, watcher: Arc<dyn ServiceWatcher>) -> Result<Handle, Answer> {
        self.watch_with(id, Watcher::Service(watcher), 0)
    }

    fn watch_capability(
        &self,
        id: &str,
        watcher: Arc<dyn CapabilityWatcher>,
    ) -> Result<Handle, Answer> {
        self.watch_with(id, Watcher::Capability(watcher), 1)
    }

    fn stop_watch(&self, watch: Handle) -> Result<(), Answer> {
        // SAFETY: as above.
        self.call(|table, answer| unsafe { (table.stop_watch)(table.data, watch.0, answer) })?;
        locked(&self.kept).watchers.remove(&watch.0);
        Ok(())
    }

    fn register(
        &self,
        id: &str,
        service: Object,
        awaiting_releases: bool,
        answerer: Option<Arc<dyn CapacityAnswerer>>,
    ) -> Result<Handle, Answer> {
        let answerer_id = answerer.map(|answerer| {
            let mut kept = locked(&self.kept);
            let token = kept.token();
            kept.answerers.insert(token, answerer);
            token
        });
        let address = address_of(&service);
        let kept_by = self.keep(None, answerer_id, service.clone());
        if let Err(refused) = self.offer_methods_of(&service) {
            self.let_go(kept_by, answerer_id);
            return Err(refused);
        }
        // SAFETY: as above.
        let registered = self.call(|table, answer| unsafe {
            (table.register)(
                table.data,
                SeamText::of(id),
                address,
                u32::from(awaiting_releases),
                answerer_id.unwrap_or(0),
                answer,
            )
        });
        match registered {
            Ok(registered) => {
                let registration = Handle(registered.number);
                let mut kept = locked(&self.kept);
                if let Some(entry) = kept.objects.iter_mut().find(|entry| entry.3 == kept_by) {
                    entry.0 = Some(registration);
                }
                Ok(registration)
            }
            Err(refused) => {
                self.let_go(kept_by, answerer_id);
                Err(refused)
            }
        }
    }

    fn unregister(&self, registration: Handle) -> Result<(), Answer> {
        // SAFETY: as above.
        self.call(|table, answer| unsafe {
            (table.unregister)(table.data, registration.0, answer)
        })?;
        let mut kept = locked(&self.kept);
        let answerers: Vec<u64> = kept
            .objects
            .iter()
            .filter(|(kept_registration, _, _, _)| *kept_registration == Some(registration))
            .filter_map(|(_, answerer, _, _)| *answerer)
            .collect();
        for answerer in answerers {
            kept.answerers.remove(&answerer);
        }
        kept.objects
            .retain(|(kept_registration, _, _, _)| *kept_registration != Some(registration));
        Ok(())
    }

    fn release_holding(&self, revocation: &Revocation) -> Result<bool, Answer> {
        if !revocation.awaits_release {
            return Ok(false);
        }
        // SAFETY: as above.
        let released = self.call(|table, answer| unsafe {
            (table.release_holding)(table.data, revocation.registration.0, answer)
        })?;
        Ok(released.number != 0)
    }

    fn provide(&self, id: &str, implementation: Object) -> Result<(), Answer> {
        let address = address_of(&implementation);
        let kept_by = self.keep(None, None, implementation.clone());
        // SAFETY: as above.
        let provided = self.offer_methods_of(&implementation).and_then(|()| {
            self.call(|table, answer| unsafe {
                (table.provide)(table.data, SeamText::of(id), address, answer)
            })
        });
        if provided.is_err() {
            self.let_go(kept_by, None);
        }
        provided.map(|_| ())
    }

    fn implementations(&self, id: &str) -> Result<Vec<Object>, Answer> {
        // SAFETY: as above.
        let listed = self.call(|table, answer| unsafe {
            (table.implementations)(table.data, SeamText::of(id), answer)
        })?;
        let descriptions = listed.descriptions;
        let mappings = listed.mappings;
        Ok(listed
            .addresses
            .into_iter()
            .enumerate()
            .map(|(at, address)| {
                self.object_at(Crossed {
                    address,
                    description: descriptions.get(at).copied().unwrap_or(0),
                    mapping: mappings.get(at).copied().unwrap_or(0),
                })
            })
            .collect())
    }

    fn write(&self, level: LogLevel, text: &str) {
        if !self.closed.load(Ordering::Acquire) {
            // SAFETY: the text lives for the call.
            unsafe { (self.table.write)(self.table.data, level.0, SeamText::of(text)) };
        }
    }

    fn report(&self, kind: FailureKind, text: &str) {
        if !self.closed.load(Ordering::Acquire) {
            // SAFETY: the text lives for the call.
            unsafe {
                (self.table.report)(self.table.data, failure_kind_code(kind), SeamText::of(text))
            };
        }
    }

    fn dispatches_here(&self) -> bool {
        // SAFETY: the call takes nothing but the table's id.
        !self.closed.load(Ordering::Acquire)
            && unsafe { (self.table.dispatches_here)(self.table.data) } != 0
    }
}

impl HostTable {
    fn watch_with(&self, id: &str, watcher: Watcher, capability: u32) -> Result<Handle, Answer> {
        let token = {
            let mut kept = locked(&self.kept);
            let token = kept.token();
            kept.watchers.insert(token, watcher);
            token
        };
        // SAFETY: every argument lives for the call.
        let made = self.call(|table, answer| unsafe {
            (table.watch)(table.data, SeamText::of(id), capability, token, answer)
        });
        if made.is_err() {
            locked(&self.kept).watchers.remove(&token);
        }
        made.map(|_| Handle(token))
    }
}

/// The host's completion of one activate or deactivate, copied; finished at most once, by the host's own rule.
#[derive(Clone, Copy)]
struct Finisher(SeamCompletion);

impl Finisher {
    fn finish(&self, finished: &Finished) {
        let (failed, text) = match finished {
            Ok(()) => (0, ""),
            Err(failure) => (1, failure.as_str()),
        };
        // SAFETY: the text lives for the call; the host's `finish` takes nothing else of the plugin's.
        unsafe { (self.0.finish)(self.0.data, failed, SeamText::of(text)) };
    }
}

/// A plugin's entry behind its library's entry table, and each context the host handed it, by the table's id. The
/// table is its first field, so the address the host is answered is the side's own.
#[repr(C)]
struct PluginSide {
    table: SeamEntry,
    entry: Arc<dyn Entry>,
    contexts: Mutex<HashMap<u64, (Arc<HostTable>, Context)>>,
}

impl PluginSide {
    fn context_of(&self, id: u64) -> Option<(Arc<HostTable>, Context)> {
        locked(&self.contexts).get(&id).cloned()
    }

    /// The context over the host's `table`, the same one for every activate the host hands that table.
    ///
    /// # Safety
    /// `table` and `manifest` are lent for the length of the call.
    unsafe fn context_for(
        &self,
        table: *const SeamContext,
        manifest: *const SeamManifest,
    ) -> Result<Context, String> {
        if table.is_null() || manifest.is_null() {
            return Err("the host handed no context table or no manifest".to_owned());
        }
        // SAFETY: as the caller promised; the `size` is read before anything past it.
        let size = unsafe { (*table).size };
        if !carries(size, CONTEXT_FIRST_SHAPE) {
            return Err(format!(
                "the host's context table is {size} bytes, below the {CONTEXT_FIRST_SHAPE} of its first shape"
            ));
        }
        // SAFETY: the table reaches its first shape, and every field past it is an optional function.
        let table = unsafe { copy_table(table) };
        let mut contexts = locked(&self.contexts);
        if let Some((_, context)) = contexts.get(&table.data) {
            return Ok(context.clone());
        }
        // SAFETY: as the caller promised.
        let manifest = unsafe { read_manifest(&*manifest) }
            .ok_or_else(|| "the host's manifest is below its first shape".to_owned())?;
        let host = Arc::new(HostTable::of(table));
        let context = Context::across_the_seam(host.clone(), Handle(table.plugin), manifest);
        contexts.insert(table.data, (host, context.clone()));
        Ok(context)
    }
}

/// The entry table of `make`'s entry, for a plugin's library to answer from its entry symbol, as
/// [`crate::export_entry`] writes it. Every call across the table catches a panic: one in the plugin's activate or
/// deactivate is told to the host's error sink and finishes it as failed, so the host quarantines it and goes on.
/// Answers NULL when `make` panics. The table lives until the host calls its `free`.
pub fn seam_entry<E: Entry + 'static>(make: impl FnOnce() -> Arc<E>) -> *const SeamEntry {
    let Ok(entry) = catch_unwind(AssertUnwindSafe(make)) else {
        return std::ptr::null();
    };
    let side = Box::into_raw(Box::new(PluginSide {
        table: SeamEntry {
            size: size_of_table::<SeamEntry>(),
            data: std::ptr::null(),
            activate: plugin_activate,
            deactivate: plugin_deactivate,
            hear: plugin_hear,
            capacity: plugin_capacity,
            forget_context: plugin_forget_context,
            free: plugin_free,
            action_names: Some(plugin_action_names),
            call: Some(plugin_call),
        },
        entry,
        contexts: Mutex::new(HashMap::new()),
    }));
    // SAFETY: the side was just made and is the plugin's own until its `free`.
    unsafe {
        (*side).table.data = side as *const c_void;
        &(*side).table
    }
}

fn side_of<'a>(data: *const c_void) -> &'a PluginSide {
    // SAFETY: `data` is the side `seam_entry` made, which the host calls only until its `free`.
    unsafe { &*(data as *const PluginSide) }
}

fn panicked(context: Option<&Context>, what: &str, payload: &(dyn std::any::Any + Send)) -> String {
    let text = format!(
        "a plugin's {what} panicked at the plugin seam: {}",
        panic_text(payload)
    );
    if let Some(context) = context {
        context.report(FailureKind::PluginCallback, text.clone());
    }
    text
}

/// # Safety
/// `completion` is lent for the length of the call, or NULL.
unsafe fn finisher_of(completion: *const SeamCompletion) -> Option<Finisher> {
    if completion.is_null() {
        return None;
    }
    // SAFETY: as the caller promised; the `size` is read before anything past it.
    if !carries(unsafe { (*completion).size }, COMPLETION_FIRST_SHAPE) {
        return None;
    }
    // SAFETY: the completion reaches its first shape.
    Some(Finisher(unsafe { std::ptr::read(completion) }))
}

unsafe extern "C" fn plugin_activate(
    data: *const c_void,
    table: *const SeamContext,
    manifest: *const SeamManifest,
    completion: *const SeamCompletion,
) -> u32 {
    // SAFETY: the completion is lent for the call.
    let Some(finisher) = (unsafe { finisher_of(completion) }) else {
        return 1;
    };
    let ran = catch_unwind(AssertUnwindSafe(|| {
        let side = side_of(data);
        // SAFETY: the table and the manifest are lent for the call.
        let context = match unsafe { side.context_for(table, manifest) } {
            Ok(context) => context,
            Err(refused) => return finisher.finish(&Err(refused)),
        };
        match catch_unwind(AssertUnwindSafe(|| side.entry.activate(&context))) {
            Ok(activating) => activating.on_settled(move |finished| finisher.finish(finished)),
            Err(payload) => finisher.finish(&Err(panicked(Some(&context), "activate", &*payload))),
        }
    }));
    if ran.is_err() {
        finisher.finish(&Err("the plugin's side of the seam panicked".to_owned()));
    }
    0
}

unsafe extern "C" fn plugin_deactivate(
    data: *const c_void,
    completion: *const SeamCompletion,
) -> u32 {
    // SAFETY: the completion is lent for the call.
    let Some(finisher) = (unsafe { finisher_of(completion) }) else {
        return 1;
    };
    let offered = catch_unwind(AssertUnwindSafe(|| {
        let side = side_of(data);
        match catch_unwind(AssertUnwindSafe(|| side.entry.deactivate())) {
            Ok(None) => false,
            Ok(Some(deactivating)) => {
                deactivating.on_settled(move |finished| finisher.finish(finished));
                true
            }
            Err(payload) => {
                let any = locked(&side.contexts)
                    .values()
                    .next()
                    .map(|(_, context)| context.clone());
                finisher.finish(&Err(panicked(any.as_ref(), "deactivate", &*payload)));
                true
            }
        }
    }));
    match offered {
        Ok(true) => 0,
        _ => 1,
    }
}

/// # Safety
/// The event is lent for the length of the call.
unsafe fn heard(host: &HostTable, context: &Context, event: &SeamEvent) {
    if event.kind == EVENT_LIBRARY_UNLOADING {
        return host.library_unloading(Crossed::of_event(event).mapping);
    }
    // SAFETY: as the caller promised.
    let id = unsafe { event.id.read() };
    // SAFETY: as above.
    let refusal = unsafe { read_refusal(&event.refusal) };
    let told = match event.kind {
        EVENT_REVOKED => PluginEvent::Revoked(Revocation {
            object: host.object_at(Crossed::of_event(event)),
            registration: Handle(event.registration),
            awaits_release: event.awaits_release != 0,
        }),
        EVENT_HOLDINGS_RELEASED => PluginEvent::HoldingsReleased {
            id,
            service: host.object_at(Crossed::of_event(event)),
        },
        EVENT_REACH_REFUSED => PluginEvent::ReachRefused {
            kind: ReachKind(event.reach_kind),
            id,
            refusal,
        },
        EVENT_DEGRADED => PluginEvent::Degraded {
            id,
            service: host.object_at(Crossed::of_event(event)),
            unmet_constraint: refusal,
        },
        // SAFETY: as above.
        _ => return unsafe { heard_by_a_token(host, context, event) },
    };
    context.tell(&told);
}

/// A want's settlement or a watched change, handed to what the plugin's own token names.
///
/// # Safety
/// The event is lent for the length of the call.
unsafe fn heard_by_a_token(host: &HostTable, context: &Context, event: &SeamEvent) {
    if event.kind == EVENT_WANT_SETTLED {
        let Some(want) = locked(&host.kept).wants.remove(&event.token) else {
            return;
        };
        let settlement = if event.settlement.is_null() {
            Err(Answer::Defect(
                "the host settled a want with nothing".to_owned(),
            ))
        } else {
            // SAFETY: as the caller promised.
            unsafe { read_answered(&*event.settlement) }
                .map(|settled| host.object_at(Crossed::answered(&settled)))
        };
        want.settle(settlement);
        return;
    }
    let watcher = match locked(&host.kept).watchers.get(&event.token) {
        Some(Watcher::Service(watcher)) => Watcher::Service(watcher.clone()),
        Some(Watcher::Capability(watcher)) => Watcher::Capability(watcher.clone()),
        None => return,
    };
    let object = host.object_at(Crossed::of_event(event));
    let text = "a plugin's watcher panicked";
    let call: Box<dyn FnOnce()> = match (watcher, event.kind) {
        (Watcher::Service(watcher), EVENT_WATCH_REGISTERED) => {
            Box::new(move || watcher.registered(&object))
        }
        (Watcher::Service(watcher), EVENT_WATCH_UNREGISTERED) => {
            Box::new(move || watcher.unregistered())
        }
        (Watcher::Capability(watcher), EVENT_CAPABILITY_PROVIDED) => {
            Box::new(move || watcher.provided(&object))
        }
        (Watcher::Capability(watcher), EVENT_CAPABILITY_WITHDRAWN) => {
            Box::new(move || watcher.withdrawn(&object))
        }
        _ => return,
    };
    context
        .inner
        .link
        .call_back(call, FailureKind::PluginCallback, text);
}

unsafe extern "C" fn plugin_hear(data: *const c_void, event: *const SeamEvent) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if event.is_null() {
            return;
        }
        // SAFETY: the event is lent for the call, its `size` read before anything past it.
        let event = unsafe { &*event };
        if !carries(event.size, EVENT_FIRST_SHAPE) {
            return;
        }
        let Some((host, context)) = side_of(data).context_of(event.context) else {
            return;
        };
        // SAFETY: as above.
        unsafe { heard(&host, &context, event) };
    }));
}

unsafe extern "C" fn plugin_capacity(
    data: *const c_void,
    context: u64,
    answerer: u64,
    question: u32,
    constraints: *const SeamNumber,
    count: usize,
    answer: *const SeamCapacity,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the answer is lent for the call, its `size` read before anything past it.
        if answer.is_null() || !carries(unsafe { (*answer).size }, CAPACITY_FIRST_SHAPE) {
            return;
        }
        let Some((host, plugin_context)) = side_of(data).context_of(context) else {
            return;
        };
        let Some(answering) = locked(&host.kept).answerers.get(&answerer).cloned() else {
            return;
        };
        // SAFETY: the constraints are lent for the call.
        let constraints = unsafe { read_numbers(constraints, count) };
        let mut answered = None;
        plugin_context.inner.link.call_back(
            || {
                answered = Some(match question {
                    1 => (false, answering.attributes()),
                    _ => (answering.serves(&constraints), Vec::new()),
                })
            },
            FailureKind::PluginCallback,
            "a capacity answerer panicked, so its question went unanswered",
        );
        let Some((serves, attributes)) = answered else {
            return;
        };
        let lent = lend_numbers(&attributes);
        // SAFETY: the answer and everything lent live for the call.
        unsafe { ((*answer).take)((*answer).data, u32::from(serves), lent.as_ptr(), lent.len()) };
    }));
}

/// Answers a call of the entry table to the host's answer: a call handed no answer it can take is not made.
fn answer_the_host(answer: *const SeamAnswer, call: impl FnOnce() -> Result<Answering, Answer>) {
    // SAFETY: a non-NULL answer begins with its `size`.
    if answer.is_null() || !carries(unsafe { (*answer).size }, ANSWER_FIRST_SHAPE) {
        return;
    }
    // SAFETY: the answer reaches its first shape, and is the host's for the length of the call.
    let answer = unsafe { &*answer };
    let outcome = catch_unwind(AssertUnwindSafe(call)).unwrap_or_else(|payload| {
        Err(Answer::Defect(format!(
            "the plugin's side of the seam panicked: {}",
            panic_text(&*payload)
        )))
    });
    // SAFETY: the answer's `take` is the host's, called once with what this call answered.
    lend_answered(&outcome, |lent| unsafe { (answer.take)(answer.data, lent) });
}

unsafe extern "C" fn plugin_action_names(data: *const c_void, answer: *const SeamAnswer) {
    answer_the_host(answer, || {
        Ok(Answering {
            texts: side_of(data).entry.actions().names(),
            ..Answering::default()
        })
    });
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn plugin_call(
    data: *const c_void,
    context: u64,
    address: usize,
    name: SeamText,
    bytes: *const u8,
    byte_count: usize,
    has_arguments: u32,
    answer: *const SeamAnswer,
) {
    answer_the_host(answer, || {
        // SAFETY: the name and the bytes are lent for the call.
        let name = unsafe { name.read() };
        let arguments = match has_arguments {
            0 => None,
            // SAFETY: as above.
            _ => Some(
                decode(unsafe { super::abi::lent(bytes, byte_count) }).map_err(Answer::Mistake)?,
            ),
        };
        let side = side_of(data);
        let result = if address == 0 {
            side.entry.actions().call(&name, arguments.as_ref())
        } else {
            side.context_of(context)
                .and_then(|(host, _)| host.call_method(address, &name, arguments.as_ref()))
        };
        answering_of(result)
    });
}

unsafe extern "C" fn plugin_forget_context(data: *const c_void, context: u64) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let forgotten = locked(&side_of(data).contexts).remove(&context);
        if let Some((host, context)) = forgotten {
            context.detach();
            host.close();
        }
    }));
}

unsafe extern "C" fn plugin_free(data: *const c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the host frees the side once, after which nothing calls the table.
        let side = unsafe { Box::from_raw(data as *mut PluginSide) };
        for (host, context) in locked(&side.contexts).drain().map(|(_, kept)| kept) {
            context.detach();
            host.close();
        }
        drop(side);
    }));
}
