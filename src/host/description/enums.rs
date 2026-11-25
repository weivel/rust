//! A description's enums by name, with `weivel_status` beside them where the description lists none.

use std::collections::BTreeMap;

use super::described::{Description, Enum};
use super::layouts::LayoutRefusal;
use crate::context::DescribedEnums;
use crate::core::{STATUS_MALFORMED, STATUS_NONE, STATUS_OK, STATUS_OUT_OF_MEMORY, STATUS_REFUSED};

const STATUS_NAME: &str = "weivel_status";

/// `weivel_status` as `core.h` spells it: every operation a description lists answers one, though a description need
/// not list it, since a role fixes it.
fn core_status() -> Enum {
    let entry = |name: &str, number: i32| (name.to_owned(), number as u32);
    Enum {
        name: STATUS_NAME.to_owned(),
        entries: vec![
            entry("WEIVEL_STATUS_OK", STATUS_OK),
            entry("WEIVEL_STATUS_NONE", STATUS_NONE),
            entry("WEIVEL_STATUS_MALFORMED", STATUS_MALFORMED),
            entry("WEIVEL_STATUS_REFUSED", STATUS_REFUSED),
            entry("WEIVEL_STATUS_OUT_OF_MEMORY", STATUS_OUT_OF_MEMORY),
        ],
    }
}

fn by_name(described: &Enum) -> BTreeMap<String, u32> {
    described.entries.iter().cloned().collect()
}

/// The enums of `description`, refusing one that lists a `weivel_status` other than `core.h`'s.
pub(crate) fn enums_of(description: &Description) -> Result<DescribedEnums, LayoutRefusal> {
    let core = core_status();
    let listed = description
        .enums
        .iter()
        .find(|described| described.name == STATUS_NAME);
    if listed.is_some_and(|listed| {
        listed.entries.len() != core.entries.len() || by_name(listed) != by_name(&core)
    }) {
        return Err(format!("it lists a {STATUS_NAME} that is not core.h's"));
    }
    let mut enums: BTreeMap<String, BTreeMap<String, u32>> = description
        .enums
        .iter()
        .map(|described| (described.name.clone(), by_name(described)))
        .collect();
    enums
        .entry(STATUS_NAME.to_owned())
        .or_insert_with(|| by_name(&core));
    Ok(DescribedEnums::new(enums))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(entries: Vec<(String, u32)>) -> Description {
        Description {
            definition: "example".to_owned(),
            types: Vec::new(),
            enums: vec![Enum {
                name: STATUS_NAME.to_owned(),
                entries,
            }],
            operations: Vec::new(),
            blocking: Vec::new(),
            definitions: Vec::new(),
        }
    }

    #[test]
    fn a_description_listing_core_hs_status_keeps_it() {
        let enums = enums_of(&listing(core_status().entries)).expect("core.h's status is kept");
        assert_eq!(
            enums.number(STATUS_NAME, "WEIVEL_STATUS_OUT_OF_MEMORY"),
            Some(4)
        );
    }

    #[test]
    fn a_description_listing_another_status_is_refused() {
        let mut entries = core_status().entries;
        entries[2].1 = 9;
        assert!(enums_of(&listing(entries)).is_err());
        let mut shorter = core_status().entries;
        shorter.pop();
        assert!(enums_of(&listing(shorter)).is_err());
    }

    #[test]
    fn a_description_listing_no_status_is_given_core_hs() {
        let mut description = listing(Vec::new());
        description.enums.clear();
        let enums = enums_of(&description).expect("no status is no fault");
        assert_eq!(enums.number(STATUS_NAME, "WEIVEL_STATUS_NONE"), Some(1));
    }
}
