//! Owns: `word_notes`' definition (docx-agent's `tools/structure.py`, `word_notes`).
//!
//! Must not: change the document but through a command.

use ooxml_tools::{ArgEnum, ToolArgs};

/// What `word_notes` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteAction {
    /// List the notes.
    List,
    /// Insert a note.
    Insert,
    /// Rewrite a note's text.
    Edit,
    /// Delete a note and its reference.
    Delete,
    /// Move a note's reference.
    Move,
}

/// Which notes.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    /// A footnote.
    Footnote,
    /// An endnote.
    Endnote,
}

/// `word_notes`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_notes",
    description = include_str!("word_notes.md"),
    group = "word_structure",
    mutates,
    places = "mark",
    place_own("at", "note"),
    place_find
)]
pub struct WordNotes {
    /// Document id.
    pub doc: String,
    /// What to do.
    pub action: NoteAction,
    /// insert. Default footnote.
    pub kind: Option<NoteKind>,
    /// insert, move: position or range; the mark goes at its end.
    #[arg(refs)]
    pub at: Option<String>,
    /// insert, move: or text occurring once; the mark goes after it.
    pub find: Option<String>,
    /// edit, delete, move: the note (fn:2, en:1).
    #[arg(refs)]
    pub note: Option<String>,
    /// insert, edit: the text.
    pub text: Option<String>,
    /// insert: ref name for the note.
    #[arg(rename = "ref")]
    pub ref_name: Option<String>,
}
