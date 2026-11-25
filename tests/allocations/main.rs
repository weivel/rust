//! How many heap allocations one call makes, pinned so a regression fails: a described call through a bound
//! definition and a call through a C library's bytes service. The binding's cost per call is mostly what it allocates,
//! so a count that grows is a call that slowed. Every allocation on the test's own thread is counted, and none on any
//! other, so the host's own threads cannot move the count.

#[path = "../support/mod.rs"]
mod support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;

use weivel_rust::host::{
    BoundDefinition, FunctionKind, Host, HostOptions, HostScheduler, Inputs, LibraryAnswer,
    LibraryDeclaration, LibraryFunction, LibraryService, LibraryServiceDeclaration, Members, Value,
};
use weivel_rust::standard::StandardDescriptions;

struct CountingOnThisThread;

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

fn count_one() {
    let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
}

// SAFETY: every call is passed on to the system allocator unchanged; the count is a thread-local cell, which neither
// allocates nor locks.
unsafe impl GlobalAlloc for CountingOnThisThread {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_one();
        // SAFETY: as the caller promised the system allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_one();
        // SAFETY: as above.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: as above.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count_one();
        // SAFETY: as above.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static COUNTING: CountingOnThisThread = CountingOnThisThread;

const CALLS: u64 = 1_000;

/// The allocations one call of `call` makes on average, after a warm-up of the same number of calls.
fn allocations_per_call(mut call: impl FnMut()) -> f64 {
    for _ in 0..CALLS {
        call();
    }
    let before = ALLOCATIONS.with(Cell::get);
    for _ in 0..CALLS {
        call();
    }
    (ALLOCATIONS.with(Cell::get) - before) as f64 / CALLS as f64
}

fn definition(id: &str, symbol: &str, description: Option<&str>) -> LibraryServiceDeclaration {
    LibraryServiceDeclaration {
        id: id.to_owned(),
        functions: vec![LibraryFunction {
            name: "definition".to_owned(),
            symbol: symbol.to_owned(),
            kind: FunctionKind::Definition,
            context: 0,
        }],
        description: description.map(str::to_owned),
    }
}

fn host() -> Host {
    let check = StandardDescriptions::load(support::shared_library("weivel_standard"))
        .expect("the standard's description checks load");
    Host::create(
        &support::core_binary(),
        HostOptions {
            scheduler: HostScheduler::NoClock,
            description_check: Some(Arc::new(check)),
            ..HostOptions::default()
        },
    )
    .expect("the host is created")
}

fn mirror(host: &Host) -> BoundDefinition {
    let declaration = LibraryDeclaration {
        import_name: "described".to_owned(),
        version: "1.0.0".to_owned(),
        release: None,
        services: vec![definition(
            "described-mirror",
            "described_mirror_definition",
            Some("described_mirror_description"),
        )],
        contracts: Vec::new(),
    };
    host.load_library(
        &support::shared_library("weivel_standard_described"),
        &declaration,
    )
    .expect("the described library loads")
    .described("described-mirror")
    .expect("the mirror is bound")
}

fn reflect_inputs() -> Inputs {
    let tags = ["accept", "host", "content-length"]
        .iter()
        .map(|tag| Some((*tag).to_owned()))
        .collect();
    let sample: Members = [
        ("label", Value::Text(Some("request".to_owned()))),
        ("tags", Value::Texts(tags)),
    ]
    .into_iter()
    .collect();
    Inputs::from([("sample".to_owned(), Value::Struct(sample))])
}

/// What a described call may allocate at most, as measured when it was pinned; lower it when a change lowers the count.
const MOST_PER_REFLECT: f64 = 10.0;
const MOST_PER_NEGATE: f64 = 1.0;
const MOST_PER_ECHO: f64 = 0.0;

#[test]
fn a_described_call_allocates_no_more_than_it_was_pinned_at() {
    let host = host();
    let mirror = mirror(&host);
    let reflect = reflect_inputs();
    let negate = Inputs::from([("number".to_owned(), Value::I64(5))]);
    let reflecting = allocations_per_call(|| {
        mirror.call("reflect", &reflect).expect("reflect answers");
    });
    let negating = allocations_per_call(|| {
        mirror.call("negate", &negate).expect("negate answers");
    });
    println!("allocations per call: reflect {reflecting}, negate {negating}");
    drop(mirror);
    host.destroy().wait();
    assert!(
        reflecting <= MOST_PER_REFLECT,
        "reflect allocates {reflecting} a call, over the {MOST_PER_REFLECT} pinned"
    );
    assert!(
        negating <= MOST_PER_NEGATE,
        "negate allocates {negating} a call, over the {MOST_PER_NEGATE} pinned"
    );
}

#[test]
fn a_library_call_allocates_no_more_than_it_was_pinned_at() {
    let host = host();
    let declaration = LibraryDeclaration {
        import_name: "echo".to_owned(),
        version: "0.1.0".to_owned(),
        release: Some("echo_release".to_owned()),
        services: vec![LibraryServiceDeclaration {
            id: "echo".to_owned(),
            functions: vec![LibraryFunction {
                name: "echo".to_owned(),
                symbol: "echo_echo".to_owned(),
                kind: FunctionKind::Bytes,
                context: 0,
            }],
            description: None,
        }],
        contracts: Vec::new(),
    };
    let loaded = host
        .load_library(&support::shared_library("echo"), &declaration)
        .expect("the echo library loads");
    let (_, object) = loaded.services().remove(0);
    let service = object
        .downcast_ref::<LibraryService>()
        .expect("a bytes service");
    let input = [7u8; 16];
    let echoing = allocations_per_call(|| match service.call("echo", &input) {
        LibraryAnswer::Returned(bytes) => assert_eq!(bytes.len(), input.len()),
        other => panic!("the echo answered {other:?}"),
    });
    println!("allocations per call: echo {echoing}");
    host.destroy().wait();
    assert!(
        echoing <= MOST_PER_ECHO,
        "the echo allocates {echoing} a call, over the {MOST_PER_ECHO} pinned"
    );
}
