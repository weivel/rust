//! The seam's tables held to their sizes both ways, texts and refusals crossing whole, and what the host keeps of the
//! seam let go of at its end.

use std::env::consts::{DLL_PREFIX, DLL_SUFFIX};
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::abi::{
    ANSWERED_FIRST_SHAPE, Answering, CONTEXT_FIRST_SHAPE, ENTRY_FIRST_SHAPE, SeamAnswered,
    SeamCompletion, SeamContext, SeamEntry, SeamText, lend_answered, read_answered,
};
use super::host_side::{entry_table_of, kept_counts};
use super::seam_entry;
use crate::context::{Context, Pending};
use crate::core::{
    Answer, CoreBinary, EdgeSource, Handle, Manifest, Refusal, RefusalCode, RefusalStatus,
};
use crate::host::{Entry, Finished, Host, HostOptions, HostScheduler};

/// Reads back what `answered` lends, as the other side of the seam would.
fn crossed(answered: Result<Answering, Answer>) -> Result<Answering, Answer> {
    let mut read = None;
    // SAFETY: the answer is lent for the call that reads it.
    lend_answered(&answered, |lent| {
        read = Some(unsafe { read_answered(lent) })
    });
    read.expect("an answer is always lent")
}

/// Lends `answered`, cut to `size`, and reads it back.
fn crossed_at_size(answered: Result<Answering, Answer>, size: u32) -> Result<Answering, Answer> {
    let mut read = None;
    lend_answered(&answered, |lent| {
        // SAFETY: the copy is read only up to the `size` it is cut to, and everything it lends lives for the call.
        let mut cut = unsafe { std::ptr::read(lent) };
        cut.size = size;
        // SAFETY: as above.
        read = Some(unsafe { read_answered(&cut) });
    });
    read.expect("an answer is always lent")
}

#[test]
fn a_refusal_crosses_whole_with_texts_past_any_fixed_length() {
    let refusal = Refusal {
        code: RefusalCode::NAME_TAKEN,
        detail: "d".repeat(5000),
        fix: "f".repeat(700),
        subject: Some("s".repeat(600)),
        has_fix: true,
    };
    let refused = Answer::Refused {
        status: RefusalStatus::Refused,
        refusal: Some(refusal),
    };
    assert_eq!(crossed(Err(refused.clone())).err(), Some(refused));
    let mistake = Answer::Mistake("m".repeat(3000));
    assert_eq!(crossed(Err(mistake.clone())).err(), Some(mistake));
}

#[test]
fn an_answer_with_its_lists_crosses_whole() {
    let answered = crossed(Ok(Answering {
        number: 7,
        texts: vec!["a".to_owned(), "b".repeat(900)],
        addresses: vec![1, 2, 3],
        description: 0,
        descriptions: Vec::new(),
    }))
    .expect("success crosses as success");
    assert_eq!(answered.number, 7);
    assert_eq!(answered.texts, vec!["a".to_owned(), "b".repeat(900)]);
    assert_eq!(answered.addresses, vec![1, 2, 3]);
}

#[test]
fn an_answer_below_its_first_shape_is_a_defect_and_never_reads_as_success() {
    let short = crossed_at_size(Err(Answer::Absent), (ANSWERED_FIRST_SHAPE - 1) as u32);
    assert!(matches!(short, Err(Answer::Defect(_))), "{:?}", short.err());
    let first_shape_only = crossed_at_size(
        Err(Answer::Mistake("cut".to_owned())),
        ANSWERED_FIRST_SHAPE as u32,
    );
    assert_eq!(first_shape_only.err(), Some(Answer::Mistake(String::new())));
    let longer = crossed_at_size(
        Err(Answer::Mistake("whole".to_owned())),
        size_of::<SeamAnswered>() as u32 + 64,
    );
    assert_eq!(longer.err(), Some(Answer::Mistake("whole".to_owned())));
}

struct Settles;

impl Entry for Settles {
    fn activate(&self, _: &Context) -> Pending<Finished> {
        Pending::settled(Ok(()))
    }
}

fn plugin_entry_table() -> SeamEntry {
    let table = seam_entry(|| Arc::new(Settles));
    // SAFETY: `seam_entry` answers a whole table, freed below by its own `free`.
    unsafe { std::ptr::read(table) }
}

#[test]
fn an_entry_table_is_refused_below_its_first_shape_and_taken_longer() {
    let table = plugin_entry_table();
    let mut short = table;
    short.size = (ENTRY_FIRST_SHAPE - 1) as u32;
    assert!(matches!(
        entry_table_of(&short, "short"),
        Err(Answer::Mistake(_))
    ));
    let mut longer = table;
    longer.size = size_of::<SeamEntry>() as u32 + 32;
    assert!(entry_table_of(&longer, "longer").is_ok());
    // SAFETY: the table is the one `seam_entry` made, freed once.
    unsafe { (table.free)(table.data) };
}

