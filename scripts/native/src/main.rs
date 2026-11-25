//! Builds the native siblings the tests load, with CMake in Release, and gathers what they built into `build/native`:
//! the core's shared library and its `echo` test library, and the core's and the standard's `layouts` programs.

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
    sibling: &'static str,
    targets: Vec<BuiltTarget>,
    needs_the_core_source: bool,
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
            sibling: "core",
            targets: vec![
                shared_library("weivel_core_shared", "weivel_core"),
                shared_library("echo", "echo"),
                program("layouts", "core-layouts"),
            ],
            needs_the_core_source: false,
        },
        NativeSibling {
            sibling: "standard",
            targets: vec![program("layouts", "standard-layouts")],
            needs_the_core_source: true,
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

fn require_sibling(root: &Path, sibling: &str) -> Result<PathBuf, String> {
    let directory = root.join("siblings").join(sibling);
    if directory.join("CMakeLists.txt").is_file() {
        return Ok(directory);
    }
    Err(format!(
        "siblings/{sibling} is missing. CI checks it out from repo.json; locally, link or clone the {sibling} checkout \
         there at the commit repo.json pins."
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
    if sibling.needs_the_core_source {
        arguments.push(format!(
            "-DWEIVEL_CORE_SOURCE_DIR={}",
            path_text(&root.join("siblings").join("core"))
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

fn build_sibling(root: &Path, sibling: &NativeSibling, gathered: &Path) -> Result<(), String> {
    let source = require_sibling(root, sibling.sibling)?;
    let build_directory = root.join("build").join(sibling.sibling);
    run_cmake(&configure_arguments(
        root,
        sibling,
        &source,
        &build_directory,
    ))?;
    run_cmake(&build_arguments(sibling, &build_directory))?;
    for built in &sibling.targets {
        let from = find_built(&build_directory, &built.file_name)?;
        let to = gathered.join(&built.gathered_name);
        fs::copy(&from, &to).map_err(|error| {
            format!(
                "{} could not be copied to {}: {error}",
                from.display(),
                to.display()
            )
        })?;
    }
    Ok(())
}

fn build_everything() -> Result<(), String> {
    let root = repository_root();
    let gathered = root.join("build").join("native");
    fs::create_dir_all(&gathered)
        .map_err(|error| format!("{} could not be made: {error}", gathered.display()))?;
    native_siblings()
        .iter()
        .try_for_each(|sibling| build_sibling(&root, sibling, &gathered))
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
