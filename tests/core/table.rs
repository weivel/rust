//! The table holds the pinned core's functions exactly: every one its header declares, in the header's order, and no
//! other.

use std::fs;

use weivel_rust::core::declarations::core_function_names;

use crate::support::repository_root;

/// The name of each function `core.h` declares, in order: what follows `WEIVEL_API weivel_status` up to its `(`.
fn declared_by_the_header() -> Vec<String> {
    let header = repository_root()
        .join("siblings")
        .join("core")
        .join("include")
        .join("weivel")
        .join("core.h");
    let text = fs::read_to_string(&header)
        .unwrap_or_else(|error| panic!("{} cannot be read: {error}", header.display()));
    text.split("WEIVEL_API weivel_status")
        .skip(1)
        .map(|declaration| {
            declaration
                .split('(')
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned()
        })
        .collect()
}

#[test]
fn the_table_names_every_function_the_core_header_declares_in_order() {
    let declared = declared_by_the_header();
    assert!(
        !declared.is_empty(),
        "the core's header declares no function the test can read"
    );
    let held: Vec<String> = core_function_names()
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    assert_eq!(held, declared);
}
