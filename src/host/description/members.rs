//! One struct's members by name, kept in one allocation in the order they were put. An answer's names are the
//! description's own: one table of them per struct, built at the bind and shared by every answer, each member naming
//! its place in the table, so reading an answer copies and counts no name.

use std::fmt;
use std::sync::Arc;

use super::value::Value;

/// One struct's members by name, as an input writes them and an answer carries them, in the order they were put: an
/// answer's in the description's order. Two are equal when they hold the same members, whatever their order.
#[derive(Clone, Default)]
pub struct Members {
    table: Option<Arc<[Arc<str>]>>,
    entries: Vec<(Name, Value)>,
}

/// A member's name: its place in the table of the description's names, or a name of its own.
#[derive(Clone)]
enum Name {
    Table(u32),
    Own(Arc<str>),
}

impl Members {
    /// No members.
    pub const fn new() -> Members {
        Members {
            table: None,
            entries: Vec::new(),
        }
    }

    /// No members, with room for `capacity` without allocating again.
    pub fn with_capacity(capacity: usize) -> Members {
        Members {
            table: None,
            entries: Vec::with_capacity(capacity),
        }
    }

    /// No members, with room for `capacity`, named from `table`.
    pub(crate) fn named_from(table: Arc<[Arc<str>]>, capacity: usize) -> Members {
        Members {
            table: Some(table),
            entries: Vec::with_capacity(capacity),
        }
    }

    fn spelled<'a>(&'a self, name: &'a Name) -> &'a str {
        match name {
            Name::Table(index) => self
                .table
                .as_ref()
                .map_or("", |table| &*table[*index as usize]),
            Name::Own(name) => name,
        }
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|(held, _)| self.spelled(held) == name)
    }

    /// The member called `name`.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.position(name).map(|index| &self.entries[index].1)
    }

    /// The member called `name`, to change.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Value> {
        let index = self.position(name)?;
        Some(&mut self.entries[index].1)
    }

    /// Whether a member is called `name`.
    pub fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Puts `value` as the member `name`, answering the value it replaced.
    pub fn insert(&mut self, name: impl Into<Arc<str>>, value: Value) -> Option<Value> {
        let name = name.into();
        match self.get_mut(&name) {
            Some(held) => Some(std::mem::replace(held, value)),
            None => {
                self.entries.push((Name::Own(name), value));
                None
            }
        }
    }

    /// Takes the member `name` out.
    pub fn remove(&mut self, name: &str) -> Option<Value> {
        let index = self.position(name)?;
        Some(self.entries.remove(index).1)
    }

    /// The names, in order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(name, _)| self.spelled(name))
    }

    /// The values, in order.
    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.entries.iter().map(|(_, value)| value)
    }

    /// Each name and its value, in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries
            .iter()
            .map(|(name, value)| (self.spelled(name), value))
    }

    /// How many members.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Puts the member named at `index` of the table, which the caller knows is not held yet.
    pub(crate) fn push_named(&mut self, index: u32, value: Value) {
        self.entries.push((Name::Table(index), value));
    }
}

impl PartialEq for Members {
    fn eq(&self, other: &Members) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .all(|(name, value)| other.get(name) == Some(value))
    }
}

impl fmt::Debug for Members {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_map().entries(self.iter()).finish()
    }
}

impl<Name: Into<Arc<str>>> FromIterator<(Name, Value)> for Members {
    fn from_iter<Pairs: IntoIterator<Item = (Name, Value)>>(pairs: Pairs) -> Members {
        let mut members = Members::new();
        for (name, value) in pairs {
            members.insert(name, value);
        }
        members
    }
}

impl<Name: Into<Arc<str>>, const COUNT: usize> From<[(Name, Value); COUNT]> for Members {
    fn from(pairs: [(Name, Value); COUNT]) -> Members {
        pairs.into_iter().collect()
    }
}

impl IntoIterator for Members {
    type Item = (Arc<str>, Value);
    type IntoIter = std::vec::IntoIter<(Arc<str>, Value)>;

    fn into_iter(self) -> Self::IntoIter {
        let table = self.table;
        self.entries
            .into_iter()
            .map(|(name, value)| match name {
                Name::Table(index) => (
                    table
                        .as_ref()
                        .map_or_else(|| Arc::from(""), |table| table[index as usize].clone()),
                    value,
                ),
                Name::Own(name) => (name, value),
            })
            .collect::<Vec<_>>()
            .into_iter()
    }
}

impl<'a> IntoIterator for &'a Members {
    type Item = (&'a str, &'a Value);
    type IntoIter = MembersIter<'a>;

    fn into_iter(self) -> MembersIter<'a> {
        MembersIter {
            members: self,
            next: 0,
        }
    }
}

/// Each name and value of [`Members`], in order.
pub struct MembersIter<'a> {
    members: &'a Members,
    next: usize,
}

impl<'a> Iterator for MembersIter<'a> {
    type Item = (&'a str, &'a Value);

    fn next(&mut self) -> Option<(&'a str, &'a Value)> {
        let (name, value) = self.members.entries.get(self.next)?;
        self.next += 1;
        Some((self.members.spelled(name), value))
    }
}
