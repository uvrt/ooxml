//! Owns: [`Memo`], values computed from content and kept under the content's key (a block's
//! digest with the context it was computed in), evicted least recently used first when the
//! [`CacheBudget`] is spent.
//!
//! A hit is a hash lookup and a clock stamp: the order of use is kept lazily (an entry sits
//! in the age order under the use it was queued at; eviction and sweeps read its last use
//! and queue it again where it was used since). A memo keyed by addresses hashes them with
//! [`AddressHash`] rather than SipHash.
//!
//! Must not: invalidate by hand -- a changed block has a new key, and an entry no version
//! asks for again ages out -- or hold more than its budget gives it.

use crate::CacheBudget;
use std::collections::hash_map::RandomState;
use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasher, Hash, Hasher};
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
    /// Its last use.
    age: u64,
    /// The use it sits under in the age order: its last use, or one before.
    queued: u64,
}

/// Hashes a key of addresses and small integers (an element's address, a type's id): a
/// multiply-rotate per word, for a memo whose keys no document chooses.
#[derive(Debug, Clone, Copy, Default)]
pub struct AddressHash;

/// [`AddressHash`]'s hasher.
#[derive(Debug, Clone, Copy, Default)]
pub struct AddressHasher(u64);

impl AddressHasher {
    const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(Self::SEED);
    }
}

impl Hasher for AddressHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word.get_mut(..chunk.len()).unwrap_or_default().copy_from_slice(chunk);
            self.add(u64::from_le_bytes(word));
        }
    }

    fn write_u64(&mut self, word: u64) {
        self.add(word);
    }

    fn write_usize(&mut self, word: usize) {
        self.add(word as u64);
    }
}

impl BuildHasher for AddressHash {
    type Hasher = AddressHasher;

    fn build_hasher(&self) -> AddressHasher {
        AddressHasher::default()
    }
}

/// Values memoised by content key, within a budget.
///
/// A lookup clones the value, so a large value (a measured paragraph, a page's paint) is
/// held behind an `Arc`. A memo belongs to one owner (a document's derived engine, a layout
/// pass) and takes `&mut self`; a pass that computes its misses in parallel looks them up
/// with [`Memo::get`] and keeps them with [`Memo::insert`].
pub struct Memo<K, V, S = RandomState> {
    entries: HashMap<K, Entry<V>, S>,
    /// Entries by the use they are queued at (their last use, or one before), oldest first.
    ages: BTreeMap<u64, K>,
    clock: u64,
    /// The clock at the last [`Memo::sweep`]: an entry last used at or before it was not
    /// used since.
    swept: u64,
    budget: Arc<dyn CacheBudget>,
    stats: MemoStats,
}

impl<K: Hash + Eq + Clone, V: Clone + Weigh> Memo<K, V> {
    /// An empty memo reserving from `budget`.
    pub fn new(budget: Arc<dyn CacheBudget>) -> Self {
        Self::with_hasher(budget)
    }
}

impl<K: Hash + Eq + Clone, V: Clone + Weigh, S: BuildHasher + Default> Memo<K, V, S> {
    /// An empty memo reserving from `budget`, its keys hashed by `S`.
    pub fn with_hasher(budget: Arc<dyn CacheBudget>) -> Self {
        Self {
            entries: HashMap::with_hasher(S::default()),
            ages: BTreeMap::new(),
            clock: 0,
            swept: 0,
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
        self.touch(key).cloned()
    }

    /// The value held for `key`, borrowed, counted as a use as [`Memo::get`] counts it: for
    /// a caller that reads a part of a value dear to clone.
    pub fn touch(&mut self, key: &K) -> Option<&V> {
        self.clock += 1;
        let Some(entry) = self.entries.get_mut(key) else {
            self.stats.misses += 1;
            return None;
        };
        entry.age = self.clock;
        self.stats.hits += 1;
        Some(&entry.value)
    }

    /// Keep `value` under `key` (replacing what was held), evicting the oldest entries
    /// for room. Returns whether it was kept: a value the budget has no room for even
    /// after evicting everything is not.
    pub fn insert(&mut self, key: K, value: V) -> bool {
        self.clock += 1;
        if let Some(old) = self.entries.remove(&key) {
            self.ages.remove(&old.queued);
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
                queued: self.clock,
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

    /// Drop every entry not used since the last sweep (or since the memo was made) that
    /// `held` says nothing else holds: what no version asks for any more, so that a memo
    /// kept across versions holds what the versions in use need and not what they used to.
    /// Returns how many entries were dropped. An entry `held` keeps stays, used or not.
    pub fn sweep(&mut self, held: impl Fn(&V) -> bool) -> usize {
        let since = self.swept;
        self.swept = self.clock;
        let stale: Vec<u64> = self
            .ages
            .range(..=since)
            .filter(|(_, key)| {
                // queued before the sweep, used since: not stale
                self.entries
                    .get(*key)
                    .is_none_or(|entry| entry.age <= since && !held(&entry.value))
            })
            .map(|(age, _)| *age)
            .collect();
        for age in &stale {
            let Some(key) = self.ages.remove(age) else { continue };
            if let Some(entry) = self.entries.remove(&key) {
                self.budget.release(entry.bytes);
                self.stats.bytes -= entry.bytes;
            }
        }
        stale.len()
    }

    /// Evict the least recently used entry: the oldest queued whose last use is the one it
    /// is queued at (one used since is queued again at its last use). Whether there was one.
    fn evict_oldest(&mut self) -> bool {
        while let Some((queued, key)) = self.ages.pop_first() {
            let Some(entry) = self.entries.get_mut(&key) else {
                continue;
            };
            if entry.age != queued {
                entry.queued = entry.age;
                self.ages.insert(entry.age, key);
                continue;
            }
            if let Some(entry) = self.entries.remove(&key) {
                self.budget.release(entry.bytes);
                self.stats.bytes -= entry.bytes;
                self.stats.evictions += 1;
            }
            return true;
        }
        false
    }
}

impl<K, V, S> Drop for Memo<K, V, S> {
    fn drop(&mut self) {
        self.budget.release(self.stats.bytes);
    }
}
