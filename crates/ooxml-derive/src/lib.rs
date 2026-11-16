//! Incremental derived data (docs/architecture/README.md, section 7, ADR-008; layer 0).
//!
//! Owns: the two primitives every derived fact is built from -- [`Memo`], per-block facts
//! memoised by content under a [`CacheBudget`], and [`MonoidTree`], a persistent sequence
//! whose order-dependent aggregate ([`Monoid`]) is kept in O(log n) per change -- and
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
mod memo;
mod monoid;
mod tree;

pub use blocks::{BlockFact, BlockFacts, Edit};
pub use budget::CacheBudget;
pub use error::DeriveError;
pub use memo::{Memo, MemoStats, Weigh};
pub use monoid::{Measured, Monoid};
pub use tree::{Iter, MonoidTree};
