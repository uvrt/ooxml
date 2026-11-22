//! Word's style tools (docx-agent's `tools/style.py` and `word_format` of `tools/text.py`):
//! `word_lists`, `word_format`, `word_styles` and `word_template`, over the edit core
//! (`ooxml-word-edit`); `word_compare_blocks` and `word_design_facts`, reading tools;
//! `word_follow_sample`.
//!
//! Owns: the tools' definitions (descriptions beside them) and their handlers: a function
//! of the typed arguments and an [`crate::Editing`], whose edits run as one command each
//! through the editor (one history step).
//!
//! Must not: hold sessions or documents by id, keep state between calls, or write the
//! document but through a command.

pub(crate) mod compare;
mod compare_args;
pub(crate) mod compare_paragraphs;
pub(crate) mod compare_tables;
mod design;
mod design_args;
mod design_layout;
mod design_look;
mod design_styles;
mod design_tables;
mod find;
mod follow;
mod follow_report;
mod format;
mod format_args;
mod lists;
mod moves;
mod style_view;
mod styles;
mod styles_args;
mod template;

pub use compare::word_compare_blocks;
pub use compare_args::{BlockIn, FollowReport, WordCompareBlocks, WordFollowSample};
pub use design::word_design_facts;
pub use design_args::{DesignKind, WordDesignFacts};
pub use follow::word_follow_sample;
pub use format::word_format;
pub use format_args::{Alignment, Cleared, FindExpect, FindItem, WordFormat};
pub use lists::{word_lists, Follow, ListKind, ListsAction, WordLists};
pub use styles::word_styles;
pub use styles_args::{
    RegionBorderStyle, RegionBordersArg, RegionItem, RegionName, RegionSide, StyleKind, StyleNumbering, StylesAction,
    TabAlignment, TabItem, TabLeader, WordStyles,
};
pub use template::{word_template, TemplateAction, WordTemplate};

/// Registers the group's tools, in the order a model is shown them (the crate's
/// [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    lists::register(r);
    format::register(r);
    styles::register(r);
    template::register(r);
    compare::register(r);
    follow::register(r);
    design::register(r);
}
