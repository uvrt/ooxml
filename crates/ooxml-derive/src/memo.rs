//! Owns: [`Memo`], values computed from content and kept under the content's key (a block's
//! digest with the context it was computed in), evicted least recently used first when the
//! [`CacheBudget`] is spent.
//!
//! Must not: invalidate by hand -- a changed block has a new key, and an entry no version
//! asks for again ages out -- or hold more than its budget gives it.

use crate::CacheBudget;
use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;
use std::sync::Arc;

/// What a cached value costs in bytes, beyond the memo's own bookkeeping.
pub trait Weigh {
    /// The bytes it holds.
    fn weight(&self) -> usize;
}

/// The bytes the memo counts for an entry besides its value: the key twice (map and age
/// order), the age, the weight and the map's own slot.
fn overhead<K>() -> usize {
    2 * std::mem::size_of::<K>() + 4 * std::mem::size_of::<u64>()
}

/// A memo's counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemoStats {
    /// Lookups answered from the memo.
    pub hits: u64,
    /// Lookups that computed.
    pub misses: u64,
    /// Entries evicted for the budget.
    pub evictions: u64,
    /// Entries held.
    pub entries: usize,
    /// Bytes reserved from the budget.
    pub bytes: usize,
}

struct Entry<V> {
    value: V,
    bytes: usize,
    age: u64,
}

/// Values memoised by content key, within a budget.
///
/// A lookup clones the value, so a large value (a measured paragraph, a page's paint) is
/// held behind an `Arc`. A memo belongs to one owner (a document's derived engine, a layout
/// pass) and takes `&mut self`; a pass that computes its misses in parallel looks them up
/// with [`Memo::get`] and keeps them with [`Memo::insert`].
pub struct Memo<K, V> {
    entries: HashMap<K, Entry<V>>,
    /// Entries by last use, oldest first.
    ages: BTreeMap<u64, K>,
    clock: u64,
    budget: Arc<dyn CacheBudget>,
    stats: MemoStats,
}

impl<K: Hash + Eq + Clone, V: Clone + Weigh> Memo<K, V> {
    /// An empty memo reserving from `budget`.
    pub fn new(budget: Arc<dyn CacheBudget>) -> Self {
        Self {
            entries: HashMap::new(),
            ages: BTreeMap::new(),
            clock: 0,
            budget,
            stats: MemoStats::default(),
        }
    }

    /// The value for `key`, computed by `compute` when the memo does not hold it. A value
    /// the budget has no room for even after evicting everything is returned uncached.
    pub fn get_or_compute(&mut self, key: &K, compute: impl FnOnce() -> V) -> V {
        if let Some(value) = self.get(key) {
            return value;
        }
        let value = compute();
        self.insert(key.clone(), value.clone());
        value
    }

    /// The value held for `key`, counted as a use (a hit, or a miss when it is not held).
    /// With [`Memo::insert`] this lets a caller compute its misses elsewhere -- in
    /// parallel, say -- and keep them afterwards.
    pub fn get(&mut self, key: &K) -> Option<V> {
        self.clock += 1;
        let Some(entry) = self.entries.get_mut(key) else {
            self.stats.misses += 1;
            return None;
        };
        self.ages.remove(&entry.age);
        entry.age = self.clock;
        self.ages.insert(self.clock, key.clone());
        self.stats.hits += 1;
        Some(entry.value.clone())
    }

    /// Keep `value` under `key` (replacing what was held), evicting the oldest entries
    /// for room. Returns whether it was kept: a value the budget has no room for even
    /// after evicting everything is not.
    pub fn insert(&mut self, key: K, value: V) -> bool {
        self.clock += 1;
        if let Some(old) = self.entries.remove(&key) {
            self.ages.remove(&old.age);
            self.budget.release(old.bytes);
            self.stats.bytes -= old.bytes;
        }
        let bytes = value.weight() + overhead::<K>();
        while !self.budget.reserve(bytes) {
            if !self.evict_oldest() {
                return false;
            }
        }
        self.stats.bytes += bytes;
        self.ages.insert(self.clock, key.clone());
        self.entries.insert(
            key,
            Entry {
                value,
                bytes,
                age: self.clock,
            },
        );
        true
    }

    /// The value held for `key`, without computing or counting a use.
    pub fn peek(&self, key: &K) -> Option<&V> {
        self.entries.get(key).map(|entry| &entry.value)
    }

    /// Drop every entry and give its bytes back.
    pub fn clear(&mut self) {
        self.budget.release(self.stats.bytes);
        self.stats.bytes = 0;
        self.entries.clear();
        self.ages.clear();
    }

    /// The memo's counters.
    pub fn stats(&self) -> MemoStats {
        MemoStats {
            entries: self.entries.len(),
            ..self.stats
        }
    }

    fn evict_oldest(&mut self) -> bool {
        let Some((_, key)) = self.ages.pop_first() else {
            return false;
        };
        if let Some(entry) = self.entries.remove(&key) {
            self.budget.release(entry.bytes);
            self.stats.bytes -= entry.bytes;
            self.stats.evictions += 1;
        }
        true
    }
}

impl<K, V> Drop for Memo<K, V> {
    fn drop(&mut self) {
        self.budget.release(self.stats.bytes);
    }
}
