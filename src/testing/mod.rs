//! Helpers for the tests of a program that hosts plugins in Rust, as the core's `docs/contract/testing.md` asks of a
//! language's `testing` module. They name nothing of any one project, and no other module of the crate needs them.
//!
//! [`scratch_home`] lays out a fresh folder with libraries, files and variables as a [`ScratchLayout`] describes it;
//! [`set_variables`] sets variables of the process and puts them back; [`described_library`] binds a described C
//! library from its folder on a host of its own; [`free_port`] answers a port nothing listens on.
//!
//! Setting a variable of the process is `unsafe` since Rust 2024: another thread reading the environment meanwhile,
//! the C code of a loaded library among them, is undefined behaviour on POSIX systems. So [`set_variables`] and
//! [`scratch_home`], and putting their variables back, are `unsafe`, their caller promising that no other thread
//! reads or writes the environment while they run.

#[forbid(unsafe_code)]
mod described_library;
#[forbid(unsafe_code)]
mod failure;
#[forbid(unsafe_code)]
mod free_port;
mod scratch_home;
mod variables;

pub use described_library::{DescribedLibrary, DescribedLibraryOptions, described_library};
pub use failure::TestingFailure;
pub use free_port::free_port;
pub use scratch_home::{
    LIBRARY_DECLARATION, PlacedLibrary, ScratchFile, ScratchHome, ScratchLayout,
    library_binary_name, scratch_home,
};
pub use variables::{VariablesSet, set_variables};
