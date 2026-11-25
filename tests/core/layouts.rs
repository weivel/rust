//! The crate's hand-written structs held to what the C compiler makes of the headers, as the `layouts` programs of the
//! core and of the standard print it.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use weivel_rust::core::declarations::{FieldLayout, StructLayout, core_struct_layouts};

use crate::support::program;

/// One struct as a `layouts` program printed it: its fields in declaration order, and its size.
#[derive(Debug, Default, PartialEq, Eq)]
struct PrintedStruct {
    fields: Vec<(String, usize, usize)>,
    size: Option<usize>,
}

fn number(word: &str, line: &str) -> usize {
    word.parse()
        .unwrap_or_else(|_| panic!("a layouts line the test cannot read: {line}"))
}

fn read_line(line: &str, printed: &mut BTreeMap<String, PrintedStruct>) {
    let words: Vec<&str> = line.split(' ').collect();
    match words.as_slice() {
        [name, "sizeof", size] => {
            printed.entry((*name).to_owned()).or_default().size = Some(number(size, line))
        }
        [name, field, offset, size] => {
            printed.entry((*name).to_owned()).or_default().fields.push((
                (*field).to_owned(),
                number(offset, line),
                number(size, line),
            ))
        }
        _ => panic!("a layouts line the test cannot read: {line}"),
    }
}

/// Every struct `layouts` prints, by name.
fn printed_by(layouts: &Path) -> BTreeMap<String, PrintedStruct> {
    let output = Command::new(layouts)
        .output()
        .unwrap_or_else(|error| panic!("{} did not run: {error}", layouts.display()));
    assert!(
        output.status.success(),
        "{} did not print its layouts: {}",
        layouts.display(),
        output.status
    );
    let text = String::from_utf8(output.stdout).expect("a layouts program prints UTF-8");
    let mut printed = BTreeMap::new();
    for line in text
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.trim().is_empty())
    {
        read_line(line, &mut printed);
    }
    printed
}

fn fields_of(layout: &StructLayout) -> Vec<(String, usize, usize)> {
    layout
        .fields
        .iter()
        .map(|FieldLayout { name, offset, size }| ((*name).to_owned(), *offset, *size))
        .collect()
}

/// Every difference between the declared layouts and the printed ones, one line each.
fn differences(
    declared: &[StructLayout],
    printed: &BTreeMap<String, PrintedStruct>,
) -> Vec<String> {
    let mut found = Vec::new();
    for layout in declared {
        let Some(struct_printed) = printed.get(layout.name) else {
            found.push(format!(
                "{} is not a struct the layouts program prints",
                layout.name
            ));
            continue;
        };
        if fields_of(layout) != struct_printed.fields {
            found.push(format!(
                "{} fields: declared {:?}, printed {:?}",
                layout.name,
                fields_of(layout),
                struct_printed.fields
            ));
        }
        if Some(layout.size) != struct_printed.size {
            found.push(format!(
                "{} sizeof: declared {}, printed {:?}",
                layout.name, layout.size, struct_printed.size
            ));
        }
    }
    found
}

#[test]
fn every_core_struct_the_crate_declares_matches_the_compilers() {
    let printed = printed_by(&program("core-layouts"));
    let found = differences(&core_struct_layouts(), &printed);
    assert!(
        found.is_empty(),
        "the hand layouts differ from the core's:\n{}",
        found.join("\n")
    );
}

#[test]
fn every_struct_the_core_prints_is_declared_by_the_crate() {
    let printed = printed_by(&program("core-layouts"));
    let declared: Vec<&str> = core_struct_layouts()
        .iter()
        .map(|layout| layout.name)
        .collect();
    let undeclared: Vec<&String> = printed
        .keys()
        .filter(|name| !declared.contains(&name.as_str()))
        .collect();
    assert!(
        undeclared.is_empty(),
        "the core prints structs the crate does not declare: {undeclared:?}"
    );
}

#[test]
fn every_struct_the_standard_prints_has_a_whole_layout() {
    let printed = printed_by(&program("standard-layouts"));
    assert!(
        !printed.is_empty(),
        "the standard's layouts program printed no struct"
    );
    let without_size: Vec<&String> = printed
        .iter()
        .filter(|(_, printed)| printed.size.is_none())
        .map(|(name, _)| name)
        .collect();
    assert!(
        without_size.is_empty(),
        "the standard printed structs with no sizeof line: {without_size:?}"
    );
}

#[test]
fn a_field_planted_at_the_wrong_offset_is_found() {
    let printed = printed_by(&program("core-layouts"));
    let mut planted = core_struct_layouts();
    planted[0].fields[1].offset += 1;
    let found = differences(&planted, &printed);
    assert_eq!(
        found.len(),
        1,
        "one planted offset is one difference: {found:?}"
    );
    assert!(
        found[0].starts_with("weivel_refusal fields"),
        "{}",
        found[0]
    );
}
