//! Owns: `word_links`' definition (docx-agent's `tools/structure.py`, `word_links`).
//!
//! Must not: change the document but through a command.

use ooxml_tools::{ArgEnum, ToolArgs};

/// What `word_links` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkAction {
    /// List bookmarks and hyperlinks.
    List,
    /// Make text a hyperlink.
    AddHyperlink,
    /// Bookmark text or blocks.
    AddBookmark,
    /// Rename a bookmark and what points at it.
    RenameBookmark,
    /// Remove a bookmark's markers.
    RemoveBookmark,
}

/// `word_links`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(name = "word_links", description = include_str!("word_links.md"), group = "word_structure", mutates)]
pub struct WordLinks {
    /// Document id.
    pub doc: String,
    /// What to do.
    pub action: LinkAction,
    /// add_*: range or paragraph; add_bookmark: or blocks (t:X, p:A..t:B).
    #[arg(refs)]
    pub target: Option<String>,
    /// add_*: or text occurring once.
    pub find: Option<String>,
    /// add_hyperlink: external address.
    pub url: Option<String>,
    /// Bookmark name; add_hyperlink: link to it instead of url.
    pub bookmark: Option<String>,
    /// rename_bookmark: new name.
    pub new_name: Option<String>,
}
