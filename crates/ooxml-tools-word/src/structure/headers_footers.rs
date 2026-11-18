//! Owns: `word_headers_footers`' definition (docx-agent's `tools/structure.py`,
//! `word_headers_footers`).
//!
//! Must not: change the document but through a command.

use ooxml_tools::{ArgEnum, ToolArgs};

/// Which story.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    /// The header.
    Header,
    /// The footer.
    Footer,
}

/// The pages a header or footer is for.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoryType {
    /// Most pages.
    Default,
    /// The section's first page.
    First,
    /// Even pages.
    Even,
}

/// What `word_headers_footers` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderAction {
    /// Set its text.
    Set,
    /// Remove the section's own.
    Remove,
    /// Link it to the previous section's.
    Link,
    /// Give the section its own.
    Unlink,
}

/// The page number a set adds.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageNumber {
    /// None.
    None,
    /// The bare number after the text.
    AfterText,
    /// 'Page X of Y'.
    #[arg(rename = "page_x_of_y")]
    PageXOfY,
}

/// `word_headers_footers`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_headers_footers",
    description = include_str!("word_headers_footers.md"),
    group = "word_structure",
    mutates
)]
pub struct WordHeadersFooters {
    /// Document id.
    pub doc: String,
    /// Section; s:body is the last.
    #[arg(refs)]
    pub section: String,
    /// Which story.
    pub which: Which,
    /// What to do.
    pub action: HeaderAction,
    /// Pages it is for. Default default (most).
    #[arg(rename = "type")]
    pub kind: Option<StoryType>,
    /// set: the text; \n between paragraphs.
    pub text: Option<String>,
    /// set: after_text, the bare number; page_x_of_y, 'Page X of Y' (' | ' after text).
    pub page_number: Option<PageNumber>,
}
