//! Owns: [`BlockFacts`], one fact kept over a sequence of blocks: each block's value
//! memoised by the block's content key, the values in a [`MonoidTree`] for the fact's
//! order-dependent aggregate, brought up to date per changed block ([`Edit`]).
//!
//! Must not: know what a block or a fact is; read a whole document to bring the fact up to
//! date -- an edit costs its blocks' values and O(log n) in the tree.

use crate::{CacheBudget, DeriveError, Measured, Memo, MemoStats, MonoidTree, Weigh};
use std::hash::Hash;
use std::sync::Arc;

/// A fact over blocks: what a block contributes, computed from its content alone.
pub trait BlockFact {
    /// What a block is (a node of a version, a handle).
    type Block;
    /// The block's content key: equal keys mean equal values (a digest of the block and of
    /// the context the value depends on).
    type Key: Hash + Eq + Clone;
    /// The block's value; its [`Measured::Summary`] is the fact's aggregate.
    type Value: Measured + Weigh;

    /// The content key of `block`.
    fn key(&self, block: &Self::Block) -> Self::Key;
    /// The value of `block`, from scratch.
    fn compute(&self, block: &Self::Block) -> Self::Value;
}

/// A change to the block sequence, by position.
#[derive(Debug, Clone, Copy)]
pub enum Edit<'b, B> {
    /// The block at `at` now has this content.
    Set {
        /// Its position.
        at: usize,
        /// Its new content.
        block: &'b B,
    },
    /// A block inserted before the block at `at`.
    Insert {
        /// Its position.
        at: usize,
        /// Its content.
        block: &'b B,
    },
    /// The block at `at` removed.
    Remove {
        /// Its position.
        at: usize,
    },
}

/// One fact over a sequence of blocks, kept up to date per changed block.
pub struct BlockFacts<F: BlockFact> {
    fact: F,
    memo: Memo<F::Key, F::Value>,
    values: MonoidTree<F::Value>,
}

impl<F: BlockFact> BlockFacts<F> {
    /// The fact over `blocks`, with its memo reserving from `budget`.
    pub fn new<'b>(fact: F, budget: Arc<dyn CacheBudget>, blocks: impl IntoIterator<Item = &'b F::Block>) -> Self
    where
        F::Block: 'b,
    {
        let mut memo = Memo::new(budget);
        let values = blocks
            .into_iter()
            .map(|block| memo.get_or_compute(&fact.key(block), || fact.compute(block)))
            .collect();
        Self { fact, memo, values }
    }

    /// Bring the fact up to date with one edit.
    pub fn apply(&mut self, edit: Edit<'_, F::Block>) -> Result<(), DeriveError> {
        match edit {
            Edit::Set { at, block } => {
                let value = self.value_of(block);
                self.values.set(at, value)
            }
            Edit::Insert { at, block } => {
                let value = self.value_of(block);
                self.values.insert(at, value)
            }
            Edit::Remove { at } => self.values.remove(at).map(drop),
        }
    }

    /// Every block's value in order, with the aggregate; cheap to clone (a snapshot).
    pub fn values(&self) -> &MonoidTree<F::Value> {
        &self.values
    }

    /// The memo's counters.
    pub fn memo_stats(&self) -> MemoStats {
        self.memo.stats()
    }

    fn value_of(&mut self, block: &F::Block) -> F::Value {
        let fact = &self.fact;
        self.memo.get_or_compute(&fact.key(block), || fact.compute(block))
    }
}
