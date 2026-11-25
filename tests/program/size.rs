//! The size and the startup of `weivel-host` built with the `smallest` profile, once with `panic = "unwind"` and once
//! with `panic = "abort"`, beside an empty program built the same way, printed for `docs/building.md`. It judges
//! nothing but that each build succeeds and each run of the host ends as its provider says.
//!
//! A target without the standard harness, so its lines reach the log of a run that passes.

#[path = "../support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const RUNS: usize = 21;
const EMPTY_RUN_EXIT: i32 = 5;
const PANIC_STRATEGIES: [&str; 2] = ["unwind", "abort"];

fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

fn smallest_profile_settings(panic_strategy: &str) -> [String; 2] {
    [
        "--config".to_owned(),
        format!("profile.smallest.panic=\"{panic_strategy}\""),
    ]
}

/// Builds `binary` of the manifest at `manifest` with the `smallest` profile and `panic_strategy`, answering its path.
fn built(manifest: &Path, binary: &str, panic_strategy: &str, target: &Path) -> PathBuf {
    let status = cargo()
        .arg("build")
        .arg("--manifest-path")
        .arg(manifest)
        .args(["--profile", "smallest", "--bin", binary, "--offline"])
        .args(smallest_profile_settings(panic_strategy))
        .arg("--target-dir")
        .arg(target)
        .status()
        .expect("cargo runs");
    assert!(
        status.success(),
        "{binary} builds with panic = {panic_strategy}"
    );
    target
        .join("smallest")
        .join(format!("{binary}{}", std::env::consts::EXE_SUFFIX))
}

/// An empty program in a crate of its own, whose `smallest` profile is the repository's.
fn empty_program_manifest(folder: &Path) -> PathBuf {
    let profile = std::fs::read_to_string(support::repository_root().join("Cargo.toml"))
        .expect("the manifest is read");
    let start = profile
        .find("[profile.smallest]")
        .expect("the manifest names the smallest profile");
    let section: String = profile[start..]
        .lines()
        .take_while(|line| !line.is_empty())
        .map(|line| format!("{line}\n"))
        .collect();
    std::fs::create_dir_all(folder.join("src")).expect("the empty program's folder is made");
    std::fs::write(folder.join("src").join("main.rs"), "fn main() {}\n")
        .expect("main.rs is written");
    let manifest = folder.join("Cargo.toml");
    std::fs::write(
        &manifest,
        format!(
            "[package]\nname = \"empty\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n{section}\n[workspace]\n"
        ),
    )
    .expect("the empty program's manifest is written");
    manifest
}

/// A run directory whose plan names one plugin with no manifest, so the run starts, refuses it, fails the action and
/// stops: every step of a run but a plugin's own.
fn empty_run_directory(folder: &Path) -> PathBuf {
    std::fs::create_dir_all(folder).expect("the run directory is made");
    std::fs::write(
        folder.join("stub-host.json"),
        format!(
            r#"{{"mode": "action", "exit": {EMPTY_RUN_EXIT}, "action": {{"plugin": "absent", "name": "none"}},
                "plugins": [{{"name": "absent", "directory": "absent", "manifest": "absent/manifest.json",
                "entry": "absent"}}]}}"#
        ),
    )
    .expect("the plan is written");
    folder.to_owned()
}

fn one_run(host: &Path, run_directory: &Path) -> Duration {
    let started = Instant::now();
    let status = Command::new(host)
        .arg("--core")
        .arg(support::shared_library("weivel_core"))
        .arg("--standard")
        .arg(support::shared_library("weivel_standard"))
        .arg(support::stub_host_library())
        .arg(run_directory)
        .status()
        .expect("weivel-host runs");
    let elapsed = started.elapsed();
    assert_eq!(
        status.code(),
        Some(EMPTY_RUN_EXIT),
        "the empty run ends with stop's code"
    );
    elapsed
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// The median and the least of [`RUNS`] runs, after one run that warms the file cache.
fn timed_runs(host: &Path, run_directory: &Path) -> (Duration, Duration) {
    one_run(host, run_directory);
    let mut durations: Vec<Duration> = (0..RUNS).map(|_| one_run(host, run_directory)).collect();
    durations.sort();
    (durations[RUNS / 2], durations[0])
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).expect("the binary is there").len()
}

fn main() {
    let measured = support::repository_root().join("build").join("size");
    let empty_manifest = empty_program_manifest(&measured.join("empty"));
    let run_directory = empty_run_directory(&measured.join("run"));
    let host_manifest = support::repository_root().join("Cargo.toml");
    println!(
        "weivel-host, profile smallest, on {} {}, the empty run's median and least of {RUNS}:",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    for panic_strategy in PANIC_STRATEGIES {
        let host = built(
            &host_manifest,
            "weivel-host",
            panic_strategy,
            &measured.join(format!("host-{panic_strategy}")),
        );
        let empty = built(
            &empty_manifest,
            "empty",
            panic_strategy,
            &measured.join(format!("empty-{panic_strategy}")),
        );
        let (median, least) = timed_runs(&host, &run_directory);
        println!(
            "size panic={panic_strategy} host={} empty={} added={} run-median-ms={:.1} run-least-ms={:.1}",
            file_size(&host),
            file_size(&empty),
            file_size(&host) as i64 - file_size(&empty) as i64,
            milliseconds(median),
            milliseconds(least)
        );
    }
}
