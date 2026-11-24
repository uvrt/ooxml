//! Owns: the layout half of the page tier of the caching policy (ADR-008): one page's
//! contributions to the layout facts made from its layout facts alone, without painting it
//! ([`LaidEntry`]: the body line lowest on it, [`crate::layout_facts::keeps::FootPart`];
//! the body tables' rows it draws lines of, [`crate::layout_facts::tables::RowsPart`]),
//! and the memo keeping them across a generation's versions by the page's facts
//! ([`LaidMemo`]): a version keeps a page's facts, the very same, where it lays the page
//! out the same ([`ooxml_word_layout::facts::LayoutFacts::after`]), so a check after a
//! local edit makes them only for the pages the edit changed.
//!
//! It is held within the engine's paint budget (the session's bounded account), the least
//! recently used let go first; what it keeps of a page is never the page's facts (a weak
//! reference tells them again), and an entry whose page's facts no version holds is swept
//! with the memos.
//!
//! Must not: read the page's paint, the version's ids or a call's arguments into a
//! contribution (the fold's), or keep a page's facts alive.

use crate::layout_facts::keeps::FootPart;
use crate::layout_facts::tables::RowsPart;
use crate::page_parts::Part;
use ooxml_derive::{CacheBudget, Memo, Weigh};
use ooxml_word_layout::facts::PageFacts;
use std::sync::{Arc, Weak};

/// One page's contributions made from its layout facts, made together when the page tier
/// first keeps it.
#[derive(Debug, Default)]
pub struct LaidEntry {
    foot: FootPart,
    rows: RowsPart,
}

impl LaidEntry {
    /// The entry of a page whose layout facts are `facts`.
    #[must_use]
    pub fn of(facts: &PageFacts) -> LaidEntry {
        LaidEntry {
            foot: FootPart::of(facts),
            rows: RowsPart::of(facts),
        }
    }

    /// The page's body lines' paragraphs, lowest first.
    #[must_use]
    pub fn foot(&self) -> &FootPart {
        &self.foot
    }

    /// The body tables' rows the page draws lines of.
    #[must_use]
    pub fn rows(&self) -> &RowsPart {
        &self.rows
    }

    /// What it holds at most, bytes, for a page whose layout facts are `facts`.
    fn weight_of(facts: &PageFacts) -> usize {
        std::mem::size_of::<LaidEntry>()
            + <FootPart as Part<PageFacts>>::weight_of(facts)
            + <RowsPart as Part<PageFacts>>::weight_of(facts)
    }
}

/// One page's entry as kept: the page's facts held weakly, and what they hold at most.
#[derive(Clone)]
struct Kept {
    facts: Weak<PageFacts>,
    entry: Arc<LaidEntry>,
    weight: usize,
}

impl Weigh for Kept {
    fn weight(&self) -> usize {
        std::mem::size_of::<Kept>() + self.weight
    }
}

/// Each page's [`LaidEntry`], by its layout facts' identity.
pub(crate) struct LaidMemo {
    memo: Memo<usize, Kept>,
}

impl std::fmt::Debug for LaidMemo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaidMemo").field("stats", &self.memo.stats()).finish()
    }
}

impl LaidMemo {
    /// An empty memo counted on `budget`.
    pub(crate) fn new(budget: Arc<dyn CacheBudget>) -> LaidMemo {
        LaidMemo {
            memo: Memo::new(budget),
        }
    }

    /// The entry of the page whose layout facts are `facts`: kept, or made and kept.
    pub(crate) fn entry(&mut self, facts: &Arc<PageFacts>) -> Arc<LaidEntry> {
        let key = Arc::as_ptr(facts) as usize;
        // A weak reference holds its allocation: the address is the same facts' while kept.
        if let Some(kept) = self
            .memo
            .touch(&key)
            .filter(|k| Weak::as_ptr(&k.facts) == Arc::as_ptr(facts))
        {
            return Arc::clone(&kept.entry);
        }
        let entry = Arc::new(LaidEntry::of(facts));
        let kept = Kept {
            facts: Arc::downgrade(facts),
            entry: Arc::clone(&entry),
            weight: LaidEntry::weight_of(facts),
        };
        self.memo.insert(key, kept);
        entry
    }

    /// What it holds: its entries and the bytes reserved for them.
    pub(crate) fn stats(&self) -> ooxml_derive::MemoStats {
        self.memo.stats()
    }

    /// Drop the entries of pages whose facts no version holds that nothing asked for since
    /// the last sweep: how many went.
    pub(crate) fn sweep(&mut self) -> usize {
        self.memo.sweep(|kept| kept.facts.strong_count() > 0)
    }
}
