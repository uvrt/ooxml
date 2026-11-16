//! Owns: [`Monoid`], an associative aggregate with an identity, and [`Measured`], what an
//! item of a [`crate::MonoidTree`] contributes to it.
//!
//! Must not: hold any particular aggregate; facts define theirs.

/// An associative combination with an identity: `a.combine(&b).combine(&c)` equals
/// `a.combine(&b.combine(&c))`, and `empty()` changes nothing. Order matters (it need not
/// commute), which is what lets prefix-dependent facts -- counters, numbering, page
/// numbers -- be kept in a tree.
pub trait Monoid: Clone {
    /// The identity.
    fn empty() -> Self;
    /// `self` followed by `other`.
    fn combine(&self, other: &Self) -> Self;
}

/// An item that contributes a summary to its sequence's aggregate.
pub trait Measured: Clone {
    /// The aggregate.
    type Summary: Monoid;
    /// This item's contribution.
    fn measure(&self) -> Self::Summary;
}
