//! Versions, transactions and history for one document, format-neutral
//! (docs/architecture/README.md, sections 4-6; layer 0).
//!
//! Owns: immutable versions published through a lock-free head ([`Store::snapshot`]); the
//! transaction ([`Txn`]) with its typed read and write sets (and its reads by condition,
//! [`ReadPredicate`]) and its change set; the commit
//! (validate, prepare on a snapshot, commit in a short critical section with conflict
//! detection -- a change set disjoint from the commits since its snapshot is rebased past
//! theirs ([`DocModel::rebase`]) -- and redo, then a run under the commit lock after
//! [`StoreConfig::optimistic_tries`] conflicts); batches as one transaction and one history
//! step ([`Batch`]); history (undo, redo, scoped undo that refuses entangled steps); version
//! numbers and id sequences that are never given twice, also across a reopen ([`Counters`]).
//!
//! Must not: know any format, do I/O, read the clock except through the [`Clock`] port, or
//! hold global state. What a key, a change and a document are is the model's
//! ([`DocModel`]); this crate only orders, checks and keeps them.
//!
//! Concurrency contract: a command's `prepare` records, through [`Txn::read`], every key its
//! changes depend on, and [`DocModel::apply`] of its changes to any version that agrees with
//! the snapshot on those keys gives what `prepare` would have given there. Under that
//! contract every run equals a serial replay in commit order (tests/serial_replay.rs).
#![forbid(unsafe_code)]

mod changeset;
mod clock;
mod command;
mod error;
mod history;
mod ids;
mod journal;
mod keys;
mod model;
mod predicate;
mod stamps;
mod stats;
mod store;
mod sync;
mod txn;
mod version;

pub use clock::Clock;
pub use command::{Batch, Command};
pub use error::{CountersError, Error, StoreError};
pub use history::{HistoryOutcome, StepInfo};
pub use ids::{Counters, IdSeq};
pub use keys::KeySet;
pub use model::DocModel;
pub use predicate::ReadPredicate;
pub use stats::StoreStats;
pub use store::{Caller, CommitTiming, Committed, Store, StoreConfig};
pub use txn::{Savepoint, Txn};
pub use version::{Seq, Snapshot, Version};
