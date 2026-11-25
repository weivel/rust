//! Builds the native siblings the tests load, with CMake in Release, and gathers what they built into `build/native`:
//! the core's shared library and its `echo` test library, the standard's shared library and its `described` test
//! library, the core's and the standard's `layouts` programs, and the kit's `stub-host` with its declarations beside
//! it in `build/native/stub-host`; then builds the test plugins the suite loads across the plugin seam with Cargo, and
//! gathers their library there too.

use std::env::consts::{DLL_PREFIX, DLL_SUFFIX, EXE_SUFFIX};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// One target a sibling's CMake project builds: the file CMake writes for it and the name it is gathered under.
struct BuiltTarget {
    target: &'static str,
    file_name: String,
    gathered_name: String,
}

/// One sibling's CMake project and what is built of it.
struct NativeSibling {
    /// The project's directory, under `siblings/`.
    project: &'static str,
    /// The build directory's name under `build/`, kept short since MSBuild's paths below it are long.
    build_name: &'static str,
    targets: Vec<BuiltTarget>,
    /// The siblings whose checkouts the project is configured with, each as `-DWEIVEL_<NAME>_SOURCE_DIR`.
    source_siblings: &'static [&'static str],
    /// Where under `build/native` what it built is gathered.
    gathered_under: &'static str,
    /// Files of the project copied beside what it built.
    companions: &'static [&'static str],
}

fn shared_library(target: &'static str, name: &str) -> BuiltTarget {
    let file_name = format!("{DLL_PREFIX}{name}{DLL_SUFFIX}");
    BuiltTarget {
        target,
        gathered_name: file_name.clone(),
        file_name,
    }
}

fn program(target: &'static str, gathered_as: &str) -> BuiltTarget {
    BuiltTarget {
        target,
        file_name: format!("{target}{EXE_SUFFIX}"),
        gathered_name: format!("{gathered_as}{EXE_SUFFIX}"),
    }
}

fn native_siblings() -> Vec<NativeSibling> {
    vec![
        NativeSibling {
            project: "core",
            build_name: "core",
            targets: vec![
                shared_library("weivel_core_shared", "weivel_core"),
                shared_library("echo", "echo"),
                program("layouts", "core-layouts"),
            ],
            source_siblings: &[],
            gathered_under: "",
            companions: &[],
        },
        NativeSibling {
            project: "standard",
            build_name: "standard",
            targets: vec![
                shared_library("weivel_standard_shared", "weivel_standard"),
                shared_library("weivel_standard_described", "weivel_standard_described"),
                program("layouts", "standard-layouts"),
            ],
            source_siblings: &["core"],
            gathered_under: "",
            companions: &[],
        },
        NativeSibling {
            project: "conformance/test/stub-host",
            build_name: "sh",
            targets: vec![shared_library("stub-host", "stub-host")],
            source_siblings: &["standard", "core"],
            gathered_under: "stub-host",
            companions: &[
                "library.json",
                "library-declares-log.json",
                "library-declares-a-contract.json",
            ],
        },
    ]
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn run_cmake(arguments: &[String]) -> Result<(), String> {
    let status = Command::new("cmake").args(arguments).status().map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            "cmake is not on the PATH. Building the core and the standard needs CMake 3.15 or later and a C compiler."
                .to_owned()
        } else {
            format!("cmake could not be started: {error}")
        }
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cmake {} failed: {status}", arguments.join(" ")))
    }
}

