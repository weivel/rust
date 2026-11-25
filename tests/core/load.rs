//! Loading the core's binary: the real core, a library that lacks a function, and paths that name no library.

use std::fs;

use weivel_rust::core::declarations::core_function_names;
use weivel_rust::core::{CoreBinary, LoadFailure};

use crate::support::{repository_root, shared_library};

#[test]
fn the_pinned_core_loads_with_every_function() {
    assert!(CoreBinary::load(shared_library("weivel_core")).is_ok());
}

#[test]
fn a_library_that_lacks_a_function_is_answered_naming_the_first_missing() {
    let failure = CoreBinary::load(shared_library("echo"))
        .expect_err("echo exports none of the core's functions");
    assert_eq!(
        failure,
        LoadFailure::MissingFunction(core_function_names()[0])
    );
}

#[test]
fn a_file_that_is_no_library_is_answered_as_one() {
    let directory = repository_root().join("build").join("load-test");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("not-a-library.txt");
    fs::write(&path, "this is no library").unwrap();
    assert_eq!(
        CoreBinary::load(&path).unwrap_err(),
        LoadFailure::NotALibrary
    );
}

#[test]
fn a_missing_path_and_an_empty_one_are_no_library() {
    assert_eq!(
        CoreBinary::load(repository_root().join("build").join("no-such-library")).unwrap_err(),
        LoadFailure::NotALibrary
    );
    assert_eq!(CoreBinary::load("").unwrap_err(), LoadFailure::NotALibrary);
}
