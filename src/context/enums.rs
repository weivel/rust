//! The enums a description carries, by the names its header spells, as a plugin reads them through its context.

use std::collections::BTreeMap;

use crate::core::Answer;

/// Each enum a description lists, by the name its header spells, holding its entries' numbers by the names the header
/// spells them, with `weivel_status` beside them as `core.h` spells it, since every operation answers one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DescribedEnums {
    enums: BTreeMap<String, BTreeMap<String, u32>>,
}

impl DescribedEnums {
    pub(crate) fn new(enums: BTreeMap<String, BTreeMap<String, u32>>) -> DescribedEnums {
        DescribedEnums { enums }
    }

    /// The names of the enums, in name order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.enums.keys().map(String::as_str)
    }

    /// The entries of the enum `name` by name, or none for an enum the description lists none of.
    pub fn entries(&self, name: &str) -> Option<&BTreeMap<String, u32>> {
        self.enums.get(name)
    }

    /// The number of the entry `entry` of the enum `name`, or none when either is missing.
    pub fn number(&self, name: &str, entry: &str) -> Option<u32> {
        self.entries(name)?.get(entry).copied()
    }

    /// Exactly the enums and entries `names` asks for, each enum by name with the entries it reads; the caller's
    /// mistake, naming `of`, for the first enum or entry these lack, so a name the description does not carry stops
    /// its reader rather than reading as missing in every comparison.
    pub fn require(&self, names: &[(&str, &[&str])], of: &str) -> Result<DescribedEnums, Answer> {
        let mut required = BTreeMap::new();
        for (name, entries) in names {
            let described = self.entries(name).ok_or_else(|| {
                Answer::Mistake(format!("the description of {of} lists no enum {name}"))
            })?;
            let mut read = BTreeMap::new();
            for entry in *entries {
                let number = described.get(*entry).ok_or_else(|| {
                    Answer::Mistake(format!("the enum {name} of {of} has no entry {entry}"))
                })?;
                read.insert((*entry).to_owned(), *number);
            }
            required.insert((*name).to_owned(), read);
        }
        Ok(DescribedEnums { enums: required })
    }
}
