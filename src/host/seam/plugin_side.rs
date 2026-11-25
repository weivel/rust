//! The seam's plugin side: a plugin's [`Entry`] behind the entry table its library exports, and the host's context
//! table behind the same safe [`Context`] an in-process plugin is handed.

use std::collections::HashMap;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::abi::{
    Answering, CAPACITY_FIRST_SHAPE, COMPLETION_FIRST_SHAPE, CONTEXT_FIRST_SHAPE,
    EVENT_CAPABILITY_PROVIDED, EVENT_CAPABILITY_WITHDRAWN, EVENT_DEGRADED, EVENT_FIRST_SHAPE,
    EVENT_HOLDINGS_RELEASED, EVENT_REACH_REFUSED, EVENT_REVOKED, EVENT_WANT_SETTLED,
    EVENT_WATCH_REGISTERED, EVENT_WATCH_UNREGISTERED, SeamAnswer, SeamAnswered, SeamCapacity,
    SeamCompletion, SeamContext, SeamEntry, SeamEvent, SeamManifest, SeamNumber, SeamText,
    SeamWantLimits, carries, failure_kind_code, lend_numbers, read_answered, read_manifest,
    read_numbers, read_refusal, size_of_table,
};
use super::locked;
use crate::context::{
    CapabilityWatcher, CapacityAnswerer, Context, ForeignObject, Object, PluginEvent, Revocation,
    SeamCalls, ServiceWatcher, WantLimits, WantRecord, address_of,
};
use crate::core::{Answer, FailureKind, Handle, LogLevel, ReachKind, panic_text};
use crate::host::entry::{Entry, Finished};

enum Watcher {
    Service(Arc<dyn ServiceWatcher>),
    Capability(Arc<dyn CapabilityWatcher>),
}

/// What the plugin's side keeps of the calls made through one context: its own objects the host holds only the
/// addresses of, and the wants, watchers and answerers events and questions come back to.
#[derive(Default)]
struct Kept {
    last_token: u64,
    objects: Vec<(Option<Handle>, Option<u64>, Object)>,
    wants: HashMap<u64, Arc<WantRecord>>,
    watchers: HashMap<u64, Watcher>,
    answerers: HashMap<u64, Arc<dyn CapacityAnswerer>>,
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
    closed: AtomicBool,
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
            closed: AtomicBool::new(false),
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

    fn object_at(&self, address: usize) -> Object {
        locked(&self.kept)
            .objects
            .iter()
            .find(|(_, _, object)| address_of(object) == address)
            .map(|(_, _, object)| object.clone())
            .unwrap_or_else(|| Arc::new(ForeignObject::at(address)))
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
        Ok(self.object_at(got.number as usize))
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
        let mut kept = locked(&self.kept);
        match registered {
            Ok(registered) => {
                let registration = Handle(registered.number);
                kept.objects
                    .push((Some(registration), answerer_id, service));
                Ok(registration)
            }
            Err(refused) => {
                if let Some(answerer_id) = answerer_id {
                    kept.answerers.remove(&answerer_id);
                }
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
            .filter(|(kept_registration, _, _)| *kept_registration == Some(registration))
            .filter_map(|(_, answerer, _)| *answerer)
            .collect();
        for answerer in answerers {
            kept.answerers.remove(&answerer);
        }
        kept.objects
            .retain(|(kept_registration, _, _)| *kept_registration != Some(registration));
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
        // SAFETY: as above.
        self.call(|table, answer| unsafe {
            (table.provide)(table.data, SeamText::of(id), address, answer)
        })?;
        locked(&self.kept)
            .objects
            .push((None, None, implementation));
        Ok(())
    }

    fn implementations(&self, id: &str) -> Result<Vec<Object>, Answer> {
        // SAFETY: as above.
        let listed = self.call(|table, answer| unsafe {
            (table.implementations)(table.data, SeamText::of(id), answer)
        })?;
        Ok(listed
            .addresses
            .into_iter()
            .map(|address| self.object_at(address))
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
        // SAFETY: the table reaches its first shape, every field this build reads.
        let table = unsafe { std::ptr::read(table) };
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
    // SAFETY: as the caller promised.
    let id = unsafe { event.id.read() };
    // SAFETY: as above.
    let refusal = unsafe { read_refusal(&event.refusal) };
    let told = match event.kind {
        EVENT_REVOKED => PluginEvent::Revoked(Revocation {
            object: host.object_at(event.address),
            registration: Handle(event.registration),
            awaits_release: event.awaits_release != 0,
        }),
        EVENT_HOLDINGS_RELEASED => PluginEvent::HoldingsReleased {
            id,
            service: host.object_at(event.address),
        },
        EVENT_REACH_REFUSED => PluginEvent::ReachRefused {
            kind: ReachKind(event.reach_kind),
            id,
            refusal,
        },
        EVENT_DEGRADED => PluginEvent::Degraded {
            id,
            service: host.object_at(event.address),
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
                .map(|settled| host.object_at(settled.number as usize))
        };
        want.settle(settlement);
        return;
    }
    let watcher = match locked(&host.kept).watchers.get(&event.token) {
        Some(Watcher::Service(watcher)) => Watcher::Service(watcher.clone()),
        Some(Watcher::Capability(watcher)) => Watcher::Capability(watcher.clone()),
        None => return,
    };
    let object = host.object_at(event.address);
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
