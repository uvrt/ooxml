//! Word's review tools (docx-agent's `tools/review.py`): `word_set_tracking`,
//! `word_changes` and `word_comments`, over the edit core (`ooxml-word-edit`).
//!
//! Owns: the tools' definitions (descriptions beside them), and their handlers: a
//! function of the typed arguments and an [`Editing`] -- the document's editor, the caller,
//! the version it was opened as, its tracking mode -- whose edits run as one command each
//! through the editor (one history step).
//!
//! Must not: hold sessions or documents by id (the host hands a handler its editor), keep
//! state between calls (the tracking mode is the host's, given in and handed back), or
//! write the document but through a command.

mod call_editor;
mod changes;
mod comment_edits;
mod comment_writes;
mod comments;
mod listing;
mod settle;
mod tracking;

pub use call_editor::{CallEditor, Made};
pub use changes::{word_changes, ChangeAction, ChangeDetail, ChangeKind, WordChanges};
pub use comments::{word_comments, CommentAction, CommentItem, WordComments};
pub(crate) use settle::Still;
pub use tracking::{word_set_tracking, Tracked, WordSetTracking};

use ooxml_base::Cancel;
use ooxml_store::Caller;
use ooxml_tools::{Failure, Refusal, ToolError, TryCall, WayOut};
use ooxml_word_edit::{EditError, FieldLayout, Tracking, WordDoc};

/// What an editing tool is handed: the document's editor and what the host keeps of it.
pub struct Editing<'a> {
    /// The document's editor (its versions), for this call: the versions its edits make
    /// are the call's ([`CallEditor::made`]).
    pub editor: &'a CallEditor<'a>,
    /// Who calls.
    pub caller: &'a Caller,
    /// The document as it was opened (what a session's own edits are told from).
    pub opened: &'a WordDoc,
    /// The tracking mode the host keeps for the document: edits by this author are
    /// tracked changes (`None`: off).
    pub tracking: Option<&'a Tracking>,
    /// The document's id, for the calls a refusal offers.
    pub doc: &'a str,
    /// The call's cancellation.
    pub cancel: &'a dyn Cancel,
    /// The call's time as the session's clock reads it (ISO 8601): the day a `DATE` field
    /// shows.
    pub date: &'a str,
    /// The layout of the document's versions, where the host keeps one (`word_fields`'
    /// results); `None`: a result that needs it is left to the host.
    pub layout: Option<&'a dyn FieldLayout>,
    /// The other open documents the call names, each at the version pinned for the call.
    pub others: crate::Others<'a>,
    /// The session's blobs, where the host hands them over (`word_drawings`' pictures);
    /// `None`: a call that reads one is left to the host.
    pub blobs: Option<&'a dyn crate::Blobs>,
    /// The values the host wrote into the call's arguments for references (`{{d:...}}`),
    /// which the call keeps on the paragraphs they went into; empty: none.
    pub written: &'a [crate::Written],
    /// The host's record of the rows the session's `word_edit_table` calls wrote (round 58,
    /// R126), read and added to; `None`: the call's own rows only, none kept.
    pub rows_written: Option<&'a crate::RowsWritten>,
}

impl<'a> Editing<'a> {
    /// What every call is handed: the editor, the caller, the document as opened, its id,
    /// the cancellation and the call's time. The host's optional context -- tracking, a
    /// layout, other documents, blobs, values written by reference -- starts empty and is
    /// given with the `with_*` setters, so a new kind of context adds a setter, not a
    /// field to every caller's literal.
    #[must_use]
    pub fn new(
        editor: &'a CallEditor<'a>,
        caller: &'a Caller,
        opened: &'a WordDoc,
        doc: &'a str,
        cancel: &'a dyn Cancel,
        date: &'a str,
    ) -> Self {
        Self {
            editor,
            caller,
            opened,
            tracking: None,
            doc,
            cancel,
            date,
            layout: None,
            others: crate::Others::default(),
            blobs: None,
            written: &[],
            rows_written: None,
        }
    }

    /// The tracking mode the host keeps for the document (`None`: off).
    #[must_use]
    pub fn with_tracking(mut self, tracking: Option<&'a Tracking>) -> Self {
        self.tracking = tracking;
        self
    }

    /// The layout of the document's versions, where the host keeps one.
    #[must_use]
    pub fn with_layout(mut self, layout: Option<&'a dyn FieldLayout>) -> Self {
        self.layout = layout;
        self
    }

    /// The other open documents the call names.
    #[must_use]
    pub fn with_others(mut self, others: crate::Others<'a>) -> Self {
        self.others = others;
        self
    }

    /// The session's blobs, where the host hands them over.
    #[must_use]
    pub fn with_blobs(mut self, blobs: Option<&'a dyn crate::Blobs>) -> Self {
        self.blobs = blobs;
        self
    }

    /// The values the host wrote into the call's arguments for references.
    #[must_use]
    pub fn with_written(mut self, written: &'a [crate::Written]) -> Self {
        self.written = written;
        self
    }

    /// The host's record of the rows written ([`crate::RowsWritten`]).
    #[must_use]
    pub fn with_rows_written(mut self, rows_written: Option<&'a crate::RowsWritten>) -> Self {
        self.rows_written = rows_written;
        self
    }
}

/// An edit's refusal as the tool's: an address refused with its way out, a refusal with
/// the call that would work, anything else internal.
pub(crate) fn edit_refusal(error: EditError, field: &str, call: TryCall) -> ToolError {
    match error {
        EditError::Address(e) => crate::refusal::address_refusal(e, field, call),
        // a table position as accepted names its own argument
        EditError::Position(m) => crate::refusal::address_refusal(m.error, &m.field, call),
        EditError::Refused(m) | EditError::Text(m) | EditError::Span(m) => {
            ToolError::refusal(Refusal::Refused, m, WayOut::call(call)).at(field)
        }
        EditError::Argument {
            field,
            message,
            options,
        } => {
            let way_out = match WayOut::names(options) {
                Some(names) => names.and_call(call),
                None => WayOut::call(call),
            };
            ToolError::refusal(Refusal::InvalidArguments, message, way_out).at(field)
        }
        other => ToolError::internal(other.to_string(), "word:edit"),
    }
}

/// An argument a call needs and lacks (`need`), with the call that would work.
pub(crate) fn needed(name: &str, why: &str, call: TryCall) -> ToolError {
    ToolError::refusal(
        Refusal::InvalidArguments,
        format!("{name} is required{why}"),
        WayOut::call(call),
    )
    .at(name)
}

/// A call stopped before it changed anything.
pub(crate) fn stopped(cancel: &dyn Cancel) -> Result<(), ToolError> {
    cancel.check().map_err(|_| {
        ToolError::failure(
            Failure::Timeout,
            "the call stopped before it finished (it was cancelled or ran past its deadline); nothing changed",
        )
    })
}

/// Registers the group's tools, in the order a model is shown them (the crate's
/// [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    tracking::register(r);
    changes::register(r);
    comments::register(r);
}
