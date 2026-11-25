//! The seam's host side: a plugin's library opened by path, its entry table taken from the symbol its caller names,
//! and the host's context table each of its activates is handed.

use std::collections::HashMap;
use std::ffi::{CString, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::abi::{
    EVENT_HOLDINGS_RELEASED, EVENT_REVOKED, SeamAnswer, SeamCompletion, SeamContext, SeamEntry,
    SeamEvent, SeamText, size_of_table,
};
use super::{failure_kind_of, locked};
use crate::context::{
    Context, ForeignObject, Listening, Object, Pending, PluginEvent, Revocation,
    ServiceRegistration, Settle, address_of, pending,
};
use crate::core::binary::SharedLibrary;
use crate::core::{Answer, Handle, LogLevel, panic_text};
use crate::host::entry::{Entry, Finished};
use crate::host::state::Host;

/// The entry symbol a plugin's library exports when its caller names none.
pub const DEFAULT_ENTRY_SYMBOL: &str = "weivel_rust_entry";

/// A pointer the seam's tables hand across, which lives as long as what owns it.
struct Shared<T>(*const T);

// SAFETY: each pointer named this way is to a table or a box that lives while the plugin's library is open, and
// every function behind it may be called from any thread, as the seam's tables promise.
unsafe impl<T> Send for Shared<T> {}
// SAFETY: as above.
unsafe impl<T> Sync for Shared<T> {}

impl<T> Shared<T> {
    fn pointer(&self) -> *const T {
        self.0
    }
}

/// A completion the host handed an activate or a deactivate, kept until the host is freed, since the plugin may
/// finish it as late as it likes.
#[repr(C)]
struct CompletionBox {
    table: SeamCompletion,
    settle: Mutex<Option<Settle<Finished>>>,
}

/// The host's context table of one plugin across the seam, and what it keeps of the calls made through it.
#[repr(C)]
struct ContextBox {
    table: SeamContext,
    context: Context,
    import_name: String,
    registrations: Mutex<HashMap<u64, ServiceRegistration>>,
    revocations: Mutex<HashMap<u64, Revocation>>,
    forwarding: Mutex<Option<Listening>>,
}

// SAFETY: a context box's raw pointers point into itself, at its own text and as its table's `data`, and everything
// it changes is behind its mutexes.
unsafe impl Send for ContextBox {}
// SAFETY: as above.
unsafe impl Sync for ContextBox {}

// SAFETY: a completion box's one raw pointer is its table's `data`, which points at itself; its settle is behind a
// mutex.
unsafe impl Send for CompletionBox {}
// SAFETY: as above.
unsafe impl Sync for CompletionBox {}

/// A plugin's library and everything the host handed across into it, kept until the host is freed: a plugin's code
/// may still run after its release. Each table is boxed so its address stays where the plugin was handed it.
pub(crate) struct PluginLibrary {
    _library: SharedLibrary,
    contexts: Mutex<HashMap<Handle, Box<ContextBox>>>,
    #[allow(clippy::vec_box)]
    completions: Mutex<Vec<Box<CompletionBox>>>,
}

/// A plugin's entry across the seam, as the host's [`Entry`].
struct SeamEntryOf {
    library: Arc<PluginLibrary>,
    table: Shared<SeamEntry>,
}

impl Host {
    /// Loads a plugin built as a library of its own, a Rust `cdylib` among them, and answers its entry, which the host
    /// registers as any other: the library's `symbol`, [`DEFAULT_ENTRY_SYMBOL`] when `None`, answers the plugin's
    /// entry table, and each activate is handed the host's context table of its plugin, every call across both
    /// catching a panic so none unwinds through the seam. The library stays open until the host is freed. Answers
    /// [`Answer::Mistake`] for a path that names no library, a symbol it lacks, a symbol answering no table, or a
    /// table shorter than this build's; [`Answer::Destroyed`] once the destroy has been called.
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
        if table.is_null() {
            return Err(Answer::Mistake(format!(
                "the entry symbol {symbol} answered no entry table"
            )));
        }
        // SAFETY: a non-NULL entry table begins with its `size`.
        if unsafe { (*table).size } < size_of_table::<SeamEntry>() {
            return Err(Answer::Mistake(format!(
                "the entry table {symbol} answered is shorter than this build's, so it was built against an older seam"
            )));
        }
        let library = Arc::new(PluginLibrary {
            _library: library,
            contexts: Mutex::new(HashMap::new()),
            completions: Mutex::new(Vec::new()),
        });
        locked(&self.shared.plugin_libraries).push(library.clone());
        Ok(Arc::new(SeamEntryOf {
            library,
            table: Shared(table),
        }))
    }
}

