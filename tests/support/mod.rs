//! What every test target shares: the native files `scripts/native` gathers, the kit's layer vectors, and a listener
//! and a sink that record.

#![allow(dead_code)]

use std::env::consts::{DLL_PREFIX, DLL_SUFFIX, EXE_SUFFIX};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use weivel_rust::core::{
    Core, CoreBinary, CoreOptions, ErrorSink, Event, EventPayload, Failure, Listener, Wrapping,
};

/// The repository's root, where `siblings/` and `build/` lie.
pub fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// A file `scripts/native` gathered into `build/native`.
pub fn native_file(name: &str) -> PathBuf {
    let path = repository_root().join("build").join("native").join(name);
    assert!(
        path.is_file(),
        "{} is missing: run `cargo run -p native` first, which builds it",
        path.display()
    );
    path
}

/// The shared library CMake builds under `name`.
pub fn shared_library(name: &str) -> PathBuf {
    native_file(&format!("{DLL_PREFIX}{name}{DLL_SUFFIX}"))
}

/// A program `scripts/native` gathered as `name`.
pub fn program(name: &str) -> PathBuf {
    native_file(&format!("{name}{EXE_SUFFIX}"))
}

/// The pinned standard's JSON reader, bound from its shared library.
pub fn standard_json() -> weivel_rust::standard::StandardJson {
    weivel_rust::standard::StandardJson::load(shared_library("weivel_standard"))
        .expect("the pinned standard loads")
}

/// The pinned kit's layer vectors, under `siblings/conformance`.
pub fn layer_vector_directory() -> PathBuf {
    let directory = repository_root()
        .join("siblings")
        .join("conformance")
        .join("vectors")
        .join("layer");
    assert!(
        directory.is_dir(),
        "{} is missing: link or clone the conformance kit at the commit repo.json pins",
        directory.display()
    );
    directory
}

/// The pinned core's binary, loaded.
pub fn core_binary() -> CoreBinary {
    CoreBinary::load(shared_library("weivel_core")).expect("the pinned core loads")
}

/// Every event a listener heard, in the order it heard them.
#[derive(Clone, Default)]
pub struct Heard {
    pub events: Arc<Mutex<Vec<Event>>>,
}

impl Heard {
    pub fn listener(&self) -> Listener {
        let events = self.events.clone();
        Arc::new(move |_core: &Core, event: &Event| events.lock().unwrap().push(event.clone()))
    }

    pub fn all(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }

    pub fn payloads(&self) -> Vec<EventPayload> {
        self.all().into_iter().map(|event| event.payload).collect()
    }

    pub fn clear(&self) {
        self.events.lock().unwrap().clear();
    }
}

/// Every failure a sink was told.
#[derive(Default)]
pub struct Told {
    pub failures: Mutex<Vec<Failure>>,
}

impl ErrorSink for Told {
    fn report(&self, failure: &Failure) {
        self.failures.lock().unwrap().push(failure.clone());
    }
}

impl Told {
    pub fn all(&self) -> Vec<Failure> {
        self.failures.lock().unwrap().clone()
    }
}

/// A core of the pinned binary with a recording listener and sink.
pub fn recorded_core(options: &CoreOptions) -> (Core, Heard, Arc<Told>) {
    let heard = Heard::default();
    let told = Arc::new(Told::default());
    let wrapping = Wrapping {
        listener: Some(heard.listener()),
        sink: Some(told.clone()),
    };
    let core = Core::create(&core_binary(), options, wrapping).expect("the core is created");
    (core, heard, told)
}
