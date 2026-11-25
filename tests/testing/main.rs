//! The `testing` module's tests: a scratch home laid out and removed, variables set and put back, a described library
//! bound from its folder, and a free port.

#[path = "../support/mod.rs"]
mod support;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use weivel_rust::host::{CallStatus, DescribedAnswer, Inputs, Value};
use weivel_rust::testing::{
    DescribedLibrary, DescribedLibraryOptions, PlacedLibrary, ScratchFile, ScratchLayout,
    TestingFailure, described_library, free_port, scratch_home, set_variables,
};

use support::{repository_root, shared_library};

/// Taken by every test, so no thread reads or writes the environment while another sets it, which is what the
/// helpers' `unsafe` asks of their caller.
static ENVIRONMENT: Mutex<()> = Mutex::new(());

fn environment_turn() -> MutexGuard<'static, ()> {
    ENVIRONMENT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn variable(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// Sets or removes a variable before a test looks at what the helpers make of it.
fn prepare(name: &str, value: Option<&str>) {
    // SAFETY: every test holds the environment's turn.
    unsafe {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}

/// A folder of its own under the temporary directory, deleted when dropped.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        let path =
            std::env::temp_dir().join(format!("weivel-rust-testing-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the folder is made");
        Folder(path)
    }

    fn with(name: &str, files: &[(&str, &[u8])]) -> Folder {
        let folder = Folder::new(name);
        for (file_name, content) in files {
            std::fs::write(folder.0.join(file_name), content).expect("the file is written");
        }
        folder
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

type Files = Box<
    dyn Fn(&Path, &BTreeMap<String, PlacedLibrary>) -> Result<Vec<ScratchFile>, TestingFailure>,
>;

struct Layout {
    libraries: Vec<(String, PathBuf)>,
    files: Files,
}

const SET_VARIABLE: &str = "WEIVEL_RUST_SCRATCH_TEST_SET";
const UNSET_VARIABLE: &str = "WEIVEL_RUST_SCRATCH_TEST_UNSET";

impl Layout {
    fn plain() -> Layout {
        Layout {
            libraries: Vec::new(),
            files: Box::new(|_, _| Ok(Vec::new())),
        }
    }
}

impl ScratchLayout for Layout {
    fn prefix(&self) -> &str {
        "scratch-test-"
    }

    fn libraries(&self) -> Vec<(String, PathBuf)> {
        self.libraries.clone()
    }

    fn environment(&self, base: &Path) -> Vec<(String, OsString)> {
        vec![
            (SET_VARIABLE.to_owned(), base.join("set").into_os_string()),
            (
                UNSET_VARIABLE.to_owned(),
                base.join("unset").into_os_string(),
            ),
        ]
    }

    fn files(
        &self,
        base: &Path,
        libraries: &BTreeMap<String, PlacedLibrary>,
    ) -> Result<Vec<ScratchFile>, TestingFailure> {
        (self.files)(base, libraries)
    }
}

#[test]
fn a_scratch_home_answers_a_base_that_is_a_real_path() {
    let _turn = environment_turn();
    // SAFETY: the test holds the environment's turn.
    let home = unsafe { scratch_home(&Layout::plain()) }.expect("the home is laid out");
    let real = std::fs::canonicalize(&home.base).expect("the base resolves");
    let plain = real
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_owned();
    assert_eq!(
        home.base,
        PathBuf::from(plain),
        "the base is its own real path, written plainly"
    );
    assert!(
        home.base
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("scratch-test-")
    );
    // SAFETY: the test holds the environment's turn.
    unsafe { home.remove() };
}

#[test]
fn a_scratch_home_places_a_library_under_libraries_and_finds_its_binary_beside_its_declaration() {
    let _turn = environment_turn();
    let source = Folder::with(
        "place",
        &[("library.json", b"{}"), ("thing.bin", b"binary")],
    );
    let layout = Layout {
        libraries: vec![("thing".to_owned(), source.0.clone())],
        ..Layout::plain()
    };
    // SAFETY: the test holds the environment's turn.
    let home = unsafe { scratch_home(&layout) }.expect("the home is laid out");
    let placed = &home.libraries["thing"];
    assert_eq!(placed.directory, home.base.join("libraries").join("thing"));
    assert_eq!(placed.binary, placed.directory.join("thing.bin"));
    assert_eq!(std::fs::read(&placed.binary).unwrap(), b"binary");
    assert!(placed.directory.join("library.json").is_file());
    // SAFETY: the test holds the environment's turn.
    unsafe { home.remove() };
}

#[test]
fn a_library_with_no_binary_is_named_and_leaves_the_environment_and_no_home_behind() {
    let _turn = environment_turn();
    let source = Folder::with("no-binary", &[("library.json", b"{}")]);
    prepare(SET_VARIABLE, Some("before"));
    prepare(UNSET_VARIABLE, None);
    let layout = Layout {
        libraries: vec![("thing".to_owned(), source.0.clone())],
        ..Layout::plain()
    };
    // SAFETY: the test holds the environment's turn.
    let failure = unsafe { scratch_home(&layout) }.expect_err("a library needs a binary");
    assert!(
        failure.detail.contains(&source.0.display().to_string()),
        "{failure}"
    );
    assert_eq!(variable(SET_VARIABLE), Some("before".into()));
    assert_eq!(variable(UNSET_VARIABLE), None);
    prepare(SET_VARIABLE, None);
}

#[test]
fn a_scratch_home_writes_text_and_bytes_as_they_are_and_copies_a_file_byte_for_byte() {
    let _turn = environment_turn();
    let source = Folder::with("copy", &[("original.txt", b"copied")]);
    let original = source.0.join("original.txt");
    let layout = Layout {
        files: Box::new(move |base, _| {
            Ok(vec![
                ScratchFile::text(base.join("deep").join("text.txt"), "plain"),
                ScratchFile::Written {
                    path: base.join("deep").join("value.bin"),
                    content: vec![0, 255, 7],
                },
                ScratchFile::Copied {
                    path: base.join("other").join("copy.txt"),
                    from: original.clone(),
                },
            ])
        }),
        ..Layout::plain()
    };
    // SAFETY: the test holds the environment's turn.
    let home = unsafe { scratch_home(&layout) }.expect("the home is laid out");
    let read = |path: &[&str]| {
        std::fs::read(
            path.iter()
                .fold(home.base.clone(), |folder, part| folder.join(part)),
        )
        .unwrap()
    };
    assert_eq!(read(&["deep", "text.txt"]), b"plain");
    assert_eq!(read(&["deep", "value.bin"]), [0, 255, 7]);
    assert_eq!(read(&["other", "copy.txt"]), b"copied");
    // SAFETY: the test holds the environment's turn.
    unsafe { home.remove() };
}

#[test]
fn a_scratch_home_points_the_environment_into_it_then_restores_it_and_is_deleted() {
    let _turn = environment_turn();
    prepare(SET_VARIABLE, Some("before"));
    prepare(UNSET_VARIABLE, None);
    // SAFETY: the test holds the environment's turn.
    let home = unsafe { scratch_home(&Layout::plain()) }.expect("the home is laid out");
    assert_eq!(variable(SET_VARIABLE), Some(home.base.join("set").into()));
    assert_eq!(
        variable(UNSET_VARIABLE),
        Some(home.base.join("unset").into())
    );
    let base = home.base.clone();
    // SAFETY: the test holds the environment's turn.
    unsafe { home.remove() };
    assert_eq!(variable(SET_VARIABLE), Some("before".into()));
    assert_eq!(variable(UNSET_VARIABLE), None);
    assert!(!base.exists());
    prepare(SET_VARIABLE, None);
}

static SEEN_BASE: Mutex<Option<PathBuf>> = Mutex::new(None);

#[test]
fn a_layout_failing_partway_leaves_the_environment_as_it_was_and_no_home() {
    let _turn = environment_turn();
    prepare(SET_VARIABLE, Some("before"));
    prepare(UNSET_VARIABLE, None);
    let layout = Layout {
        files: Box::new(|base, _| {
            *SEEN_BASE.lock().unwrap() = Some(base.to_owned());
            std::fs::create_dir(base.join("partial")).unwrap();
            Err(TestingFailure {
                detail: "layout failed".to_owned(),
            })
        }),
        ..Layout::plain()
    };
    // SAFETY: the test holds the environment's turn.
    let failure = unsafe { scratch_home(&layout) }.expect_err("the layout fails");
    assert_eq!(failure.detail, "layout failed");
    assert_eq!(variable(SET_VARIABLE), Some("before".into()));
    assert_eq!(variable(UNSET_VARIABLE), None);
    let seen = SEEN_BASE
        .lock()
        .unwrap()
        .clone()
        .expect("files saw the base");
    assert!(!seen.exists());
    prepare(SET_VARIABLE, None);
}

#[test]
fn set_variables_sets_each_then_restores_a_set_one_and_removes_an_unset_one() {
    let _turn = environment_turn();
    const SET: &str = "WEIVEL_RUST_VARIABLES_TEST_SET";
    const UNSET: &str = "WEIVEL_RUST_VARIABLES_TEST_UNSET";
    prepare(SET, Some("before"));
    prepare(UNSET, None);
    // SAFETY: the test holds the environment's turn.
    let set = unsafe { set_variables(&[(SET, "during"), (UNSET, "during")]) };
    assert_eq!(variable(SET), Some("during".into()));
    assert_eq!(variable(UNSET), Some("during".into()));
    // SAFETY: the test holds the environment's turn.
    unsafe { set.restore() };
    assert_eq!(variable(SET), Some("before".into()));
    assert_eq!(variable(UNSET), None);
    prepare(SET, None);
}

#[test]
fn restoring_two_sets_in_reverse_order_puts_back_what_was_there_before_both() {
    let _turn = environment_turn();
    const NAME: &str = "WEIVEL_RUST_VARIABLES_TEST_ORDER";
    prepare(NAME, Some("before"));
    // SAFETY: the test holds the environment's turn.
    let first = unsafe { set_variables(&[(NAME, "first")]) };
    // SAFETY: the test holds the environment's turn.
    let second = unsafe { set_variables(&[(NAME, "second"), (NAME, "again")]) };
    assert_eq!(variable(NAME), Some("again".into()));
    // SAFETY: the test holds the environment's turn.
    unsafe { second.restore() };
    assert_eq!(
        variable(NAME),
        Some("first".into()),
        "a name set twice gets what it held before the first"
    );
    // SAFETY: the test holds the environment's turn.
    unsafe { first.restore() };
    assert_eq!(variable(NAME), Some("before".into()));
    prepare(NAME, None);
}

fn options() -> DescribedLibraryOptions {
    DescribedLibraryOptions {
        core_library: shared_library("weivel_core"),
        standard_library: shared_library("weivel_standard"),
    }
}

/// A folder holding the standard's described test library beside a declaration written as `declaration`.
fn described_folder(name: &str, declaration: &[u8]) -> Folder {
    let binary = shared_library("weivel_standard_described");
    let folder = Folder::new(name);
    std::fs::copy(&binary, folder.0.join(binary.file_name().unwrap()))
        .expect("the described library is copied");
    std::fs::write(folder.0.join("library.json"), declaration).expect("the declaration is written");
    folder
}

fn pinned_declaration() -> Vec<u8> {
    std::fs::read(
        repository_root()
            .join("siblings")
            .join("standard")
            .join("test")
            .join("described")
            .join("library.json"),
    )
    .expect("the pinned standard's described declaration is read")
}

fn count_releases(library: &DescribedLibrary) -> Result<DescribedAnswer, CallStatus> {
    library
        .get("described-mirror")
        .expect("the mirror is bound")
        .call("count_releases", &Inputs::new())
        .map_err(|refused| refused.status)
}

/// The cursors the library's walker has released, a count kept in the binary's own memory, so a binary closed and
/// mapped again counts from zero.
fn cursors_released(library: &DescribedLibrary) -> u64 {
    let Ok(DescribedAnswer::Answered(answers)) = count_releases(library) else {
        panic!("the mirror answers count_releases");
    };
    match answers.value() {
        Some(Value::Struct(counts)) => match counts.get("cursors") {
            Some(Value::U64(cursors)) => *cursors,
            other => panic!("count_releases counted cursors as {other:?}"),
        },
        other => panic!("count_releases answered {other:?}"),
    }
}

fn open_and_release_a_cursor(library: &DescribedLibrary) {
    let opened = library
        .get("described-walker")
        .expect("the walker is bound")
        .call(
            "open",
            &[("label".to_owned(), Value::Text(Some("walk".to_owned())))]
                .into_iter()
                .collect(),
        );
    let Ok(DescribedAnswer::Answered(answers)) = opened else {
        panic!("open answered {opened:?}");
    };
    let Some(Value::Handle(cursor)) = answers.value().cloned() else {
        panic!("open answered no cursor");
    };
    drop(answers);
    cursor.release();
}

/// Each load maps the binary at the same path, so a count left behind by the first load shows that closing it left
/// the binary mapped, as a host kept alive would, even while a bound service of it is still held.
#[test]
fn closing_a_described_library_closes_its_binary() {
    let _turn = environment_turn();
    let folder = described_folder("closes", &pinned_declaration());
    let first = described_library(&folder.0, &options()).expect("the library binds");
    open_and_release_a_cursor(&first);
    assert_eq!(cursors_released(&first), 1);
    let kept = first
        .get("described-mirror")
        .cloned()
        .expect("the mirror is bound");
    first.close();
    let second = described_library(&folder.0, &options()).expect("the library binds again");
    drop(kept);
    assert_eq!(
        cursors_released(&second),
        0,
        "the binary was mapped again, its count starting over"
    );
}

#[test]
fn a_described_library_binds_its_described_services_from_its_folder_callable() {
    let _turn = environment_turn();
    let folder = described_folder("bound", &pinned_declaration());
    let library = described_library(&folder.0, &options()).expect("the library binds");
    assert_eq!(
        library.described().keys().collect::<Vec<_>>(),
        ["described-mirror", "described-walker"]
    );
    let Ok(DescribedAnswer::Answered(answers)) = count_releases(&library) else {
        panic!("the mirror answers count_releases");
    };
    assert!(matches!(answers.value(), Some(Value::Struct(_))));
    library.close();
}

#[test]
fn a_bound_service_answers_unloaded_once_its_library_is_closed() {
    let _turn = environment_turn();
    let folder = described_folder("closed", &pinned_declaration());
    let library = described_library(&folder.0, &options()).expect("the library binds");
    let mirror = library.get("described-mirror").unwrap().clone();
    library.close();
    let refused = mirror
        .call("count_releases", &Inputs::new())
        .expect_err("a closed library is called no more");
    assert_eq!(refused.status, CallStatus::Unloaded);
}

#[test]
fn a_declaration_the_standard_refuses_is_named() {
    let _turn = environment_turn();
    let folder = described_folder("refused", b"{}");
    let failure = described_library(&folder.0, &options()).expect_err("{} declares nothing");
    let declaration = folder.0.join("library.json").display().to_string();
    assert!(failure.detail.contains(&declaration), "{failure}");
}

#[test]
fn a_free_port_is_one_a_listener_can_then_bind() {
    let port = free_port().expect("a port is free");
    assert!(port > 0);
    TcpListener::bind((Ipv4Addr::LOCALHOST, port)).expect("the free port binds");
}
