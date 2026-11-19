//! Word's style tools (docx-agent's `tools/style.py` and `word_format` of `tools/text.py`):
//! `word_lists` so far, over the edit core (`ooxml-word-edit`).
//!
//! Owns: the tools' definitions (descriptions beside them) and their handlers: a function
//! of the typed arguments and an [`crate::Editing`], whose edits run as one command each
//! through the editor (one history step).
//!
//! Must not: hold sessions or documents by id, keep state between calls, or write the
//! document but through a command.

mod lists;

pub use lists::{word_lists, Follow, ListKind, ListsAction, WordLists};
