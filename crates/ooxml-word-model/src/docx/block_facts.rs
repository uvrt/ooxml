//! Owns: the block tier's slot for facts of one block that its reader names ([`BlockFact`]),
//! kept across a document's versions: one memo per document ([`BlockFacts`]) on the
//! document's budget, each value under its fact and its block's element, and valid while
//! the block's element, the outermost table holding it (whose reading gives a paragraph's
//! cell) and the formatting parts are the ones it was made from -- all by identity, held
//! weakly. A block no edit touched keeps its facts wherever it moved.
//!
//! The crate that reads a fact (the editing crate's `check` facts) declares it and makes its
//! value; this crate owns what the key is built from (the tree, the tables, the formatting
//! parts) and keeps the values, without knowing what they are.
//!
//! Must not: key by a path or an id, keep a value whose block no version holds past a
//! sweep, or hold a fact that reads anything but its block, the outermost table holding it
//! and the formatting parts (a fact reading its neighbours or its place does not fit here).

use super::cells::{held_roots, same_roots, HeldRoots};
use super::formatting::Formatting;
use crate::ids::W;
use ooxml_derive::{CacheBudget, Limit, Memo, MemoStats, Weigh};
use ooxml_xml::Element;
use std::any::{Any, TypeId};
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// What the block facts hold at most where no budget is given, bytes.
pub const BLOCK_FACTS_BYTES: usize = 16 << 20;

/// A fact of one block: a pure function of the block's element, the outermost table holding
/// it and the formatting parts.
pub trait BlockFact: 'static {
    /// Its value for one block.
    type Value: Clone + Send + Sync + 'static;

    /// What a value holds, bytes, roughly.
    fn weight(value: &Self::Value) -> usize;
}

/// One block's value of one fact, and the elements it was made from, held weakly.
#[derive(Clone)]
struct Kept {
    element: Weak<Element>,
    outer: Option<Weak<Element>>,
    roots: HeldRoots,
    value: Arc<dyn Any + Send + Sync>,
    bytes: usize,
}

impl Kept {
    /// Whether it was made from `element` in `outer` with `formatting`'s parts.
    fn of(&self, element: &Arc<Element>, outer: Option<&Arc<Element>>, formatting: &Formatting) -> bool {
        let same_outer = match (&self.outer, outer) {
            (Some(kept), Some(now)) => std::ptr::eq(kept.as_ptr(), Arc::as_ptr(now)),
            (None, None) => true,
            _ => false,
        };
        std::ptr::eq(self.element.as_ptr(), Arc::as_ptr(element)) && same_outer && same_roots(&self.roots, formatting)
    }

    /// Whether a version can still ask for it: its element is held.
    fn alive(&self) -> bool {
        self.element.strong_count() > 0
    }
}

impl Weigh for Kept {
    fn weight(&self) -> usize {
        std::mem::size_of::<Kept>() + self.bytes
    }
}

/// A fact's value of a block: by the fact and the block element's address.
type Key = (TypeId, usize);

/// The block facts kept across a document's versions, by fact and the block element's
/// address (a weak reference keeps the address its own while it is kept), and how many
/// were alive when last swept.
pub(crate) struct BlockFacts {
    memo: Mutex<(Memo<Key, Kept>, usize)>,
}

impl std::fmt::Debug for BlockFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let memo = self.memo.lock().unwrap_or_else(PoisonError::into_inner);
        f.debug_struct("BlockFacts").field("stats", &memo.0.stats()).finish()
    }
}

impl Default for BlockFacts {
    fn default() -> BlockFacts {
        BlockFacts::new(Arc::new(Limit::new(BLOCK_FACTS_BYTES)))
    }
}

impl BlockFacts {
    /// An empty memo counted on `budget`.
    pub(crate) fn new(budget: Arc<dyn CacheBudget>) -> BlockFacts {
        BlockFacts {
            memo: Mutex::new((Memo::new(budget), 0)),
        }
    }

    /// What it holds.
    pub(crate) fn stats(&self) -> MemoStats {
        self.memo.lock().unwrap_or_else(PoisonError::into_inner).0.stats()
    }

    /// `F`'s value for the block `element` (in `outer`, the outermost table holding it) with
    /// `formatting`: as kept, else made by `read` and kept.
    pub(crate) fn get<F: BlockFact>(
        &self,
        formatting: &Formatting,
        element: &Arc<Element>,
        outer: Option<&Arc<Element>>,
        read: impl FnOnce() -> F::Value,
    ) -> F::Value {
        let key: Key = (TypeId::of::<F>(), Arc::as_ptr(element) as usize);
        let found = {
            let mut memo = self.memo.lock().unwrap_or_else(PoisonError::into_inner);
            memo.0
                .get(&key)
                .filter(|k| k.of(element, outer, formatting))
                .and_then(|k| k.value.downcast_ref::<F::Value>().cloned())
        };
        if let Some(found) = found {
            return found;
        }
        // Made outside the lock: another reader of another block is not held up.
        let value = read();
        let kept = Kept {
            element: Arc::downgrade(element),
            outer: outer.map(Arc::downgrade),
            roots: held_roots(formatting),
            value: Arc::new(value.clone()),
            bytes: F::weight(&value),
        };
        let mut memo = self.memo.lock().unwrap_or_else(PoisonError::into_inner);
        let (kept_now, held) = &mut *memo;
        kept_now.insert(key, kept);
        // Facts of blocks no version holds: swept once the memo has doubled since.
        if kept_now.stats().entries > 2 * *held + 64 {
            kept_now.sweep(Kept::alive);
            *held = kept_now.stats().entries;
        }
        value
    }
}

/// The outermost table on the way from `root` down to the element at `place` (child
/// indexes from the root), the element itself left out; `None` outside tables.
pub(crate) fn outer_table<'a>(root: &'a Arc<Element>, place: &[u32]) -> Option<&'a Arc<Element>> {
    let (_, above) = place.split_last()?;
    let mut node = root;
    for &k in above {
        node = node.children().get(k as usize)?.as_element()?;
        if node.name().is(Some(W), "tbl") {
            return Some(node);
        }
    }
    None
}
