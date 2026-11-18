//! Incremental derived data (docs/architecture/README.md, section 7, ADR-008; layer 0).
//!
//! Owns: the primitives every derived fact is built from -- [`Memo`], per-block facts
//! memoised by content under a [`CacheBudget`]; [`MonoidTree`], a persistent sequence
//! whose order-dependent aggregate ([`Monoid`]) is kept in O(log n) per change; and
//! [`PersistentMap`], a hash map whose versions share their unchanged shards, for the
//! whole-document facts (counts of ids in use) kept from a change's blocks -- and
//! [`BlockFacts`], which keeps one fact over a sequence of blocks up to date per changed
//! block.
//!
//! Must not: know any format, any document type or any particular fact; read the clock or
//! hold global state. A fact (list labels, heading numbers, the block-to-page index)
//! lives with its format and ships an oracle that recomputes it from scratch, against which
//! property tests check the incremental result (tests/list_labels.rs is the reference).
#![forbid(unsafe_code)]

mod blocks;
mod budget;
mod error;
mod map;
mod memo;
mod monoid;
mod tree;

pub use blocks::{BlockFact, BlockFacts, Edit};
pub use budget::CacheBudget;
pub use error::DeriveError;
pub use map::PersistentMap;
pub use memo::{Memo, MemoStats, Weigh};
pub use monoid::{Measured, Monoid};
pub use tree::{Iter, MonoidTree};
