//! Word's text tools (docx-agent's `tools/text.py` and the shared `replace_text`):
//! `word_set_text`, `replace_text`, `word_insert_text`, `word_delete` and
//! `word_insert_markdown`, over the edit core's commands.
//!
//! Owns: the tools' definitions (descriptions beside them), their handlers -- each call's
//! edits one command through the editor (one history step), written under the document's
//! tracking mode -- and what a call needs that is not ported yet, said in `unported`
//! before anything changes ([`Unported`]) for the host to answer its own way.
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
use ooxml_tools::{Done, ToolResult};
use ooxml_word_edit::RevisionPolicy;
use serde_json::Value;

/// What a text tool's call needs that the Rust core does not do as docx-agent does yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unported {
    /// Text with `**strong**`, `*emphasis*`, `[text]{Style}` or escapes (lane P2b3).
    Styled,
    /// A value written by reference (`{{d:...}}`) or a `$ref` address: the host's.
    Reference,
    /// A paragraph style set with the text.
    ParagraphStyle,
    /// An address form or option the handler does not serve yet.
    Form,
    /// Content from data the host holds: rows or a field (`from`), a blob.
    HostData,
    /// Blocks whose look new ones take (`like`).
    Like,
}

impl Unported {
    /// What `unported` says.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Unported::Styled => "styled text (strong, emphasis, character styles)",
            Unported::Reference => "values or addresses by reference",
            Unported::ParagraphStyle => "a paragraph style set with the text",
            Unported::Form => "an address form or option not ported",
            Unported::HostData => "content from the host's data (from, blob)",
            Unported::Like => "blocks looking like given ones (like)",
        }
    }

    /// The reply saying so: nothing changed, the host answers the call its own way.
    #[must_use]
    pub fn reply(self) -> ToolResult<Value> {
        let mut done = Done::new(String::new());
        done.unported.push(self.reason().to_owned());
        ToolResult::done(done)
    }
}

/// What of `text`, written as a tool writes it, is not ported yet.
pub(crate) fn text_unported(text: &str) -> Option<Unported> {
    if text.contains("{{") {
        return Some(Unported::Reference);
    }
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
pub(crate) fn register(r: &mut impl crate::Registrar) {
    set_text::register(r);
    replace_text::register(r);
    insert_text::register(r);
    delete::register(r);
    insert_markdown::register(r);
}
