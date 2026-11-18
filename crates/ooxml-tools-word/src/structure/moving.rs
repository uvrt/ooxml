//! Owns: `word_move`'s definition (docx-agent's `tools/structure.py`, `word_move`).
//!
//! Must not: change the document but through a command.

use ooxml_tools::ToolArgs;

/// `word_move`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_move",
    description = include_str!("word_move.md"),
    group = "word_structure",
    mutates,
    exactly_one("section_heading", "blocks"),
    places = "sides",
    place_own("section_heading", "blocks", "after", "before", "after_section", "target", "where")
)]
pub struct WordMove {
    /// Document id.
    pub doc: String,
    /// Heading whose section moves.
    #[arg(refs)]
    pub section_heading: Option<String>,
    /// Or a block or span p:A..t:B.
    #[arg(refs)]
    pub blocks: Option<String>,
    /// Move after this block.
    #[arg(refs)]
    pub after: Option<String>,
    /// Move before this block.
    #[arg(refs)]
    pub before: Option<String>,
    /// Move after this heading's section.
    #[arg(refs)]
    pub after_section: Option<String>,
}
