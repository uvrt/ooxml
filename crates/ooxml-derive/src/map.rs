//! Owns: [`PersistentMap`], a hash map whose versions share what they have in common: the
//! entries are spread over a fixed number of shards, each shared between the versions
//! that did not change it, so a clone costs O(shards) pointer copies and a change copies
//! one shard. Whole-document facts -- counts of ids in use, a block index by id -- are
//! kept in one and brought up to date from a change's blocks (design section 7), instead
//! of being rebuilt per version.
//!
//! Must not: know what its keys and values are; mutate a shard another version can see
//! (every change copies the shard first).

use std::borrow::Borrow;
use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash};
use std::sync::Arc;

/// How many shards a map has: a change copies about `len / SHARDS` entries.
const SHARDS: usize = 256;

type Shard<K, V> = Arc<HashMap<K, V, RandomState>>;

/// A persistent hash map (structural sharing by shard).
pub struct PersistentMap<K, V> {
    shards: Arc<Vec<Option<Shard<K, V>>>>,
    hasher: RandomState,
    len: usize,
}

impl<K, V> Clone for PersistentMap<K, V> {
    fn clone(&self) -> Self {
        Self {
            shards: Arc::clone(&self.shards),
            hasher: self.hasher.clone(),
            len: self.len,
        }
    }
}

impl<K, V> std::fmt::Debug for PersistentMap<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersistentMap")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

impl<K: Hash + Eq + Clone, V: Clone> Default for PersistentMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Hash + Eq + Clone, V: Clone> PersistentMap<K, V> {
    /// An empty map; its versions all hash with the keys chosen here.
    pub fn new() -> Self {
        Self {
            shards: Arc::new(vec![None; SHARDS]),
            hasher: RandomState::new(),
            len: 0,
        }
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.len
    }

    /// No entries.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn shard_of<Q: Hash + ?Sized>(&self, key: &Q) -> usize {
        // the top bits: the shard's own table uses the low ones
        #[allow(clippy::cast_possible_truncation)]
        let hash = (self.hasher.hash_one(key) >> 56) as usize;
        hash % SHARDS
    }

    /// The value of `key`.
    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.shards.get(self.shard_of(key))?.as_ref()?.get(key)
    }

    /// Whether `key` has a value.
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.get(key).is_some()
    }

    /// The shard of `key`, unshared (copied when another version holds it).
    fn shard_mut(&mut self, at: usize) -> Option<&mut HashMap<K, V, RandomState>> {
        let hasher = self.hasher.clone();
        let slot = Arc::make_mut(&mut self.shards).get_mut(at)?;
        let shard = slot.get_or_insert_with(|| Arc::new(HashMap::with_hasher(hasher)));
        Some(Arc::make_mut(shard))
    }

    /// Set `key` to `value`; the value it had.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        let at = self.shard_of(&key);
        let old = self.shard_mut(at)?.insert(key, value);
        if old.is_none() {
            self.len += 1;
        }
        old
    }

    /// Remove `key`; the value it had.
    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let at = self.shard_of(key);
        // nothing to copy when the key is not there
        self.shards.get(at)?.as_ref()?.get(key)?;
        let old = self.shard_mut(at)?.remove(key);
        if old.is_some() {
            self.len -= 1;
        }
        old
    }

    /// Change `key`'s value by `change` (given the value it has, if any; returning the
    /// value it is to have, `None` to remove it).
    pub fn update(&mut self, key: K, change: impl FnOnce(Option<&V>) -> Option<V>) {
        match change(self.get(&key)) {
            Some(value) => {
                self.insert(key, value);
            }
            None => {
                self.remove(&key);
            }
        }
    }

    /// Every entry, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.shards.iter().flatten().flat_map(|shard| shard.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::PersistentMap;

    #[test]
    fn versions_share_and_differ() {
        let mut a: PersistentMap<String, u32> = PersistentMap::new();
        for k in 0..2000u32 {
            a.insert(format!("k{k}"), k);
        }
        let mut b = a.clone();
        b.insert("k5".to_owned(), 50);
        b.remove("k6");
        b.update("k7".to_owned(), |v| v.map(|v| v + 1));
        b.update("new".to_owned(), |_| Some(1));
        assert_eq!(a.get("k5"), Some(&5));
        assert_eq!(a.get("k6"), Some(&6));
        assert_eq!(a.get("k7"), Some(&7));
        assert_eq!(a.len(), 2000);
        assert_eq!(b.get("k5"), Some(&50));
        assert_eq!(b.get("k6"), None);
        assert_eq!(b.get("k7"), Some(&8));
        assert_eq!(b.len(), 2000);
        assert_eq!(b.iter().count(), 2000);
        assert!(b.remove("absent").is_none());
    }
}