fn require_sibling(root: &Path, project: &str) -> Result<PathBuf, String> {
    let directory = root.join("siblings").join(project);
    if directory.join("CMakeLists.txt").is_file() {
        return Ok(directory);
    }
    let sibling = project.split('/').next().unwrap_or(project);
    Err(format!(
        "siblings/{project} is missing. CI checks it out from repo.json; locally, link or clone the {sibling} \
         checkout there at the commit repo.json pins."
    ))
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn find_built(build_directory: &Path, file_name: &str) -> Result<PathBuf, String> {
    let candidates = [
        build_directory.join("bin").join("Release").join(file_name),
        build_directory.join("bin").join(file_name),
    ];
    candidates
        .iter()
        .find(|candidate| candidate.is_file())
        .cloned()
        .ok_or_else(|| {
            format!(
                "{file_name} was not built; looked for {} and {}",
                candidates[0].display(),
                candidates[1].display()
            )
        })
}

fn configure_arguments(
    root: &Path,
    sibling: &NativeSibling,
    source: &Path,
    build_directory: &Path,
) -> Vec<String> {
    let mut arguments = vec![
        "-S".to_owned(),
        path_text(source),
        "-B".to_owned(),
        path_text(build_directory),
        "-DCMAKE_BUILD_TYPE=Release".to_owned(),
    ];
    for source_sibling in sibling.source_siblings {
        arguments.push(format!(
            "-DWEIVEL_{}_SOURCE_DIR={}",
            source_sibling.to_uppercase(),
            path_text(&root.join("siblings").join(source_sibling))
        ));
    }
    arguments
}

fn build_arguments(sibling: &NativeSibling, build_directory: &Path) -> Vec<String> {
    let mut arguments = vec![
        "--build".to_owned(),
        path_text(build_directory),
        "--config".to_owned(),
        "Release".to_owned(),
    ];
    arguments.push("--target".to_owned());
    arguments.extend(sibling.targets.iter().map(|built| built.target.to_owned()));
    arguments.push("--parallel".to_owned());
    arguments
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    fs::copy(from, to).map(|_| ()).map_err(|error| {
        format!(
            "{} could not be copied to {}: {error}",
            from.display(),
            to.display()
        )
    })
}

fn build_sibling(root: &Path, sibling: &NativeSibling, gathered: &Path) -> Result<(), String> {
    let source = require_sibling(root, sibling.project)?;
    let build_directory = root.join("build").join(sibling.build_name);
    let gathered = gathered.join(sibling.gathered_under);
    fs::create_dir_all(&gathered)
        .map_err(|error| format!("{} could not be made: {error}", gathered.display()))?;
    run_cmake(&configure_arguments(
        root,
        sibling,
        &source,
        &build_directory,
    ))?;
    run_cmake(&build_arguments(sibling, &build_directory))?;
    for built in &sibling.targets {
        let from = find_built(&build_directory, &built.file_name)?;
        copy(&from, &gathered.join(&built.gathered_name))?;
    }
    for companion in sibling.companions {
        copy(&source.join(companion), &gathered.join(companion))?;
    }
    Ok(())
}

/// Where Cargo writes what it builds: `CARGO_TARGET_DIR`, read against the repository's root when relative, or the
/// workspace's `target`.
fn cargo_target_directory(root: &Path) -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(directory) => root.join(directory),
        None => root.join("target"),
    }
}

fn build_the_test_plugins(root: &Path, gathered: &Path) -> Result<(), String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(root)
        .args(["build", "--package", "weivel-test-plugins"])
        .status()
        .map_err(|error| format!("cargo could not be started: {error}"))?;
    if !status.success() {
        return Err(format!(
            "cargo build --package weivel-test-plugins failed: {status}"
        ));
    }
    let file_name = format!("{DLL_PREFIX}weivel_test_plugins{DLL_SUFFIX}");
    let from = cargo_target_directory(root).join("debug").join(&file_name);
    copy(&from, &gathered.join(&file_name))
}

fn build_everything() -> Result<(), String> {
    let root = repository_root();
    let gathered = root.join("build").join("native");
    fs::create_dir_all(&gathered)
        .map_err(|error| format!("{} could not be made: {error}", gathered.display()))?;
    native_siblings()
        .iter()
        .try_for_each(|sibling| build_sibling(&root, sibling, &gathered))?;
    build_the_test_plugins(&root, &gathered)
}

fn main() -> ExitCode {
    match build_everything() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("native: {message}");
            ExitCode::FAILURE
        }
    }
}
