//! Owns: footnotes and endnotes as the layout sees them -- notes numbered and endnotes
//! placed in the flow ([`numbering`]); a footnote's (or a separator's) paragraphs as the
//! lines a page's foot stacks, and where widow control lets a note split ([`lines`])
//! (docx2svg's `notes` and the notes' part of `paginate`).
//!
//! Must not: place a note on a page (the paginator does), draw, or read XML.

pub mod flowed;
pub mod lines;
pub mod numbering;

pub use lines::{note_minimum, note_prefix, note_spacing, NoteLine, NoteLines};
pub use numbering::{
    footnote_numbers, footnote_references, footnotes_by_page, separator, with_endnotes, with_footnotes, FootnotePages,
    FootnoteReference, NotesPart, Placed,
};

/// What of the notes a block placed in the flow is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoteRole {
    /// The endnotes' separator.
    Separator,
    /// An endnote's block.
    Endnote,
}

/// Note id -> the page (0-based, blank pages counted) its reference's line is on, the
/// first reference of a note winning (`_footnote_pages`).
pub fn footnote_pages(layout: &crate::DocumentLayout) -> FootnotePages {
    let mut line_pages: std::collections::HashMap<(usize, usize), usize> = std::collections::HashMap::new();
    for page in &layout.pagination.pages {
        for line in &page.lines {
            line_pages.entry((line.item, line.line)).or_insert(page.page);
        }
    }
    let mut out = FootnotePages::new();
    for (index, item) in layout.flow.items.iter().enumerate() {
        let crate::flow::FlowItem::Para(p) = item else { continue };
        for (line, lines) in &p.notes {
            let (Some(page), Some(first)) = (line_pages.get(&(index, *line)), lines.first()) else {
                continue;
            };
            if let Some(note) = first.note {
                out.entry(note).or_insert(*page);
            }
        }
    }
    out
}
