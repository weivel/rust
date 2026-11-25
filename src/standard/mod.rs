//! The standard's defaults, reached through the standard's own binary, which its caller names by path.
//!
//! [`StandardJson`] binds the standard's JSON reader, `json.h`, and reads a document into a [`Json`] value of the
//! crate's own, so the crate reads JSON without a JSON parser of its own. [`StandardDescriptions`] binds the
//! standard's description checks, `description.h`, as the host's description check, and [`declarations`] lists the
//! layout of every struct of the standard's headers the crate declares by hand, for a test to hold to the compiler.
//! Nothing here is used by the core's wrapper or the contexts: a JSON document reaches them only as the values the
//! standard read, and the host runs a description check only when its caller hands it one.

#[doc(hidden)]
pub mod declarations;
mod description;
mod json;

pub use description::StandardDescriptions;
pub use json::{Json, JsonRefusal, StandardJson};
