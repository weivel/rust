//! Definitions bound from the description their C library answers, so a Rust plugin calls a library that ships no
//! Rust source: every struct laid out by this platform's rules and held against the description at the load, one plan
//! per operation built then, and each call made from those plans through the one native call it needs.

#[forbid(unsafe_code)]
pub(crate) mod abi;
mod definition;
#[forbid(unsafe_code)]
mod described;
#[forbid(unsafe_code)]
mod enums;
#[forbid(unsafe_code)]
mod holdings;
mod invoke;
#[forbid(unsafe_code)]
mod layouts;
#[forbid(unsafe_code)]
mod members;
mod memory;
mod operations;
mod read;
mod reader;
#[forbid(unsafe_code)]
mod stack;
#[forbid(unsafe_code)]
mod value;
mod write;

pub use definition::BoundDefinition;
pub(crate) use definition::{bind_described, contract_of};
pub use holdings::DescribedHandle;
pub(crate) use holdings::{CarriedScope, Holdings, Mapping, RemoteMapping, ScopeCall};
pub(crate) use layouts::Layouts;
pub use value::{
    Answers, CallStatus, DescribedAnswer, DescribedCallError, Inputs, Members, MembersIter, Tree,
    Value,
};
