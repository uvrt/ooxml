//! The reference fact: list labels (design section 7, spike 3), kept with ooxml-derive's
//! primitives. A made-up block has text, a list (0 for none) and a level; an item's label
//! is its list's counters after it (a level counts up, deeper levels restart). Each block's
//! (list, level) is memoised by the block's content; the order-dependent part is one
//! MonoidTree over all blocks whose summary is, per list, the composed counter transform.
//! Shared by tests/list_labels.rs and examples/list_labels.rs; written without indexing or
//! unwrapping so that it builds under the library lints in the example too.
#![allow(dead_code)]

use ooxml_derive::{BlockFact, CacheBudget, DeriveError, Measured, Monoid, MonoidTree, Weigh};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};

/// List levels.
pub const LEVELS: usize = 9;
/// A list's counter per level.
pub type Counters = [u32; LEVELS];

/// A made-up block.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Block {
    /// Its text.
    pub text: String,
    /// Its list; 0 for none.
    pub list: u32,
    /// Its level in the list, 0 to 8.
    pub level: u8,
}

/// A transform of one list's counters: levels below `m` kept, level `m` raised by `k`,
/// levels after `m` set to `tail`. `m == LEVELS`: the identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    m: u8,
    k: u32,
    tail: Counters,
}

const IDENTITY: Step = Step {
    m: LEVELS as u8,
    k: 0,
    tail: [0; LEVELS],
};

fn bump(counters: &mut Counters, level: usize) {
    if let Some(at) = counters.get_mut(level) {
        *at += 1;
    }
    for deeper in counters.iter_mut().skip(level + 1) {
        *deeper = 0;
    }
}

/// `a` then `b`.
fn then(a: &Step, b: &Step) -> Step {
    if b.m as usize == LEVELS {
        return *a;
    }
    if a.m as usize == LEVELS || b.m < a.m {
        return *b;
    }
    if b.m == a.m {
        return Step {
            m: a.m,
            k: a.k + b.k,
            tail: b.tail,
        };
    }
    let mut tail = a.tail;
    if let Some(at) = tail.get_mut(b.m as usize) {
        *at += b.k;
    }
    for (mine, theirs) in tail.iter_mut().zip(b.tail.iter()).skip(b.m as usize + 1) {
        *mine = *theirs;
    }
    Step { m: a.m, k: a.k, tail }
}

fn apply(step: &Step, counters: &Counters) -> Counters {
    let mut out = *counters;
    if let Some(at) = out.get_mut(step.m as usize) {
        *at += step.k;
        for (mine, theirs) in out.iter_mut().zip(step.tail.iter()).skip(step.m as usize + 1) {
            *mine = *theirs;
        }
    }
    out
}

/// Per list, the composed transform of a run of blocks; sorted by list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lists(Vec<(u32, Step)>);

impl Lists {
    fn get(&self, list: u32) -> Option<&Step> {
        self.0
            .binary_search_by_key(&list, |(l, _)| *l)
            .ok()
            .and_then(|at| self.0.get(at))
            .map(|(_, step)| step)
    }

    /// Only the lists in `wanted` (sorted): a homomorphism, so a prefix query through it
    /// costs the lists a read needs, not every list of the document.
    fn only(&self, wanted: &[u32]) -> Lists {
        Lists(
            wanted
                .iter()
                .filter_map(|&list| self.get(list).map(|step| (list, *step)))
                .collect(),
        )
    }
}

impl Monoid for Lists {
    fn empty() -> Self {
        Lists(Vec::new())
    }

    fn combine(&self, other: &Self) -> Self {
        if other.0.is_empty() {
            return self.clone();
        }
        if self.0.is_empty() {
            return other.clone();
        }
        let mut out = Vec::with_capacity(self.0.len() + other.0.len());
        let (mut a, mut b) = (self.0.iter().peekable(), other.0.iter().peekable());
        loop {
            match (a.peek(), b.peek()) {
                (Some((la, ta)), Some((lb, tb))) if la == lb => {
                    out.push((*la, then(ta, tb)));
                    a.next();
                    b.next();
                }
                (Some(x), Some((lb, _))) if x.0 < *lb => out.extend(a.next().copied()),
                (_, Some(_)) => out.extend(b.next().copied()),
                (Some(_), None) => out.extend(a.next().copied()),
                (None, None) => return Lists(out),
            }
        }
    }
}

