//! Owns: the definitions of `word_compare_blocks` and `word_follow_sample` (docx-agent's
//! `tools/style.py`), byte-equal to today's: a block compared with, or copies made to follow,
//! a reference block here or in another open document (`reference.doc`, `sample.doc`: the
//! declared documents rule, [`ooxml_tools::documents`]).
//!
//! Must not: hold a handler, or resolve the other document (the session does).

use ooxml_tools::{ArgEnum, ToolArgs};

/// A block, here or in another open document.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
pub struct BlockIn {
    /// Its document. Default doc.
    pub doc: Option<String>,
    /// Its block.
    pub block: String,
}

/// `word_compare_blocks`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_compare_blocks",
    description = include_str!("word_compare_blocks.md"),
    group = "word_style",
    documents("doc", "reference.doc")
)]
pub struct WordCompareBlocks {
    /// Document id.
    pub doc: String,
    /// Blocks to compare, t:... or p:....
    #[arg(min_items = 1, refs)]
    pub blocks: Vec<String>,
    /// The block with the expected values.
    pub reference: BlockIn,
    /// `next_cursor` of the previous page.
    #[arg(description = "next_cursor of the previous page.")]
    pub cursor: Option<String>,
}

/// What `word_follow_sample` reports of each copy.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowReport {
    /// Every difference.
    All,
    /// The copies' own differences left out, counted.
    Actionable,
}

/// `word_follow_sample`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_follow_sample",
    description = include_str!("word_follow_sample.md"),
    group = "word_style",
    mutates,
    documents("doc", "sample.doc")
)]
pub struct WordFollowSample {
    /// Document id.
    pub doc: String,
    /// The revised sample, t:... or p:....
    pub sample: BlockIn,
    /// Blocks made from it, t:... or p:....
    #[arg(min_items = 1, refs)]
    pub copies: Vec<String>,
    /// Report what would change, change nothing.
    pub preview: Option<bool>,
    /// actionable: leave out the copies' own (kind own), counted.
    #[arg(empty_is_absent)]
    pub report: Option<FollowReport>,
}
