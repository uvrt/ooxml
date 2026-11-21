//! Owns: [`Call`], what the session hands a tool it dispatches -- the document's version the
//! call reads, who calls, the call's cancellation and, for a Word document, the session's
//! editing state and the host's context (its layout, blobs, values written, the other
//! documents the call names) -- and the [`Editing`] an editing tool is run with, made from it.
//!
//! A new kind of host context is a field here and a `with_*` setter in [`Call::edit`],
//! not a change to the registry or to any tool.
//!
//! Must not: run a tool or know any tool's logic.

use crate::document::Word;
use crate::session::Opened;
use ooxml_base::Cancel;
use ooxml_store::Caller;
use ooxml_tools::ToolResult;
use ooxml_tools_word::{CallEditor, Editing, Made, Others, Version};
use serde_json::Value;
use std::sync::Mutex;

/// What a tool is handed: the document's version the call reads, who calls, the call's
/// cancellation, and for a Word document the session's editing state.
pub struct Call<'a> {
    pub(crate) doc: &'a Opened,
    /// The name the call gives its document (the host's; the session's id where the host
    /// gives none): what its reply names it by.
    pub(crate) name: &'a str,
    pub(crate) word: Option<&'a Word>,
    pub(crate) caller: &'a Caller,
    pub(crate) cancel: &'a dyn Cancel,
    /// The revision date edits made now carry (the session's clock, to the minute).
    pub(crate) date: String,
    /// The versions the call's edits made (an editing tool's: [`CallEditor::made`]).
    pub(crate) made: &'a Mutex<Option<Made>>,
    /// The host's layout of the document's versions.
    pub(crate) layout: Option<&'a dyn ooxml_tools_word::WordLayout>,
    /// How long laying a version out may take in the call.
    pub(crate) layout_limit: Option<ooxml_tools_word::LayoutLimit<'a>>,
    /// The session's blobs, where the host hands them over.
    pub(crate) blobs: Option<&'a dyn ooxml_tools_word::Blobs>,
    /// The values the host wrote into the call's arguments for references.
    pub(crate) written: &'a [ooxml_tools_word::Written],
    /// The other documents the call names (`ooxml_tools::documents`), by id, each at the
    /// version pinned for the call when it was dispatched.
    pub(crate) others: &'a [(String, Opened)],
    /// The host's limits on the call's reply.
    pub(crate) limits: ooxml_tools::Limits,
}

impl Call<'_> {
    /// The document at the version the call reads.
    #[must_use]
    pub fn doc(&self) -> &Opened {
        self.doc
    }

    /// The name the call gives its document, which its reply names it by.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name
    }

    /// The call's cancellation.
    #[must_use]
    pub fn cancel(&self) -> &dyn Cancel {
        self.cancel
    }

    /// The version the call reads as the session's editor holds it, with the problems the
    /// document was opened with and the version it was opened as, where the session owns
    /// it as a Word document.
    #[must_use]
    pub fn owned(&self) -> Option<ooxml_tools_word::Owned<'_>> {
        let doc = self.doc.word()?;
        let word = self.word?;
        Some(ooxml_tools_word::Owned {
            doc,
            opened: word.opened.doc(),
            baseline: word.editor.baseline(),
            last_check: &word.last_check,
            layout: self.layout,
            layout_limit: self.layout_limit,
        })
    }

    /// The other documents the call names, by id, at their pinned versions.
    #[must_use]
    pub fn others(&self) -> &[(String, Opened)] {
        self.others
    }

    /// The Word document the session owns, where the call's is one.
    pub(crate) fn word(&self) -> Option<&Word> {
        self.word
    }

    /// `run` with the editing state of the Word document `word` (the call's): the session's
    /// editor, the call's caller, tracking mode and host context; the versions its edits
    /// made are kept for the call (its `made`), and the ids its edits drew as stand-ins are
    /// named in the reply as they were drawn where the edits landed
    /// ([`ooxml_tools_word::resolve_drawn`]).
    pub(crate) fn edit(&self, word: &Word, run: impl FnOnce(&Editing<'_>) -> ToolResult<Value>) -> ToolResult<Value> {
        let policy = word.policy(&self.date);
        let tracking = match &policy {
            ooxml_word_edit::RevisionPolicy::Tracked(tracking) => Some(tracking),
            ooxml_word_edit::RevisionPolicy::Untracked => None,
        };
        let editor = CallEditor::new(&word.editor);
        let others = self.other_versions();
        let editing = Editing::new(
            &editor,
            self.caller,
            word.opened.doc(),
            self.name,
            self.cancel,
            &self.date,
        )
        .with_tracking(tracking)
        .with_layout(self.layout.map(|layout| layout as &dyn ooxml_word_edit::FieldLayout))
        .with_layout_limit(self.layout_limit)
        .with_others(Others::new(&others))
        .with_blobs(self.blobs)
        .with_written(self.written)
        .with_rows_written(Some(&word.rows_written))
        .with_limits(self.limits);
        let mut result = run(&editing);
        *self.made.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = editor.made();
        ooxml_tools_word::resolve_drawn(&mut result, &editor.moved());
        result
    }

    /// The version the call reads as a reading tool reads it: with the other documents it
    /// names and the session's own reading of the version.
    pub(crate) fn read<R>(&self, run: impl FnOnce(&Version<'_>) -> R) -> R {
        let others = self.other_versions();
        let version = self
            .doc
            .version_of()
            .with_others(Others::new(&others))
            .with_owned(self.owned())
            .with_limits(self.limits);
        run(&version)
    }

    /// [`Call::others`] as a tool reads them.
    fn other_versions(&self) -> Vec<(&str, Version<'_>)> {
        self.others
            .iter()
            .map(|(id, o)| (id.as_str(), o.version_of()))
            .collect()
    }
}