impl SeamEntryOf {
    fn table(&self) -> &SeamEntry {
        // SAFETY: the table lives while the library is open, which `self.library` keeps it.
        unsafe { &*self.table.0 }
    }

    fn completion(&self, settle: Settle<Finished>) -> *const SeamCompletion {
        let mut boxed = Box::new(CompletionBox {
            table: SeamCompletion {
                size: size_of_table::<SeamCompletion>(),
                data: std::ptr::null(),
                finish: finish_completion,
            },
            settle: Mutex::new(Some(settle)),
        });
        boxed.table.data = &*boxed as *const CompletionBox as *const c_void;
        let table = &boxed.table as *const SeamCompletion;
        locked(&self.library.completions).push(boxed);
        table
    }

    /// The host's context table of `context`'s plugin, made at its first activate, its events forwarded from now.
    fn context_table(&self, context: &Context) -> *const SeamContext {
        let mut contexts = locked(&self.library.contexts);
        let boxed = contexts
            .entry(context.plugin())
            .or_insert_with(|| context_box(context));
        let table = &boxed.table as *const SeamContext;
        let forwarding = forward_events(context, Shared(self.table.0), Shared(&**boxed));
        if let Some(previous) = locked(&boxed.forwarding).replace(forwarding) {
            previous.stop();
        }
        table
    }
}

impl Entry for SeamEntryOf {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let (activating, settle) = pending();
        let table = self.table();
        let context_table = self.context_table(context);
        let completion = self.completion(settle);
        // SAFETY: the tables live while the library is open; the plugin's side catches every panic.
        let taken = unsafe { (table.activate)(table.data, context_table, completion) };
        if taken != 0 {
            settle_completion(
                completion,
                Err("the plugin's entry table refused its activate".to_owned()),
            );
        }
        activating
    }

    fn deactivate(&self) -> Option<Pending<Finished>> {
        let (deactivating, settle) = pending();
        let table = self.table();
        let completion = self.completion(settle);
        // SAFETY: as for `activate`.
        let taken = unsafe { (table.deactivate)(table.data, completion) };
        (taken == 0).then_some(deactivating)
    }
}

fn context_box(context: &Context) -> Box<ContextBox> {
    let mut boxed = Box::new(ContextBox {
        table: SeamContext {
            size: size_of_table::<SeamContext>(),
            data: std::ptr::null(),
            plugin: context.plugin().0,
            import_name: SeamText::of(""),
            write: context_write,
            report: context_report,
            get: context_get,
            register: context_register,
            unregister: context_unregister,
            release_holding: context_release_holding,
            provide: context_provide,
        },
        context: context.clone(),
        import_name: context.manifest().import_name.clone(),
        registrations: Mutex::new(HashMap::new()),
        revocations: Mutex::new(HashMap::new()),
        forwarding: Mutex::new(None),
    });
    boxed.table.data = &*boxed as *const ContextBox as *const c_void;
    boxed.table.import_name = SeamText::of(&boxed.import_name);
    boxed
}

/// Forwards what the core reports about the plugin to its side of the seam: the revocations and the releases of what
/// it registered awaiting them, which this table carries.
fn forward_events(
    context: &Context,
    entry: Shared<SeamEntry>,
    boxed: Shared<ContextBox>,
) -> Listening {
    context.listen(move |event| {
        // SAFETY: both live while the plugin's library is open, which the host keeps until it is freed.
        let (entry, boxed) = unsafe { (&*entry.pointer(), &*boxed.pointer()) };
        let (kind, address, registration, awaits_release, id) = match event {
            PluginEvent::Revoked(revocation) => {
                locked(&boxed.revocations).insert(revocation.registration.0, revocation.clone());
                let address = address_of(&revocation.object);
                (
                    EVENT_REVOKED,
                    address,
                    revocation.registration.0,
                    revocation.awaits_release,
                    String::new(),
                )
            }
            PluginEvent::HoldingsReleased { id, service } => (
                EVENT_HOLDINGS_RELEASED,
                address_of(service),
                0,
                false,
                id.clone(),
            ),
            _ => return,
        };
        let told = SeamEvent {
            size: size_of_table::<SeamEvent>(),
            kind,
            address,
            registration,
            awaits_release: u32::from(awaits_release),
            id: SeamText::of(&id),
        };
        // SAFETY: the event lives for the call; the plugin's side catches every panic.
        unsafe { (entry.hear)(entry.data, &told) };
    })
}

fn settle_completion(completion: *const SeamCompletion, finished: Finished) {
    // SAFETY: a completion's `data` is its box, kept until the host is freed.
    let boxed = unsafe { &*((*completion).data as *const CompletionBox) };
    if let Some(settle) = locked(&boxed.settle).take() {
        settle.settle(finished);
    }
}

