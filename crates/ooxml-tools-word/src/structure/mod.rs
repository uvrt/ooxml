//! Word's structure tools (docx-agent's `tools/structure.py`): `word_move`,
//! `word_copy_from`, `word_sections`, `word_headers_footers`, `word_fields`, `word_notes`
//! and `word_links`, over the edit core (`ooxml-word-edit`).
//!
//! Owns: the tools' definitions (descriptions beside them) and their handlers: a function
//! of the typed arguments and an [`crate::Editing`], whose edits run as one command each
//! through the editor (one history step).
//!
//! Must not: hold sessions or documents by id, keep state between calls, or write the
//! document but through a command.

pub(crate) mod common;
mod copy_from;
mod fields;
mod headers_footers;
mod links;
mod moving;
pub(crate) mod notes;
mod sections;

pub use copy_from::{word_copy_from, CopyStyles, StyleMapping, Unmapped, WordCopyFrom};
pub use fields::{word_fields, FieldAction, ReferenceKind, WordFields};
pub use headers_footers::{word_headers_footers, HeaderAction, PageNumber, StoryType, Which, WordHeadersFooters};
pub use links::{word_links, LinkAction, WordLinks};
pub use moving::{word_move, WordMove};
pub use notes::{word_notes, NoteAction, NoteKind, WordNotes};
pub use sections::{word_sections, Margins, Orientation, SectionAction, SectionStart, WordSections};