static FINISHED: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());

unsafe extern "C" fn record_finish(_: u64, failed: u32, text: SeamText) {
    // SAFETY: the text is lent for the call.
    FINISHED
        .lock()
        .unwrap()
        .push((failed, unsafe { text.read() }));
}

fn host_context_table(size: u32) -> SeamContext {
    unsafe extern "C" fn write(_: u64, _: u32, _: SeamText) {}
    unsafe extern "C" fn get(_: u64, _: SeamText, _: *const super::abi::SeamAnswer) {}
    unsafe extern "C" fn listed(_: u64, _: *const super::abi::SeamAnswer) {}
    unsafe extern "C" fn want(
        _: u64,
        _: SeamText,
        _: *const super::abi::SeamWantLimits,
        _: u64,
        _: *const super::abi::SeamAnswer,
    ) {
    }
    unsafe extern "C" fn by_handle(_: u64, _: u64, _: *const super::abi::SeamAnswer) {}
    unsafe extern "C" fn watch(
        _: u64,
        _: SeamText,
        _: u32,
        _: u64,
        _: *const super::abi::SeamAnswer,
    ) {
    }
    unsafe extern "C" fn register(
        _: u64,
        _: SeamText,
        _: usize,
        _: u32,
        _: u64,
        _: *const super::abi::SeamAnswer,
    ) {
    }
    unsafe extern "C" fn provide(_: u64, _: SeamText, _: usize, _: *const super::abi::SeamAnswer) {}
    unsafe extern "C" fn dispatches_here(_: u64) -> u32 {
        0
    }
    SeamContext {
        size,
        data: 1,
        plugin: 1,
        write,
        report: write,
        get,
        ids: listed,
        want,
        withdraw: by_handle,
        watch,
        stop_watch: by_handle,
        register,
        unregister: by_handle,
        release_holding: by_handle,
        provide,
        implementations: get,
        dispatches_here,
    }
}

#[test]
fn a_context_table_is_refused_below_its_first_shape_and_taken_longer() {
    let manifest = Manifest {
        import_name: "skewed".to_owned(),
        ..Manifest::default()
    };
    let lent = super::abi::LentManifest::of(&manifest);
    let completion = SeamCompletion {
        size: size_of::<SeamCompletion>() as u32,
        data: 1,
        finish: record_finish,
    };
    for (size, failed) in [
        ((CONTEXT_FIRST_SHAPE - 1) as u32, 1),
        (size_of::<SeamContext>() as u32 + 64, 0),
    ] {
        let table = plugin_entry_table();
        FINISHED.lock().unwrap().clear();
        let context = host_context_table(size);
        // SAFETY: every table lives for the call.
        let taken = unsafe { (table.activate)(table.data, &context, &lent.table(), &completion) };
        assert_eq!(taken, 0);
        let finished = FINISHED.lock().unwrap().clone();
        assert_eq!(finished[0].0, failed, "{finished:?}");
        // SAFETY: as above.
        unsafe { (table.free)(table.data) };
    }
}

fn core_binary() -> CoreBinary {
    let library = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("build")
        .join("native")
        .join(format!("{DLL_PREFIX}weivel_core{DLL_SUFFIX}"));
    CoreBinary::load(&library).expect("the pinned core loads: run `cargo run -p native` first")
}

#[test]
fn what_the_host_keeps_of_a_plugin_across_the_seam_is_let_go_of_at_its_forget() {
    let plugins = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("build")
        .join("native")
        .join(format!("{DLL_PREFIX}weivel_test_plugins{DLL_SUFFIX}"));
    let host = Host::create(
        &core_binary(),
        HostOptions {
            scheduler: HostScheduler::NoClock,
            ..HostOptions::default()
        },
    )
    .unwrap();
    let mut plugins_registered: Vec<Handle> = Vec::new();
    for (name, symbol) in [("owner", "owning_entry"), ("panicking", "panicking_entry")] {
        let entry = host
            .load_entry(&plugins, Some(symbol))
            .expect("the plugin loads");
        let provides: &[&str] = if name == "owner" {
            &["owner:feed"]
        } else {
            &[]
        };
        let manifest = Manifest {
            import_name: name.to_owned(),
            services_provided: provides
                .iter()
                .map(|id| crate::core::ServiceOffer {
                    id: (*id).to_owned(),
                    attributes: Vec::new(),
                })
                .collect(),
            ..Manifest::default()
        };
        plugins_registered.push(host.register(manifest, entry).unwrap());
    }
    let activation = host
        .activate(&plugins_registered, EdgeSource::SERVICES)
        .unwrap();
    activation
        .outcomes
        .iter()
        .for_each(|outcome| drop(outcome.wait()));
    assert_eq!(kept_counts().0, 2);
    host.destroy().wait();
    assert_eq!(kept_counts(), (0, 0));
}

