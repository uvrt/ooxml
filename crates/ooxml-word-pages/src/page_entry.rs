//! Owns: one page's entry in the page tier of the caching policy ([`PageEntry`]; ADR-008,
//! `docs/architecture/README.md` section 7): what a check read of the page ([`PageRead`]),
//! and each layout fact's contribution of that page, made from the read when first asked
//! and kept with it -- the text drawn per paragraph, kind and size
//! ([`crate::layout_facts::sizes::SizesPart`]) and the body cells' lines as measured
//! ([`crate::layout_facts::columns::CellsPart`]).
//!
//! The entry is kept by the engine's page memo ([`crate::page_reads::ReadMemo`]) under
//! what the read depends on, so a page another version lays out the same keeps its
//! contributions too: a check after a local edit makes them only for the pages it
//! changed, and the version's facts are a fold over its pages' contributions.
//!
//! Must not: read another page, the version's ids or a call's arguments into a
//! contribution (a contribution is the page's alone: what depends on the call or on the
//! version's naming is the fold's), or paint.

use crate::layout_facts::columns::CellsPart;
use crate::layout_facts::sizes::SizesPart;
use crate::page_read::PageRead;
use std::sync::OnceLock;

/// One page as the page tier keeps it: its read, and its facts' contributions.
#[derive(Debug, Default)]
pub struct PageEntry {
    read: PageRead,
    sizes: OnceLock<SizesPart>,
    cells: OnceLock<CellsPart>,
}

impl PageEntry {
    /// The entry of a page read as `read`, no contribution made yet.
    #[must_use]
    pub fn new(read: PageRead) -> PageEntry {
        PageEntry {
            read,
            sizes: OnceLock::new(),
            cells: OnceLock::new(),
        }
    }

    /// What a check read of the page.
    #[must_use]
    pub fn read(&self) -> &PageRead {
        &self.read
    }

    /// The page's text drawn per paragraph, kind and size, made once.
    #[must_use]
    pub fn sizes(&self) -> &SizesPart {
        self.sizes.get_or_init(|| SizesPart::of(&self.read))
    }

    /// The page's body cells' lines as measured, by table, made once.
    #[must_use]
    pub fn cells(&self) -> &CellsPart {
        self.cells.get_or_init(|| CellsPart::of(&self.read))
    }

    /// What its contributions hold at most, bytes, made or not: what the memo counts for
    /// them beside the read when it keeps the entry.
    #[must_use]
    pub(crate) fn parts_weight(&self) -> usize {
        SizesPart::weight_of(&self.read) + CellsPart::weight_of(&self.read)
    }
}
