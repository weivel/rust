//! The driver for the layer suite, as the core's `docs/contract/conformance.md` asks of a language's `conformance`
//! module.
//!
//! [`LayerDriver`] reads the pinned kit's layer vectors through the standard's JSON reader and writes no file; it
//! builds each vector's plugins as manifest values and entries, drives them through the host's public surface on the
//! real core, a `want` vector on a clock and scheduler of the suite's own, a `library` vector through the core's
//! `echo` test library, whose definition `echo` reads as its header lays it out, a `description` vector through the
//! standard's `described` test library on the standard's description check, and answers a [`LayerWitness`] in the
//! shape of the vector's `then`, so the witness equals the `then` exactly when the run witnessed the vector.

#[forbid(unsafe_code)]
mod consume;
#[forbid(unsafe_code)]
mod description;
#[forbid(unsafe_code)]
mod driver;
mod echo;
#[forbid(unsafe_code)]
mod host;
#[forbid(unsafe_code)]
mod library;
#[forbid(unsafe_code)]
mod lifecycle;
#[forbid(unsafe_code)]
mod observations;
#[forbid(unsafe_code)]
mod shapes;
#[forbid(unsafe_code)]
mod want;
#[forbid(unsafe_code)]
mod world;

pub use driver::{LayerDriver, LayerVector, LayerWitness};
pub use host::{
    HOST_VECTOR_BOUND, HOST_VECTOR_ENTRY_SYMBOL, HostProgramFixtures, PLUGIN_WORDS_DIRECTORY,
    RunPointer, point_runs_at, run_pointer_file,
};
pub use shapes::DriverFailure;
