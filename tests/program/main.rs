//! The `program` module's tests beside the `host` vectors: the `weivel-host` command line run as a child, a terminate
//! signal ending its run, and a running host answering the service a plugin registered.

#[path = "../support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use weivel_rust::conformance::{
    HOST_VECTOR_ENTRY_SYMBOL, PLUGIN_WORDS_DIRECTORY, point_runs_at, run_pointer_file,
};
use weivel_rust::host::{Tree, methods_of};
use weivel_rust::program::{FAILED_RUN_EXIT, RunHostOptions, start_host};

/// Taken by each test that points the test plugins at a run, so none sees another's pointer file between its own
/// pointer's drop and its look at the file.
static POINTING_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn run_directory(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("weivel-rust-program-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the run directory is made");
    directory
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("a file lies in a directory"))
        .expect("its directory is made");
    std::fs::write(path, text).expect("the file is written");
}

fn json_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn record(directory: &Path) -> Vec<String> {
    std::fs::read_to_string(directory.join("record.txt"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn weivel_host() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_weivel-host"));
    command
        .arg("--core")
        .arg(support::shared_library("weivel_core"))
        .arg("--standard")
        .arg(support::shared_library("weivel_standard"))
        .arg(support::stub_host_library());
    command
}

#[test]
fn the_command_line_without_a_host_library_exits_failed() {
    let status = Command::new(env!("CARGO_BIN_EXE_weivel-host"))
        .status()
        .expect("weivel-host runs");
    assert_eq!(status.code(), Some(FAILED_RUN_EXIT as i32));
}

/// The plugin's entry is the test plugins library's default one, which fails its activation for want of a probe the
/// host offers, so the action the plan names runs on no activated plugin and reaches `finished` as a failure.
#[test]
fn the_command_line_runs_an_action_plan_to_its_end_and_exits_with_stops_code() {
    let directory = run_directory("action");
    write(
        &directory.join("provider/manifest.json"),
        r#"{"api": 1, "importName": "provider", "language": "rust", "entry": "entry",
            "services": {"provides": ["provider:probe"], "consumes": ["host:probe"]}}"#,
    );
    write(
        &directory.join("stub-host.json"),
        &format!(
            r#"{{"mode": "action", "exit": 9, "action": {{"plugin": "provider", "name": "none"}},
                "plugins": [{{"name": "provider", "directory": "provider", "manifest": "provider/manifest.json",
                "entry": "{}"}}]}}"#,
            json_text(&support::shared_library("weivel_test_plugins"))
        ),
    );
    let status = weivel_host()
        .arg(&directory)
        .status()
        .expect("weivel-host runs");
    assert_eq!(status.code(), Some(9));
    assert_eq!(
        record(&directory),
        [
            "start",
            "open provider empty",
            "started 1",
            "outcome provider failed detail",
            "finished failure",
            "stop",
        ]
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[cfg(unix)]
#[test]
fn sigterm_to_the_command_line_stops_its_serve_run() {
    stops_its_serve_run_on("-TERM", "sigterm");
}

#[cfg(unix)]
#[test]
fn sigint_to_the_command_line_stops_its_serve_run() {
    stops_its_serve_run_on("-INT", "sigint");
}

#[test]
fn an_exit_code_past_255_wraps_modulo_256() {
    assert_eq!(weivel_rust::program::process_exit_code(7), 7);
    assert_eq!(weivel_rust::program::process_exit_code(255), 255);
    assert_eq!(weivel_rust::program::process_exit_code(256), 0);
    assert_eq!(weivel_rust::program::process_exit_code(300), 44);
}

#[cfg(unix)]
fn stops_its_serve_run_on(signal: &str, name: &str) {
    use std::time::{Duration, Instant};
    const BOUND: Duration = Duration::from_secs(30);
    let directory = run_directory(name);
    write(
        &directory.join("stub-host.json"),
        r#"{"mode": "serve", "exit": 4, "plugins": []}"#,
    );
    let mut child = weivel_host()
        .arg(&directory)
        .spawn()
        .expect("weivel-host starts");
    let started = Instant::now();
    while !record(&directory).iter().any(|line| line == "tick 1 first") {
        assert!(
            started.elapsed() < BOUND,
            "the run never ticked: {:?}",
            record(&directory)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let killed = Command::new("kill")
        .args([signal, &child.id().to_string()])
        .status()
        .expect("kill runs");
    assert!(killed.success());
    let status = child.wait().expect("weivel-host ends");
    assert_eq!(status.code(), Some(4));
    assert_eq!(
        record(&directory),
        ["start", "started 0", "tick 1 first", "stop"]
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_running_host_answers_the_service_its_plugin_registered_until_it_stops() {
    let _alone = POINTING_TESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let directory = run_directory("service");
    write(
        &directory.join("alpha/manifest.json"),
        r#"{"api": 1, "importName": "alpha", "language": "rust", "entry": "entry",
            "services": {"provides": ["alpha:accounts"], "consumes": []}}"#,
    );
    write(
        &directory.join(PLUGIN_WORDS_DIRECTORY).join("alpha.json"),
        r#"{"name": "alpha", "provides": {"alpha:accounts": {"login": {"echo": true}}}}"#,
    );
    write(
        &directory.join("stub-host.json"),
        &format!(
            r#"{{"mode": "serve", "exit": 6, "plugins": [{{"name": "alpha", "directory": "alpha",
                "manifest": "alpha/manifest.json", "entry": "{}"}}]}}"#,
            json_text(&support::shared_library("weivel_test_plugins"))
        ),
    );
    let standard = support::shared_library("weivel_standard");
    let _pointer = point_runs_at(&standard, &directory).expect("the run is pointed at");
    let mut options = RunHostOptions::new(
        support::stub_host_library(),
        support::shared_library("weivel_core"),
        standard,
    );
    options.arguments = vec![directory.to_string_lossy().into_owned()];
    options.entry_symbol = Some(HOST_VECTOR_ENTRY_SYMBOL.to_owned());
    let running = start_host(options);
    let accounts = running
        .service("alpha", "alpha:accounts")
        .expect("alpha's own service is answered");
    let methods = methods_of(&accounts).expect("the service offers its methods by name");
    let handed = Tree::Text("someone".to_owned());
    assert_eq!(
        methods.call("login", Some(&handed)),
        Some(Ok(Some(handed.clone())))
    );
    assert!(running.service("alpha", "alpha:other").is_none());
    assert!(running.service("beta", "alpha:accounts").is_none());
    assert_eq!(running.stop().wait(), 6);
    assert!(running.service("alpha", "alpha:accounts").is_none());
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_run_pointer_waits_for_the_run_before_it_and_is_removed_with_it() {
    let _alone = POINTING_TESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    use std::sync::atomic::{AtomicBool, Ordering};
    let standard = support::shared_library("weivel_standard");
    let first = point_runs_at(&standard, Path::new("first")).expect("the first run is pointed at");
    assert!(run_pointer_file().is_file());
    let second_pointed = std::sync::Arc::new(AtomicBool::new(false));
    let pointed = second_pointed.clone();
    let second = std::thread::spawn(move || {
        let pointer = point_runs_at(
            &support::shared_library("weivel_standard"),
            Path::new("second"),
        )
        .expect("the second run is pointed at");
        pointed.store(true, Ordering::SeqCst);
        let read = std::fs::read_to_string(run_pointer_file()).unwrap_or_default();
        drop(pointer);
        read
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        !second_pointed.load(Ordering::SeqCst),
        "the second run was pointed at while the first stood"
    );
    drop(first);
    let read = second.join().expect("the second run ends");
    assert!(read.ends_with("second\n"), "{read:?}");
    assert!(!run_pointer_file().exists());
}
