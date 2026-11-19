//! Owns: `word_copy_from`'s definition (docx-agent's `tools/structure.py`,
//! `word_copy_from`).
//!
//! Must not: change the document but through a command.

use ooxml_tools::{ArgEnum, ToolArgs};

/// Which styles copied blocks keep.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyStyles {
    /// This document's styles of the same names.
    UseDestination,
    /// The source's, renamed where they differ.
    KeepSource,
    /// The source's merged into this document's.
    Merge,
}

/// What a source style this document lacks becomes.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unmapped {
    /// Imported.
    Import,
    /// The body style.
    Body,
}

/// A source style and this document's style it becomes.
// Today's schema describes these fields not at all: a doc comment would become a description.
#[allow(missing_docs)]
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
pub struct StyleMapping {
    pub source: String,
    pub destination: String,
}

/// `word_copy_from`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_copy_from",
    description = include_str!("word_copy_from.md"),
    group = "word_structure",
    mutates,
    documents("doc", "source_doc"),
    exactly_one("range", "section_heading"),
    places = "blocks",
    place_own("at", "range", "section_heading")
)]
pub struct WordCopyFrom {
    /// Document id.
    pub doc: String,
    /// Open document to copy from, e.g. d2.
    pub source_doc: String,
    /// end, `after:<id>`, `before:<id>` or `replace:<id>[..<id>]`.
    #[arg(description = "end, after:<id>, before:<id> or replace:<id>[..<id>].")]
    pub at: String,
    /// Source block or span p:A..t:B.
    pub range: Option<String>,
    /// Or a source heading: its whole section.
    pub section_heading: Option<String>,
    /// Default use_destination.
    pub styles: Option<CopyStyles>,
    /// Source style -> this document's style.
    pub style_map: Option<Vec<StyleMapping>>,
    /// Source styles this document lacks: import them (default) or use body.
    pub unmapped: Option<Unmapped>,
    /// Ref name for the new blocks ($name).
    #[arg(rename = "ref")]
    pub ref_name: Option<String>,
}