/// A block's value for the fact: its list and level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    /// The list; 0 for none.
    pub list: u32,
    /// The level.
    pub level: u8,
}

impl Measured for Item {
    type Summary = Lists;
    fn measure(&self) -> Lists {
        if self.list == 0 {
            return Lists::empty();
        }
        Lists(vec![(
            self.list,
            Step {
                m: self.level,
                k: 1,
                tail: [0; LEVELS],
            },
        )])
    }
}

impl Weigh for Item {
    fn weight(&self) -> usize {
        std::mem::size_of::<Item>()
    }
}

/// The fact: a block's (list, level), keyed by a digest of its content.
pub struct ListItems;

impl BlockFact for ListItems {
    type Block = Block;
    type Key = u64;
    type Value = Item;

    fn key(&self, block: &Block) -> u64 {
        let mut hasher = DefaultHasher::new();
        block.hash(&mut hasher);
        hasher.finish()
    }

    fn compute(&self, block: &Block) -> Item {
        Item {
            list: block.list,
            level: block.level.min(LEVELS as u8 - 1),
        }
    }
}

/// A label: the list and its counters down to the item's level.
pub fn digest(list: u32, counters: &[u32]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ u64::from(list);
    for &x in counters {
        h = (h ^ u64::from(x)).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn label(state: &mut Counters, list: u32, level: u8) -> u64 {
    let level = usize::from(level).min(LEVELS - 1);
    bump(state, level);
    digest(list, state.get(..=level).unwrap_or_default())
}

/// The labels of blocks `from..to`, incrementally: the counters before `from` of the lists
/// the range has items of, from the tree's prefix, then stepped through the range.
pub fn labels(values: &MonoidTree<Item>, from: usize, to: usize) -> Result<Vec<Option<u64>>, DeriveError> {
    let count = to
        .checked_sub(from)
        .ok_or(DeriveError::OutOfRange { at: from, len: to })?;
    let mut wanted: Vec<u32> = values
        .iter_from(from)?
        .take(count)
        .map(|item| item.list)
        .filter(|&l| l != 0)
        .collect();
    wanted.sort_unstable();
    wanted.dedup();
    let start = values.prefix_by(from, |lists| lists.only(&wanted))?;
    let mut state: HashMap<u32, Counters> = HashMap::new();
    Ok(values
        .iter_from(from)?
        .take(count)
        .map(|item| {
            if item.list == 0 {
                return None;
            }
            let counters = state.entry(item.list).or_insert_with(|| {
                start
                    .get(item.list)
                    .map_or([0; LEVELS], |step| apply(step, &[0; LEVELS]))
            });
            Some(label(counters, item.list, item.level))
        })
        .collect())
}

/// The oracle: every label from scratch.
pub fn full(blocks: &[Block]) -> Vec<Option<u64>> {
    let mut state: HashMap<u32, Counters> = HashMap::new();
    blocks
        .iter()
        .map(|block| {
            (block.list != 0).then(|| label(state.entry(block.list).or_insert([0; LEVELS]), block.list, block.level))
        })
        .collect()
}

/// A seeded source (xorshift).
pub struct Rng(pub u64);

impl Rng {
    /// A number below `n`.
    pub fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n.max(1)
    }
}

/// A made-up list item's list and level, shaped like a long report's: two long lists
/// running through the whole document, many short ones, mostly level 0.
pub fn some_item(rng: &mut Rng) -> (u32, u8) {
    let list = match rng.below(100) {
        0..=29 => 1,
        30..=57 => 4,
        _ => 10 + rng.below(360) as u32,
    };
    let level = match rng.below(100) {
        0..=84 => 0,
        85..=96 => 1,
        _ => 2 + rng.below(3) as u8,
    };
    (list, level)
}

/// `n` made-up blocks, one in eight a list item.
pub fn generate(n: usize, seed: u64) -> Vec<Block> {
    let mut rng = Rng(seed | 1);
    (0..n)
        .map(|i| {
            let (list, level) = if rng.below(8) == 0 { some_item(&mut rng) } else { (0, 0) };
            Block {
                text: format!("paragraph {i}"),
                list,
                level,
            }
        })
        .collect()
}

/// A budget with a fixed number of bytes.
pub struct Budget {
    used: AtomicUsize,
    limit: usize,
}

impl Budget {
    /// A budget of `limit` bytes.
    pub fn new(limit: usize) -> Self {
        Budget {
            used: AtomicUsize::new(0),
            limit,
        }
    }
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
