//! The Rust language implementation of weivel: it loads the C core its caller names and decides nothing itself.
//!
//! Each module is the one of the same name the core's contract describes. A module named `core` shadows the built-in
//! `core` crate here, so the crate writes `::core::` for that one, and `std` wherever either would do.

pub mod core;
