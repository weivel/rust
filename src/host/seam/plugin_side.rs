//! The seam's plugin side: a plugin's [`Entry`] behind the entry table its library exports, and the host's context
//! table behind the same safe [`Context`] an in-process plugin is handed.

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use super::abi::{
    EVENT_HOLDINGS_RELEASED, EVENT_REVOKED, SeamAnswer, SeamCompletion, SeamContext, SeamEntry,
    SeamEvent, SeamText, size_of_table,
};
use super::{failure_kind_code, locked};
use crate::context::{Context, ForeignObject, Object, PluginEvent, Revocation, SeamCalls};
use crate::core::{Answer, FailureKind, Handle, LogLevel, Manifest, panic_text};
use crate::host::entry::{Entry, Finished};

/// The host's context table, as the plugin's side calls it.
struct HostTable(*const SeamContext);

// SAFETY: the host keeps the table until it is freed, and each of its functions may be called from any thread.
unsafe impl Send for HostTable {}
// SAFETY: as above.
unsafe impl Sync for HostTable {}

impl HostTable {
    fn table(&self) -> &SeamContext {
        // SAFETY: as above.
        unsafe { &*self.0 }
    }

    fn call(&self, call: impl FnOnce(&SeamContext, *mut SeamAnswer)) -> Result<(), Answer> {
        let mut answer = SeamAnswer::empty();
        call(self.table(), &mut answer);
        answer.read()
    }
}

impl SeamCalls for HostTable {
    fn get(&self, id: &str) -> Result<usize, Answer> {
        let mut address = 0usize;
        // SAFETY: every argument lives for the call.
        self.call(|table, answer| unsafe {
            (table.get)(table.data, SeamText::of(id), &mut address, answer)
        })?;
        Ok(address)
    }

    fn register(
        &self,
        id: &str,
        address: usize,
        awaiting_releases: bool,
    ) -> Result<Handle, Answer> {
        let mut registration = 0u64;
        // SAFETY: as above.
        self.call(|table, answer| unsafe {
            (table.register)(
                table.data,
                SeamText::of(id),
                address,
                u32::from(awaiting_releases),
                &mut registration,
                answer,
            )
        })?;
        Ok(Handle(registration))
    }

    fn unregister(&self, registration: Handle) -> Result<(), Answer> {
        // SAFETY: as above.
        self.call(|table, answer| unsafe { (table.unregister)(table.data, registration.0, answer) })
    }

    fn release_holding(&self, registration: Handle) -> Result<bool, Answer> {
        let mut released = 0u32;
        // SAFETY: as above.
        self.call(|table, answer| unsafe {
            (table.release_holding)(table.data, registration.0, &mut released, answer)
        })?;
        Ok(released != 0)
    }

    fn provide(&self, id: &str, address: usize) -> Result<(), Answer> {
        // SAFETY: as above.
        self.call(|table, answer| unsafe {
            (table.provide)(table.data, SeamText::of(id), address, answer)
        })
    }

    fn write(&self, level: LogLevel, text: &str) {
        let table = self.table();
        // SAFETY: the text lives for the call.
        unsafe { (table.write)(table.data, level.0, SeamText::of(text)) };
    }

    fn report(&self, kind: FailureKind, text: &str) {
        let table = self.table();
        // SAFETY: the text lives for the call.
        unsafe { (table.report)(table.data, failure_kind_code(kind), SeamText::of(text)) };
    }
}

/// The host's completion of one activate or deactivate, finished once.
struct Finisher(*const SeamCompletion);

// SAFETY: the host keeps a completion until it is freed, and its `finish` may be called from any thread.
unsafe impl Send for Finisher {}

impl Finisher {
    fn finish(&self, finished: &Finished) {
        // SAFETY: as above.
        let completion = unsafe { &*self.0 };
        let (failed, text) = match finished {
            Ok(()) => (0, ""),
            Err(failure) => (1, failure.as_str()),
        };
        // SAFETY: the text lives for the call.
        unsafe { (completion.finish)(completion.data, failed, SeamText::of(text)) };
    }
}

/// A plugin's entry behind its library's entry table, and the context of its current open.
struct PluginSide {
    entry: Arc<dyn Entry>,
    context: Mutex<Option<(usize, Context)>>,
}

