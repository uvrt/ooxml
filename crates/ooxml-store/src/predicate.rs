//! Owns: [`ReadPredicate`], what a transaction read by a condition rather than by key (a find:
//! "every block that holds this text"), checked at commit against the keys written since the
//! transaction's snapshot as the head holds them (design section 5, predicate reads).
//!
//! Must not: know any format; the format crates implement the trait.

use crate::DocModel;
use std::fmt::Debug;

/// A read by condition. A transaction that records one depends on every key whose content,
/// as any later version holds it, meets the condition, and on nothing else through it: at
/// commit, a key written since the snapshot conflicts when [`ReadPredicate::depends`]
/// answers `true` for it on the head.
///
/// Contract: the transaction must also read, by key, every key that met the condition on its
/// snapshot and that its outcome used (a find's matches), so that a key that stops meeting
/// the condition after the snapshot is a conflict through those reads. Then a commit that
/// passes the check gives what `prepare` would have made on the head.
pub trait ReadPredicate<D: DocModel>: Debug + Send + Sync {
    /// Whether the transaction's outcome may depend on `key` as `head` holds it (`true` when
    /// not known). Runs inside the commit lock, so it must cost what reading the one key
    /// costs.
    fn depends(&self, head: &D, key: &D::Key) -> bool;
}
