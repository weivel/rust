//! The standard's defaults, reached through the standard's own binary, which its caller names by path.
//!
//! [`StandardJson`] binds the standard's JSON reader, `json.h`, and reads a document into a [`Json`] value of the
//! crate's own, so the crate reads JSON without a JSON parser of its own. Nothing here is used by the core's wrapper,
//! the contexts or the host: a JSON document reaches them only as the values the standard read.

mod json;

pub use json::{Json, JsonRefusal, StandardJson};