impl PluginSide {
    /// The context over `table`, the same one for every activate the host hands that table.
    fn context_for(&self, table: *const SeamContext) -> Result<Context, String> {
        let mut current = locked(&self.context);
        if let Some((kept, context)) = current.as_ref()
            && *kept == table as usize
        {
            return Ok(context.clone());
        }
        // SAFETY: the host's table begins with its `size`, read before anything past it.
        let host = unsafe { &*table };
        if host.size < size_of_table::<SeamContext>() {
            return Err("the host's context table is shorter than this plugin's seam".to_owned());
        }
        // SAFETY: the import name lives while the table does.
        let import_name = unsafe { host.import_name.read() };
        let context = Context::across_the_seam(
            Arc::new(HostTable(table)),
            Handle(host.plugin),
            Manifest {
                import_name,
                ..Manifest::default()
            },
        );
        *current = Some((table as usize, context.clone()));
        Ok(context)
    }

    fn current(&self) -> Option<Context> {
        locked(&self.context)
            .as_ref()
            .map(|(_, context)| context.clone())
    }
}

/// The entry table of `make`'s entry, for a plugin's library to answer from its entry symbol, as
/// [`crate::export_entry`] writes it. Every call across the table catches a panic: one in the plugin's activate or
/// deactivate is told to the host's error sink and finishes it as failed, so the host quarantines it and goes on.
/// Answers NULL when `make` panics. The table lives for the process, since the host may call it until it is freed.
pub fn seam_entry<E: Entry + 'static>(make: impl FnOnce() -> Arc<E>) -> *const SeamEntry {
    let Ok(entry) = catch_unwind(AssertUnwindSafe(make)) else {
        return std::ptr::null();
    };
    let side: &'static PluginSide = Box::leak(Box::new(PluginSide {
        entry,
        context: Mutex::new(None),
    }));
    let table: &'static SeamEntry = Box::leak(Box::new(SeamEntry {
        size: size_of_table::<SeamEntry>(),
        data: side as *const PluginSide as *const c_void,
        activate: plugin_activate,
        deactivate: plugin_deactivate,
        hear: plugin_hear,
    }));
    table
}

fn side_of<'a>(data: *const c_void) -> &'a PluginSide {
    // SAFETY: `data` is the side `seam_entry` leaked for the table.
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

unsafe extern "C" fn plugin_activate(
    data: *const c_void,
    table: *const SeamContext,
    completion: *const SeamCompletion,
) -> u32 {
    let finisher = Finisher(completion);
    let taken = catch_unwind(AssertUnwindSafe(|| {
        let side = side_of(data);
        let context = match side.context_for(table) {
            Ok(context) => context,
            Err(refused) => {
                finisher.finish(&Err(refused));
                return;
            }
        };
        match catch_unwind(AssertUnwindSafe(|| side.entry.activate(&context))) {
            Ok(activating) => {
                let finisher = Finisher(finisher.0);
                activating.on_settled(move |finished| finisher.finish(finished));
            }
            Err(payload) => finisher.finish(&Err(panicked(Some(&context), "activate", &*payload))),
        }
    }));
    u32::from(taken.is_err())
}

unsafe extern "C" fn plugin_deactivate(
    data: *const c_void,
    completion: *const SeamCompletion,
) -> u32 {
    let finisher = Finisher(completion);
    let offered = catch_unwind(AssertUnwindSafe(|| {
        let side = side_of(data);
        match catch_unwind(AssertUnwindSafe(|| side.entry.deactivate())) {
            Ok(None) => false,
            Ok(Some(deactivating)) => {
                let finisher = Finisher(finisher.0);
                deactivating.on_settled(move |finished| finisher.finish(finished));
                true
            }
            Err(payload) => {
                let current = side.current();
                finisher.finish(&Err(panicked(current.as_ref(), "deactivate", &*payload)));
                true
            }
        }
    }));
    match offered {
        Ok(true) => 0,
        _ => 1,
    }
}

fn object_at(context: &Context, address: usize) -> Object {
    context
        .crossed_at(address)
        .unwrap_or_else(|| Arc::new(ForeignObject::at(address)))
}

unsafe extern "C" fn plugin_hear(data: *const c_void, event: *const SeamEvent) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(context) = side_of(data).current() else {
            return;
        };
        // SAFETY: the event is lent for the call and begins with its `size`.
        let event = unsafe { &*event };
        if event.size < size_of_table::<SeamEvent>() {
            return;
        }
        let heard = match event.kind {
            EVENT_REVOKED => PluginEvent::Revoked(Revocation {
                object: object_at(&context, event.address),
                registration: Handle(event.registration),
                awaits_release: event.awaits_release != 0,
            }),
            EVENT_HOLDINGS_RELEASED => PluginEvent::HoldingsReleased {
                // SAFETY: the id is lent for the call.
                id: unsafe { event.id.read() },
                service: object_at(&context, event.address),
            },
            _ => return,
        };
        context.tell(&heard);
    }));
}
