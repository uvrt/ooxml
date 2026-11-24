//! Owns: one page's entry in the page tier of the caching policy ([`PageEntry`]; ADR-008,
//! `docs/architecture/README.md` section 7): what a check read of the page ([`PageRead`]),
//! and each layout fact's contribution of that page, made from the read when first asked
//! and kept with it -- the text drawn per paragraph, kind and size
//! ([`crate::layout_facts::sizes::SizesPart`]) and the body cells' lines as measured
//! ([`crate::layout_facts::columns::CellsPart`]), the body lines' ink
//! ([`crate::layout_facts::spacers::InkedPart`]) and the body tables' rules and ink
//! ([`crate::layout_facts::margins::TableInkPart`]) -- each a [`crate::page_parts::Part`]
//! of the read.
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
use crate::layout_facts::margins::TableInkPart;
use crate::layout_facts::sizes::SizesPart;
use crate::layout_facts::spacers::InkedPart;
use crate::page_parts::{Part, Slot};
use crate::page_read::PageRead;

/// One page as the page tier keeps it: its read, and its facts' contributions.
#[derive(Debug, Default)]
pub struct PageEntry {
    read: PageRead,
    sizes: Slot<SizesPart>,
    cells: Slot<CellsPart>,
    inked: Slot<InkedPart>,
    tables: Slot<TableInkPart>,
}

impl PageEntry {
    /// The entry of a page read as `read`, no contribution made yet.
    #[must_use]
    pub fn new(read: PageRead) -> PageEntry {
        PageEntry {
            read,
            ..PageEntry::default()
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
        self.sizes.get(&self.read)
    }

    /// The page's body cells' lines as measured, by table, made once.
    #[must_use]
    pub fn cells(&self) -> &CellsPart {
        self.cells.get(&self.read)
    }

    /// The page's body lines' characters that are not white space, made once.
    #[must_use]
    pub fn inked(&self) -> &InkedPart {
        self.inked.get(&self.read)
    }

    /// The body tables the page draws a rule or a line's ink of, with their extents, made
    /// once.
    #[must_use]
    pub fn tables(&self) -> &TableInkPart {
        self.tables.get(&self.read)
    }

    /// What its contributions hold at most, bytes, made or not: what the memo counts for
    /// them beside the read when it keeps the entry.
    #[must_use]
    pub(crate) fn parts_weight(&self) -> usize {
        let read = &self.read;
        <SizesPart as Part<PageRead>>::weight_of(read)
            + <CellsPart as Part<PageRead>>::weight_of(read)
            + <InkedPart as Part<PageRead>>::weight_of(read)
            + <TableInkPart as Part<PageRead>>::weight_of(read)
    }
}
