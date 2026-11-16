//! MonoidTree against a plain vector under random sets, inserts and removals (contents,
//! prefix and range aggregates with an order-sensitive monoid, balance); clones are
//! independent snapshots; Memo stays within its budget and evicts the oldest first.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_derive::{CacheBudget, DeriveError, Measured, Memo, Monoid, MonoidTree, Weigh};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// A polynomial hash of the sequence: equal only for the same items in the same order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Poly {
    hash: u64,
    power: u64,
}

impl Monoid for Poly {
    fn empty() -> Self {
        Poly { hash: 0, power: 1 }
    }
    fn combine(&self, other: &Self) -> Self {
        Poly {
            hash: self.hash.wrapping_mul(other.power).wrapping_add(other.hash),
            power: self.power.wrapping_mul(other.power),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Num(u64);

impl Measured for Num {
    type Summary = Poly;
    fn measure(&self) -> Poly {
        Poly {
            hash: self.0,
            power: 1_000_003,
        }
    }
}

fn fold(items: &[Num]) -> Poly {
    items.iter().fold(Poly::empty(), |acc, n| acc.combine(&n.measure()))
}

struct Rng(u64);
impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n.max(1)
    }
}

#[test]
fn random_edits_equal_a_vector() {
    for seed in 1..=12u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let start = rng.below(300) as usize;
        let mut oracle: Vec<Num> = (0..start as u64).map(Num).collect();
        let mut tree: MonoidTree<Num> = oracle.iter().copied().collect();
        let mut snapshots = Vec::new();
        for step in 0..3000u64 {
            let len = oracle.len();
            match rng.below(10) {
                0..=3 => {
                    let at = rng.below(len as u64 + 1) as usize;
                    tree.insert(at, Num(step)).unwrap();
                    oracle.insert(at, Num(step));
                }
                4..=6 if len > 0 => {
                    let at = rng.below(len as u64) as usize;
                    assert_eq!(tree.remove(at).unwrap(), oracle.remove(at));
                }
                7 | 8 if len > 0 => {
                    let at = rng.below(len as u64) as usize;
                    tree.set(at, Num(step + 7)).unwrap();
                    oracle[at] = Num(step + 7);
                }
                _ => snapshots.push((tree.clone(), oracle.clone())),
            }
            assert!(tree.is_well_formed(), "seed {seed} step {step}");
            assert_eq!(tree.len(), oracle.len());
            let at = rng.below(oracle.len() as u64 + 1) as usize;
            assert_eq!(tree.prefix(at).unwrap(), fold(&oracle[..at]));
            let to = at + rng.below((oracle.len() - at) as u64 + 1) as usize;
            assert_eq!(tree.range(at, to).unwrap(), fold(&oracle[at..to]));
            assert_eq!(tree.get(at).copied(), oracle.get(at).copied());
        }
        assert_eq!(tree.iter().copied().collect::<Vec<_>>(), oracle);
        assert_eq!(tree.summary(), fold(&oracle));
        for (snapshot, items) in snapshots {
            assert_eq!(
                snapshot.iter().copied().collect::<Vec<_>>(),
                items,
                "a clone is not changed by later edits"
            );
        }
    }
}

#[test]
fn positions_out_of_range_are_refused() {
    let mut tree: MonoidTree<Num> = (0..5).map(Num).collect();
    assert_eq!(tree.set(5, Num(0)), Err(DeriveError::OutOfRange { at: 5, len: 5 }));
    assert!(tree.remove(5).is_err());
    assert!(tree.insert(6, Num(0)).is_err());
    assert!(tree.prefix(6).is_err());
    assert!(tree.range(3, 2).is_err());
    assert!(tree.iter_from(6).is_err());
    assert_eq!(tree.iter_from(5).unwrap().count(), 0);
    let mut empty = MonoidTree::<Num>::new();
    assert!(empty.remove(0).is_err());
    assert_eq!(empty.prefix(0).unwrap(), Poly::empty());
}

struct Budget {
    used: AtomicUsize,
    limit: usize,
}
impl CacheBudget for Budget {
    fn reserve(&self, bytes: usize) -> bool {
        let mut used = self.used.load(Ordering::SeqCst);
        loop {
            if used + bytes > self.limit {
                return false;
            }
            match self
                .used
                .compare_exchange(used, used + bytes, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return true,
                Err(now) => used = now,
            }
        }
    }
    fn release(&self, bytes: usize) {
        self.used.fetch_sub(bytes, Ordering::SeqCst);
    }
    fn used(&self) -> usize {
        self.used.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Blob(usize);
impl Weigh for Blob {
    fn weight(&self) -> usize {
        self.0
    }
}

#[test]
fn memo_keeps_within_its_budget_and_evicts_the_oldest() {
    let budget = Arc::new(Budget {
        used: AtomicUsize::new(0),
        limit: 1000,
    });
    let mut memo: Memo<u32, Blob> = Memo::new(budget.clone());
    let mut computed = 0;
    for key in 0..10u32 {
        memo.get_or_compute(&key, || {
            computed += 1;
            Blob(100)
        });
    }
    let stats = memo.stats();
    assert!(budget.used() <= 1000 && budget.used() == stats.bytes);
    assert!(stats.evictions > 0 && stats.entries < 10);
    assert!(memo.peek(&0).is_none(), "the oldest went first");
    assert!(memo.peek(&9).is_some());
    // A hit computes nothing and makes the key the newest.
    memo.get_or_compute(&9, || unreachable!());
    assert_eq!(memo.stats().hits, 1);
    // Too big for the whole budget: computed and returned, not kept.
    assert_eq!(memo.get_or_compute(&99, || Blob(5000)), Blob(5000));
    assert!(memo.peek(&99).is_none());
    drop(memo);
    assert_eq!(budget.used(), 0, "a dropped memo gives its bytes back");
}

#[test]
fn memo_misses_computed_elsewhere_are_kept_with_insert() {
    let budget = Arc::new(Budget {
        used: AtomicUsize::new(0),
        limit: 1000,
    });
    let mut memo: Memo<u32, Blob> = Memo::new(budget.clone());
    let keys = [1u32, 2, 3];
    let misses: Vec<u32> = keys.iter().copied().filter(|key| memo.get(key).is_none()).collect();
    assert_eq!(misses, keys);
    // Computed outside the memo (a parallel pass would do this), then kept.
    let computed: Vec<(u32, Blob)> = misses.into_iter().map(|key| (key, Blob(10 * key as usize))).collect();
    for (key, value) in computed {
        assert!(memo.insert(key, value));
    }
    assert_eq!(memo.get(&2), Some(Blob(20)));
    assert_eq!(
        memo.stats(),
        ooxml_derive::MemoStats {
            hits: 1,
            misses: 3,
            evictions: 0,
            entries: 3,
            bytes: budget.used()
        }
    );
    // Replacing a value gives the old one's bytes back.
    let before = budget.used();
    assert!(memo.insert(2, Blob(0)));
    assert_eq!(budget.used(), before - 20);
    assert!(!memo.insert(4, Blob(5000)), "larger than the whole budget");
    assert!(memo.peek(&4).is_none());
}
