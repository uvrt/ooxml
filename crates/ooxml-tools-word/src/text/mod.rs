//! Word's text tools (docx-agent's `tools/text.py` and the shared `replace_text`):
//! `word_set_text`, `replace_text`, `word_insert_text`, `word_delete` and
//! `word_insert_markdown`, over the edit core's commands.
//!
//! Owns: the tools' definitions (descriptions beside them), their handlers -- each call's
//! edits one command through the editor (one history step), written under the document's
//! tracking mode -- and what a call needs that is not supported yet, refused before
//! anything changes ([`Unported`]).
//!
//! Must not: hold sessions or documents by id, keep state between calls, or write the
//! document but through a command.

mod delete;
mod fields;
mod insert_markdown;
mod insert_text;
mod replace_text;
mod set_text;

pub use delete::{word_delete, WordDelete};
pub use insert_markdown::{word_insert_markdown, StyleElement, StyleMapEntry, WordInsertMarkdown};
pub use insert_text::{word_insert_text, InsertItem, Where, WordInsertText};
pub use replace_text::{replace_text, Expect, ReplaceStories, ReplaceText};
pub use set_text::{word_set_text, SetTextItem, WordSetText};

use crate::Editing;
use ooxml_tools::{ToolError, TryCall, WayOut};
use ooxml_word_edit::RevisionPolicy;

/// What a text tool's call needs that the Rust core does not do yet: each is refused as not
/// supported ([`ToolError::not_supported`]), its form named `"<tool> <what>"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unported {
    /// Text with `**strong**`, `*emphasis*`, `[text]{Style}` or escapes.
    Styled,
    /// A `$ref` address (a value by reference reaches the tool written as its value).
    Reference,
    /// Content from data the host holds: rows or a field (`from`), a blob.
    HostData,
    /// A table whose frame new tables take (`like` naming a table).
    Like,
}

impl Unported {
    /// What the form's id says after the tool's name.
    #[must_use]
    pub fn what(self) -> &'static str {
        match self {
            Unported::Styled => "styled text",
            Unported::Reference => "by reference",
            Unported::HostData => "from data (from, blob)",
            Unported::Like => "like",
        }
    }

    /// The refusal: `call` (the same call) is the way out, with `fill` the arguments to give
    /// literally.
    #[must_use]
    pub fn refusal(self, call: TryCall, fill: &[&str]) -> ToolError {
        let form = format!("{} {}", call.tool, self.what());
        not_supported(&form, call, fill)
    }
}

/// A form of a text tool's call not supported yet, refused with `call` as the way out and
/// `fill` the arguments to give otherwise.
#[must_use]
pub(crate) fn not_supported(form: &str, mut call: TryCall, fill: &[&str]) -> ToolError {
    call.fill.extend(fill.iter().map(|f| (*f).to_owned()));
    ToolError::not_supported(form, WayOut::call(call))
}

/// What of `text`, written as a tool writes it, is not supported yet.
/// Braces are literal text here: the host writes every reference (`{{d:...}}`) as its value
/// before the call reaches the session, and refuses one for a tool that does not keep it.
pub(crate) fn text_unported(text: &str) -> Option<Unported> {
    if text.contains('*') || text.contains('\\') || text.contains("]{") {
        return Some(Unported::Styled);
    }
    None
}

/// Whether an address is a ref the host resolves (`$name`).
pub(crate) fn by_reference(address: &str) -> bool {
    address.starts_with('$')
}

impl Editing<'_> {
    /// The policy edits are written under: tracked by the mode's author, or not.
    #[must_use]
    pub fn policy(&self) -> RevisionPolicy {
        self.tracking
            .map_or(RevisionPolicy::Untracked, |t| RevisionPolicy::Tracked(t.clone()))
    }
}

/// `renamed` merged into `into` as a dict update: a key kept in its place, its value the
/// latest.
pub(crate) fn merge_renamed(into: &mut Vec<(String, String)>, renamed: &[(String, String)]) {
    for (old, new) in renamed {
        match into.iter_mut().find(|(k, _)| k == old) {
            Some(entry) => entry.1.clone_from(new),
            None => into.push((old.clone(), new.clone())),
        }
    }
}

/// Registers the group's tools, in the order a model is shown them (the crate's
/// [`crate::register`]).
pub(crate) fn register(r: &mut impl ooxml_tools::Registrar<crate::Word>) {
    set_text::register(r);
    replace_text::register(r);
    insert_text::register(r);
    delete::register(r);
    insert_markdown::register(r);
}