unsafe extern "C" fn finish_completion(data: *const c_void, failed: u32, text: SeamText) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `data` is the completion's own box, kept until the host is freed.
        let boxed = unsafe { &*(data as *const CompletionBox) };
        // SAFETY: the text is lent for the call.
        let finished = if failed == 0 {
            Ok(())
        } else {
            Err(unsafe { text.read() })
        };
        if let Some(settle) = locked(&boxed.settle).take() {
            settle.settle(finished);
        }
    }));
}

/// Runs a call of the context table on its box, writing a panic into the answer as the crate's defect.
fn answered(
    data: *const c_void,
    answer: *mut SeamAnswer,
    call: impl FnOnce(&ContextBox) -> Result<(), Answer>,
) {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `data` is the table's own box, kept until the host is freed.
        call(unsafe { &*(data as *const ContextBox) })
    }))
    .unwrap_or_else(|payload| {
        Err(Answer::Defect(format!(
            "the host's side of the plugin seam panicked: {}",
            panic_text(&*payload)
        )))
    });
    if let Err(refused) = outcome
        && !answer.is_null()
    {
        // SAFETY: the answer is the caller's, of the size it states, for the length of the call.
        unsafe { (*answer).write(&refused) };
    }
}

fn boxed_of<'a>(data: *const c_void) -> Option<&'a ContextBox> {
    // SAFETY: `data` is the table's own box, kept until the host is freed.
    (!data.is_null()).then(|| unsafe { &*(data as *const ContextBox) })
}

unsafe extern "C" fn context_write(data: *const c_void, level: u32, text: SeamText) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(boxed) = boxed_of(data) {
            // SAFETY: the text is lent for the call.
            boxed
                .context
                .log()
                .write(LogLevel(level), unsafe { text.read() });
        }
    }));
}

unsafe extern "C" fn context_report(data: *const c_void, kind: u32, text: SeamText) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(boxed) = boxed_of(data) {
            // SAFETY: the text is lent for the call.
            boxed
                .context
                .report(failure_kind_of(kind), unsafe { text.read() });
        }
    }));
}

unsafe extern "C" fn context_get(
    data: *const c_void,
    id: SeamText,
    address: *mut usize,
    answer: *mut SeamAnswer,
) {
    answered(data, answer, |boxed| {
        // SAFETY: the id is lent for the call.
        let object = boxed.context.get(&unsafe { id.read() })?;
        // SAFETY: the out pointer is the caller's, for the length of the call.
        unsafe { *address = address_of(&object) };
        Ok(())
    });
}

fn foreign(address: usize) -> Object {
    Arc::new(ForeignObject::at(address))
}

unsafe extern "C" fn context_register(
    data: *const c_void,
    id: SeamText,
    address: usize,
    awaiting_releases: u32,
    registration: *mut u64,
    answer: *mut SeamAnswer,
) {
    answered(data, answer, |boxed| {
        // SAFETY: the id is lent for the call.
        let id = unsafe { id.read() };
        let registered = if awaiting_releases != 0 {
            boxed
                .context
                .register_awaiting_releases(&id, foreign(address))?
        } else {
            boxed.context.register(&id, foreign(address))?
        };
        // SAFETY: the out pointer is the caller's, for the length of the call.
        unsafe { *registration = registered.handle().0 };
        locked(&boxed.registrations).insert(registered.handle().0, registered);
        Ok(())
    });
}

unsafe extern "C" fn context_unregister(
    data: *const c_void,
    registration: u64,
    answer: *mut SeamAnswer,
) {
    answered(data, answer, |boxed| {
        let registered = locked(&boxed.registrations)
            .remove(&registration)
            .ok_or_else(|| {
                Answer::Mistake("the plugin unregistered a registration it never made".to_owned())
            })?;
        registered.unregister()
    });
}

unsafe extern "C" fn context_release_holding(
    data: *const c_void,
    registration: u64,
    released: *mut u32,
    answer: *mut SeamAnswer,
) {
    answered(data, answer, |boxed| {
        let revocation = locked(&boxed.revocations).remove(&registration);
        let let_go = match revocation {
            Some(revocation) => boxed.context.release_holding(&revocation)?,
            None => false,
        };
        // SAFETY: the out pointer is the caller's, for the length of the call.
        unsafe { *released = u32::from(let_go) };
        Ok(())
    });
}

unsafe extern "C" fn context_provide(
    data: *const c_void,
    id: SeamText,
    address: usize,
    answer: *mut SeamAnswer,
) {
    answered(data, answer, |boxed| {
        // SAFETY: the id is lent for the call.
        boxed
            .context
            .provide(&unsafe { id.read() }, foreign(address))
    });
}
