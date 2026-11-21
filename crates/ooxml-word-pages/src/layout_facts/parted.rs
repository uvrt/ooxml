//! Owns: `table_parted` (docx-agent's `table_parted`, round 36 R29): a top-level table drawn
//! on a later page than the paragraph just before it, or the paragraph just after it drawn
//! on a later page than its last row -- where that paragraph belongs with the table (a
//! caption, a lead-in ending in a colon, a line drawn on one line) -- with the keep that
//! moves them together.
//!
//! Must not: list a paragraph that keeps with the next already, a last row that does (both
//! are `keep_with_next_at_foot`'s), or a page break or a section start between them.

use super::keeps::new_page_after;
use super::tree::{page_break_at, stripped_text};
use super::{Block, BlockKind, Facts};
use ooxml_word_edit::{row_keeps_at, TableKeeps};

/// Why a paragraph next to a table belongs with it (`PARTED_KINDS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartedKind {
    /// In the Caption style.
    Caption,
    /// It introduces the table: ends in a colon (before it only).
    Colon,
    /// Drawn on one line (a title or a source line).
    OneLine,
}

impl PartedKind {
    /// docx-agent's name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            PartedKind::Caption => "caption",
            PartedKind::Colon => "colon",
            PartedKind::OneLine => "one_line",
        }
    }
}

/// What moves a line after a table to the table's page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AfterFix {
    /// The table on one page: keep it with the next (`word_format_table`), and together
    /// where it is not kept together yet.
    Table {
        /// Whether to keep it together too.
        keep_together: bool,
    },
    /// Over several pages: the last row's deciding paragraph keeps with the next
    /// (`word_format`).
    Row(String),
}

/// A table parted from a paragraph that belongs with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parted {
    /// The paragraph just before it, on an earlier page; keeping it with the next fixes it.
    LeadIn {
        /// The table's id.
        table: String,
        /// The paragraph's id.
        para: String,
        /// Why it belongs with the table.
        kind: PartedKind,
        /// The paragraph's last page.
        para_page: usize,
        /// The table's first page.
        table_page: usize,
    },
    /// The paragraph just after it, on a later page.
    LineAfter {
        /// The table's id.
        table: String,
        /// The paragraph's id.
        para: String,
        /// Why it belongs with the table.
        kind: PartedKind,
        /// The table's last row, from 0.
        row: usize,
        /// The table's last page.
        row_page: usize,
        /// The paragraph's first page.
        para_page: usize,
        /// What fixes it, where a call does.
        fix: Option<AfterFix>,
    },
}

/// `table_parted`, table by table.
#[must_use]
pub fn table_parted(facts: &Facts<'_>) -> Vec<Parted> {
    let blocks = facts.blocks();
    let mut out = Vec::new();
    for (k, block) in blocks.iter().enumerate() {
        if block.kind != BlockKind::Table {
            continue;
        }
        let pages = facts.table_pages(&block.path);
        let (Some(&first), Some(&last)) = (pages.first(), pages.last()) else {
            continue;
        };
        let before = k
            .checked_sub(1)
            .and_then(|b| blocks.get(b))
            .filter(|b| b.kind == BlockKind::Paragraph);
        if let Some(lead) = before {
            out.extend(lead_in(facts, k, block, lead, first));
        }
        let after = blocks.get(k + 1).filter(|b| b.kind == BlockKind::Paragraph);
        if let Some(after) = after {
            out.extend(line_after(facts, block, after, pages.len(), last));
        }
    }
    out
}

fn lead_in(facts: &Facts<'_>, k: usize, table: &Block<'_>, lead: &Block<'_>, first: usize) -> Option<Parted> {
    let para_page = facts.pages_of(&lead.path).last().copied().filter(|p| *p < first)?;
    if !facts.shown([para_page, first]) {
        return None;
    }
    let kind = parted_kind(facts, lead, true)?;
    if facts.paragraph(lead.element).keep_with_next || new_page_after(facts, k - 1) {
        return None;
    }
    Some(Parted::LeadIn {
        table: table.id.clone(),
        para: lead.id.clone(),
        kind,
        para_page,
        table_page: first,
    })
}

fn line_after(facts: &Facts<'_>, table: &Block<'_>, after: &Block<'_>, pages: usize, last: usize) -> Option<Parted> {
    let para_page = facts.pages_of(&after.path).first().copied().filter(|p| *p > last)?;
    if !facts.shown([last, para_page]) {
        return None;
    }
    let kind = parted_kind(facts, after, false)?;
    if facts.paragraph(after.element).page_break_before || page_break_at(after.element, true) {
        return None;
    }
    let story = facts.docx().body()?;
    let rows = row_keeps_at(facts.docx(), &story.part, &table.place).ok()?;
    let final_row = rows.last().filter(|r| !r.keeps)?;
    let fix = if pages == 1 {
        Some(AfterFix::Table {
            keep_together: !TableKeeps::of(&rows).together,
        })
    } else {
        final_row.lead.as_deref().and_then(|lead| {
            let place: Vec<u32> = table.place.iter().chain(lead).copied().collect();
            Some(AfterFix::Row(story.ids.paragraph_at(&place)?.id))
        })
    };
    Some(Parted::LineAfter {
        table: table.id.clone(),
        para: after.id.clone(),
        kind,
        row: rows.len() - 1,
        row_page: last,
        para_page,
        fix,
    })
}

/// Why a paragraph next to a table belongs with it, if it does (`_parted_kind`).
fn parted_kind(facts: &Facts<'_>, block: &Block<'_>, before: bool) -> Option<PartedKind> {
    let text = stripped_text(block.element);
    if text.is_empty() {
        return None;
    }
    if facts.is_caption(block.element) {
        return Some(PartedKind::Caption);
    }
    if before && text.ends_with(':') {
        return Some(PartedKind::Colon);
    }
    (facts.lines_of(&block.path).len() == 1).then_some(PartedKind::OneLine)
}
