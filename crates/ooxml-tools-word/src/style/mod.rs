//! Word's style tools (docx-agent's `tools/style.py` and `word_format` of `tools/text.py`):
//! `word_lists`, `word_format` and `word_styles`, over the edit core (`ooxml-word-edit`).
//!
//! Owns: the tools' definitions (descriptions beside them) and their handlers: a function
//! of the typed arguments and an [`crate::Editing`], whose edits run as one command each
//! through the editor (one history step).
//!
//! Must not: hold sessions or documents by id, keep state between calls, or write the
//! document but through a command.

mod find;
mod format;
mod format_args;
mod lists;
mod moves;
mod style_view;
mod styles;
mod styles_args;

pub use format::word_format;
pub use format_args::{Alignment, Cleared, FindExpect, FindItem, WordFormat};
pub use lists::{word_lists, Follow, ListKind, ListsAction, WordLists};
pub use styles::word_styles;
pub use styles_args::{
    RegionBorderStyle, RegionBordersArg, RegionItem, RegionName, RegionSide, StyleKind, StyleNumbering, StylesAction,
    TabAlignment, TabItem, TabLeader, WordStyles,
};
