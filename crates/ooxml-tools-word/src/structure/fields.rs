//! Owns: `word_fields`' definition (docx-agent's `tools/structure.py`, `word_fields`).
//!
//! Must not: change the document but through a command.

use ooxml_tools::{ArgEnum, ToolArgs};

/// What `word_fields` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldAction {
    /// List the fields.
    List,
    /// Insert a table of contents.
    InsertToc,
    /// Insert a caption.
    InsertCaption,
    /// Insert a cross-reference.
    InsertCrossReference,
    /// Insert a date.
    InsertDate,
    /// Insert a field.
    InsertField,
    /// Update fields' results.
    Update,
}

/// What a cross-reference shows.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    /// The bookmark's text.
    Text,
    /// Its page.
    Page,
}

/// `word_fields`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_fields",
    description = include_str!("word_fields.md"),
    group = "word_structure",
    mutates,
    places = "mark",
    place_own("after", "before", "at"),
    place_find
)]
pub struct WordFields {
    /// Document id.
    pub doc: String,
    /// What to do.
    pub action: FieldAction,
    /// insert_toc, insert_caption: the block before.
    #[arg(refs)]
    pub after: Option<String>,
    /// insert_toc, insert_caption: the block after.
    #[arg(refs)]
    pub before: Option<String>,
    /// Other inserts: position or range (p:A@5), or find.
    #[arg(refs)]
    pub at: Option<String>,
    /// Other inserts: text occurring once; goes after it.
    pub find: Option<String>,
    /// insert_toc: heading levels. Default 1-3.
    pub levels: Option<String>,
    /// insert_cross_reference: the bookmark.
    pub bookmark: Option<String>,
    /// insert_cross_reference: its text (default) or page.
    pub kind: Option<ReferenceKind>,
    /// insert_caption: Figure (default), Table...
    pub label: Option<String>,
    /// insert_caption: text after the number.
    pub text: Option<String>,
    /// insert_field: the instruction, e.g. NUMPAGES.
    pub instruction: Option<String>,
    /// update: only these fields (fld:...).
    #[arg(refs)]
    pub ids: Option<Vec<String>>,
}
