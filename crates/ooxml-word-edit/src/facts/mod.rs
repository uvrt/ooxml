//! Owns: `check`'s facts read from the document alone (docx-agent's `layout_facts.py`
//! functions that need no layout): blank paragraphs used as space ([`empty_paragraphs`]),
//! list levels whose labels show a level nothing uses ([`numbering_skips_level`]), tables off
//! their grid or ragged ([`table_grid_mismatch`]), the cells Word's Accept All removes
//! with their text ([`accept_loses_text`]) and the rows the session wrote unlike the rows
//! around them ([`row_text_format`]) -- each as typed
//! entries in document order, with what fixes them.
//!
//! Must not: lay out, filter by pages or ranges, cap or word a reply (the tool layer does),
//! or change the document.

mod accept;
mod empty;
mod grid;
mod numbering;
mod rows;
mod scratch;
mod shapes;

pub use accept::{accept_loses_text, LostCell};
pub use empty::{empty_paragraphs, EmptyRun};
pub use grid::{table_grid_mismatch, GridFix, GridMismatch, OffGridRow, RaggedRow};
pub use numbering::{numbering_skips_level, SkippedLevel};
pub use rows::row_text_format;

/// Whether `c` is white space as Python's `str.strip` and `str.split` take it.
fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}
