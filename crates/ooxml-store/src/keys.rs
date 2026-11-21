//! Owns: [`KeySet`], the typed read and write sets of a transaction and a history step.
//!
//! Must not: know what a key means; the model's key type does.

use std::collections::BTreeSet;

/// A set of the model's keys, or every key at once (a transaction that read the whole
/// document, a change whose extent the model cannot name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySet<K: Ord> {
    /// These keys.
    Keys(BTreeSet<K>),
    /// Every key.
    Everything,
}

impl<K: Ord> Default for KeySet<K> {
    fn default() -> Self {
        Self::Keys(BTreeSet::new())
    }
}

impl<K: Ord + Clone> KeySet<K> {
    /// The empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// A set of these keys.
    pub fn of(keys: impl IntoIterator<Item = K>) -> Self {
        Self::Keys(keys.into_iter().collect())
    }

    /// Add `key`.
    pub fn insert(&mut self, key: K) {
        if let Self::Keys(keys) = self {
            keys.insert(key);
        }
    }

    /// Make this the set of every key.
    pub fn insert_everything(&mut self) {
        *self = Self::Everything;
    }

    /// Add every key of `other`.
    pub fn extend(&mut self, other: &Self) {
        match (&mut *self, other) {
            (Self::Everything, _) => {}
            (_, Self::Everything) => *self = Self::Everything,
            (Self::Keys(mine), Self::Keys(theirs)) => mine.extend(theirs.iter().cloned()),
        }
    }

    /// No key.
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Keys(keys) if keys.is_empty())
    }

    /// Whether `key` is in the set.
    pub fn contains(&self, key: &K) -> bool {
        match self {
            Self::Keys(keys) => keys.contains(key),
            Self::Everything => true,
        }
    }

    /// The keys in both sets.
    pub fn intersection(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Everything, x) | (x, Self::Everything) => x.clone(),
            (Self::Keys(a), Self::Keys(b)) => Self::Keys(a.intersection(b).cloned().collect()),
        }
    }

    /// The keys of this set not in `other` (every key less some keys stays every key).
    pub fn without(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Everything, _) => Self::Everything,
            (Self::Keys(_), Self::Everything) => Self::new(),
            (Self::Keys(a), Self::Keys(b)) => Self::Keys(a.difference(b).cloned().collect()),
        }
    }

    /// The keys of this set but those `drop` names (every key stays every key).
    pub fn without_where(&self, drop: impl Fn(&K) -> bool) -> Self {
        match self {
            Self::Everything => Self::Everything,
            Self::Keys(keys) => Self::Keys(keys.iter().filter(|key| !drop(key)).cloned().collect()),
        }
    }

    /// Whether the sets share a key.
    pub fn intersects(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Everything, x) | (x, Self::Everything) => !x.is_empty(),
            (Self::Keys(a), Self::Keys(b)) => {
                let (small, large) = if a.len() <= b.len() { (a, b) } else { (b, a) };
                small.iter().any(|key| large.contains(key))
            }
        }
    }

    /// The keys, or `None` for every key.
    pub fn keys(&self) -> Option<&BTreeSet<K>> {
        match self {
            Self::Keys(keys) => Some(keys),
            Self::Everything => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_absorbs_and_intersects() {
        let mut a = KeySet::of([1, 2]);
        let b = KeySet::of([2, 3]);
        assert!(a.intersects(&b));
        assert_eq!(a.intersection(&b), KeySet::of([2]));
        assert!(!KeySet::<u8>::Everything.intersects(&KeySet::new()));
        assert_eq!(KeySet::Everything.intersection(&b), b);
        a.extend(&KeySet::Everything);
        assert_eq!(a, KeySet::Everything);
        a.insert(9);
        assert!(a.contains(&9) && a.keys().is_none());
    }
}
