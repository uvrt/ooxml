//! Owns: Word's tool types for registration ([`Word`]: the version a reading tool reads,
//! the editing state an editing tool is handed, the tracking mode a tool leaves), and
//! [`register`], every Word tool in the order a model is shown them, through a
//! [`Registrar`] of Word's types (the session's registry).
//!
//! To add a tool: a `register` function in its handler's module (one `Registrar` call: its
//! arguments' struct and its handler), called from its group's `register` (or, for a tool
//! outside a group, from [`register`] here). Nothing else lists the tools.
//!
//! Must not: hold the registry (the session does) or know document kinds besides Word's.

use crate::{Editing, Tracked, Version};
use ooxml_tools::{FormatTypes, Registrar};

/// Word's tool types: what its handlers are given.
#[derive(Debug, Clone, Copy)]
pub enum Word {}

impl FormatTypes for Word {
    type Version<'a> = Version<'a>;
    type Editing<'a> = Editing<'a>;
    type Tracked = Tracked;
}

/// Every Word tool, in the order a model is shown them: `describe` first, then the other
/// reading tools outside a group, then each group's, then `check`.
pub fn register(r: &mut impl Registrar<Word>) {
    crate::describe::register(r);
    crate::word_read::register(r);
    crate::word_inspect::register(r);
    crate::find_text::register(r);
    crate::review::register(r);
    crate::text::register(r);
    crate::objects::register(r);
    crate::style::register(r);
    crate::structure::register(r);
    crate::document::register(r);
    crate::check::register(r);
}
