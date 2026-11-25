//! One struct's members by name, kept in one allocation in the order they were put, each name shared rather than
//! copied: an answer's names are the description's own, taken from it at the bind.

use std::fmt;
use std::sync::Arc;

use super::value::Value;

/// One struct's members by name, as an input writes them and an answer carries them, in the order they were put: an
/// answer's in the description's order. Two are equal when they hold the same members, whatever their order.
#[derive(Clone, Default)]
pub struct Members {
    entries: Vec<(Arc<str>, Value)>,
}

impl Members {
    /// No members.
    pub const fn new() -> Members {
        Members {
            entries: Vec::new(),
        }
    }

    /// No members, with room for `capacity` without allocating again.
    pub fn with_capacity(capacity: usize) -> Members {
        Members {
            entries: Vec::with_capacity(capacity),
        }
    }

    /// The member called `name`.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(held, _)| **held == *name)
            .map(|(_, value)| value)
    }

    /// The member called `name`, to change.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Value> {
        self.entries
            .iter_mut()
            .find(|(held, _)| **held == *name)
            .map(|(_, value)| value)
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
                self.entries.push((name, value));
                None
            }
        }
    }

    /// Takes the member `name` out.
    pub fn remove(&mut self, name: &str) -> Option<Value> {
        let index = self.entries.iter().position(|(held, _)| **held == *name)?;
        Some(self.entries.remove(index).1)
    }

    /// The names, in order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(name, _)| &**name)
    }

    /// The values, in order.
    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.entries.iter().map(|(_, value)| value)
    }

    /// Each name and its value, in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries.iter().map(|(name, value)| (&**name, value))
    }

    /// How many members.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Puts a member the caller knows is not held yet, sharing its name.
    pub(crate) fn push(&mut self, name: Arc<str>, value: Value) {
        self.entries.push((name, value));
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
        self.entries.into_iter()
    }
}

impl<'a> IntoIterator for &'a Members {
    type Item = (&'a str, &'a Value);
    type IntoIter = std::iter::Map<
        std::slice::Iter<'a, (Arc<str>, Value)>,
        fn(&'a (Arc<str>, Value)) -> (&'a str, &'a Value),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter().map(|(name, value)| (&**name, value))
    }
}