#[derive(Default)]
struct Listens {
    heard: Arc<Mutex<usize>>,
}

impl Entry for Listens {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let heard = self.heard.clone();
        context.listen(move |_| *heard.lock().unwrap() += 1);
        Pending::settled(Ok(()))
    }
}

#[test]
fn an_event_is_ignored_below_its_first_shape_and_heard_longer() {
    let listens = Arc::new(Listens::default());
    let heard = listens.heard.clone();
    let table = seam_entry(move || listens);
    // SAFETY: `seam_entry` answers a whole table, freed below by its own `free`.
    let table = unsafe { std::ptr::read(table) };
    let manifest = Manifest::default();
    let lent = super::abi::LentManifest::of(&manifest);
    let completion = SeamCompletion {
        size: size_of::<SeamCompletion>() as u32,
        data: 1,
        finish: record_finish,
    };
    let context = host_context_table(size_of::<SeamContext>() as u32);
    // SAFETY: every table lives for the call.
    unsafe { (table.activate)(table.data, &context, &lent.table(), &completion) };
    let id = String::new();
    for size in [
        (super::abi::EVENT_FIRST_SHAPE - 1) as u32,
        size_of::<super::abi::SeamEvent>() as u32 + 16,
    ] {
        let event = super::abi::SeamEvent {
            size,
            kind: super::abi::EVENT_REVOKED,
            context: context.data,
            token: 0,
            address: 8,
            registration: 3,
            awaits_release: 0,
            reach_kind: 0,
            id: SeamText::of(&id),
            refusal: super::abi::lend_refusal(None),
            settlement: std::ptr::null(),
            description: 0,
        };
        // SAFETY: the event lives for the call.
        unsafe { (table.hear)(table.data, &event) };
    }
    assert_eq!(*heard.lock().unwrap(), 1);
    // SAFETY: the table is the one `seam_entry` made, freed once.
    unsafe { (table.free)(table.data) };
}

/// Records the plugin of every context that heard an event.
#[derive(Default)]
struct RecordsWhoHeard {
    heard: Arc<Mutex<Vec<u64>>>,
}

impl Entry for RecordsWhoHeard {
    fn activate(&self, context: &Context) -> Pending<Finished> {
        let heard = self.heard.clone();
        let plugin = context.plugin().0;
        context.listen(move |_| heard.lock().unwrap().push(plugin));
        Pending::settled(Ok(()))
    }
}

#[test]
fn an_event_reaches_the_context_it_names_of_an_entry_activated_for_two_plugins() {
    let entry = Arc::new(RecordsWhoHeard::default());
    let heard = entry.heard.clone();
    let table = seam_entry(move || entry);
    // SAFETY: `seam_entry` answers a whole table, freed below by its own `free`.
    let table = unsafe { std::ptr::read(table) };
    let manifest = Manifest::default();
    let lent = super::abi::LentManifest::of(&manifest);
    let completion = SeamCompletion {
        size: size_of::<SeamCompletion>() as u32,
        data: 1,
        finish: record_finish,
    };
    let mut first = host_context_table(size_of::<SeamContext>() as u32);
    first.data = 11;
    first.plugin = 101;
    let mut second = first;
    second.data = 22;
    second.plugin = 202;
    for context in [&first, &second] {
        // SAFETY: every table lives for the call.
        unsafe { (table.activate)(table.data, context, &lent.table(), &completion) };
    }
    let id = String::new();
    let event = super::abi::SeamEvent {
        size: size_of::<super::abi::SeamEvent>() as u32,
        kind: super::abi::EVENT_REVOKED,
        context: second.data,
        token: 0,
        address: 8,
        registration: 3,
        awaits_release: 0,
        reach_kind: 0,
        id: SeamText::of(&id),
        refusal: super::abi::lend_refusal(None),
        settlement: std::ptr::null(),
        description: 0,
    };
    // SAFETY: the event lives for the call.
    unsafe { (table.hear)(table.data, &event) };
    assert_eq!(*heard.lock().unwrap(), vec![202]);
    // SAFETY: the table is the one `seam_entry` made, freed once.
    unsafe { (table.free)(table.data) };
}

#[test]
fn a_call_handed_no_answer_is_not_made_and_crashes_nothing() {
    // SAFETY: every argument lives for the call; the answer is NULL on purpose.
    unsafe { super::host_side::call_get_without_an_answer() };
}
