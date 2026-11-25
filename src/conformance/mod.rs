//! The driver for the layer suite, as the core's `docs/contract/conformance.md` asks of a language's `conformance`
//! module.
//!
//! [`LayerDriver`] reads the pinned kit's layer vectors through the standard's JSON reader and writes no file; it
//! builds each vector's plugins as manifest values and entries, drives them through the host's public surface on the
//! real core, a `want` vector on a clock and scheduler of the suite's own, and answers a [`LayerWitness`] in the shape
//! of the vector's `then`, so the witness equals the `then` exactly when the run witnessed the vector.

mod consume;
mod driver;
mod lifecycle;
mod observations;
mod shapes;
mod want;
mod world;

pub use driver::{LayerDriver, LayerVector, LayerWitness};
pub use shapes::DriverFailure;
