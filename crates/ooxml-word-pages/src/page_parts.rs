//! Owns: how the page tier keeps one page's contributions to the layout facts (ADR-008,
//! `docs/architecture/README.md` section 7): a contribution is made from what the tier
//! keeps of the page -- its read ([`crate::page_entry::PageEntry`]) or its layout facts
//! ([`crate::laid_entry::LaidEntry`]) -- by a pure function of that alone ([`Part`]), once,
//! when first asked, and kept in its entry's [`Slot`]; the entry is weighed before any is
//! made by what each can hold at most ([`Part::weight_of`]). A layout fact is a fold over
//! its pages' contributions: a check after a local edit makes them only for the pages it
//! changed.
//!
//! Must not: read another page, the version's ids or a call's arguments into a
//! contribution (what depends on them is the fold's).

use std::sync::OnceLock;

/// One page's contribution to a layout fact, made from `S`: what the page tier keeps of
/// the page.
pub trait Part<S: ?Sized>: Sized {
    /// The contribution of a page whose source is `source`.
    fn of(source: &S) -> Self;

    /// What the contribution of a page whose source is `source` holds at most, bytes: what
    /// its entry is weighed by before it is made.
    fn weight_of(source: &S) -> usize;
}

/// Where an entry keeps one contribution: made once, when first asked.
#[derive(Debug)]
pub struct Slot<P>(OnceLock<P>);

impl<P> Default for Slot<P> {
    fn default() -> Self {
        Slot(OnceLock::new())
    }
}

impl<P> Slot<P> {
    /// The contribution, made from `source` when first asked.
    pub fn get<S: ?Sized>(&self, source: &S) -> &P
    where
        P: Part<S>,
    {
        self.0.get_or_init(|| P::of(source))
    }

    /// Whether it was made.
    #[must_use]
    pub fn is_made(&self) -> bool {
        self.0.get().is_some()
    }
}
