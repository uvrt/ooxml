//! Word's text tools (docx-agent's `tools/text.py` and the shared `replace_text`):
//! `word_set_text` and `replace_text`, over the edit core's commands.
//!
//! Owns: the tools' definitions (descriptions beside them), their handlers -- each call's
//! edits one command through the editor (one history step), written under the document's
//! tracking mode -- and what a call needs that is not ported yet, said in `unported`
//! before anything changes ([`Unported`]) for the host to answer its own way.
//!
//! Must not: hold sessions or documents by id, keep state between calls, or write the
//! document but through a command.

mod replace_text;
mod set_text;

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
    /// Text a language's spacing may change (`: ; ! ?` and guillemets; lane P2b3).
    Spacing,
    /// A value written by reference (`{{d:...}}`) or a `$ref` address: the host's.
    Reference,
    /// A paragraph style set with the text.
    ParagraphStyle,
    /// An address form or option the handler does not serve yet.
    Form,
}

impl Unported {
    /// What `unported` says.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Unported::Styled => "styled text (strong, emphasis, character styles)",
            Unported::Spacing => "a language's spacing of the text written",
            Unported::Reference => "values or addresses by reference",
            Unported::ParagraphStyle => "a paragraph style set with the text",
            Unported::Form => "an address form or option not ported",
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
    if text.chars().any(|c| matches!(c, ':' | ';' | '!' | '?' | '«' | '»')) {
        return Some(Unported::Spacing);
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
