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
pub use changes::{word_changes, word_changes_read, ChangeAction, ChangeDetail, ChangeKind, WordChanges};
pub use comments::{word_comments, CommentAction, CommentItem, WordComments};
pub(crate) use settle::Still;
pub use tracking::{word_set_tracking, Tracked, WordSetTracking};

use ooxml_base::Cancel;
use ooxml_store::Caller;
use ooxml_tools::{Failure, Refusal, ToolError, TryCall, WayOut};
use ooxml_word_edit::{EditError, EditLayout, FieldLayout, Tracking, WordDoc};

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
    /// How long laying a version out may take in the call, where the host limits it
    /// (`None`: only the call's cancellation stops it).
    pub layout_limit: Option<crate::LayoutLimit<'a>>,
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
    /// The locale the call reads its document under where the host names one (the host's
    /// document locale): a table of contents' style names and the separator between them;
    /// `None`: the document's own language.
    pub locale: Option<&'a ooxml_numfmt::Locale>,
    /// The host's limits on the call's reply (a listing's items per page).
    pub limits: ooxml_tools::Limits,
    /// What the document's last check that laid it out saw, where the host keeps it
    /// (`word_format`'s keep measured against the version before its change); `None`:
    /// nothing is measured.
    pub last_check: Option<&'a crate::LastCheck>,
    /// Whether the host's batch holds the document in a step group: what an op would lay
    /// the whole document out for (a table kept together's pages) waits for the host, which
    /// measures it once after the batch's last op.
    pub in_group: bool,
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
            layout_limit: None,
            others: crate::Others::default(),
            blobs: None,
            written: &[],
            rows_written: None,
            locale: None,
            limits: ooxml_tools::Limits::DEFAULT,
            last_check: None,
            in_group: false,
        }
    }

    /// Whether the host's batch holds the document in a step group.
    #[must_use]
    pub fn with_in_group(mut self, in_group: bool) -> Self {
        self.in_group = in_group;
        self
    }

    /// What the document's last check that laid it out saw, where the host keeps it.
    #[must_use]
    pub fn with_last_check(mut self, last_check: Option<&'a crate::LastCheck>) -> Self {
        self.last_check = last_check;
        self
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

    /// How long laying a version out may take in the call.
    #[must_use]
    pub fn with_layout_limit(mut self, limit: Option<crate::LayoutLimit<'a>>) -> Self {
        self.layout_limit = limit;
        self
    }

    /// The layout an edit reads, where the host keeps one: stopped by the call's limit on
    /// laying out where it has one, else by its cancellation.
    #[must_use]
    pub fn edit_layout(&self) -> Option<EditLayout<'a>> {
        let layout = self.layout?;
        Some(match self.layout_limit {
            Some(limit) => EditLayout {
                layout,
                stop: limit.cancel,
                limit: Some(limit.timeout),
            },
            None => EditLayout {
                layout,
                stop: self.cancel,
                limit: None,
            },
        })
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

    /// The host's limits on the call's reply.
    #[must_use]
    pub fn with_limits(mut self, limits: ooxml_tools::Limits) -> Self {
        self.limits = limits;
        self
    }

    /// The locale the call reads its document under, where the host names one.
    #[must_use]
    pub fn with_locale(mut self, locale: Option<&'a ooxml_numfmt::Locale>) -> Self {
        self.locale = locale;
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
        EditError::LayoutStopped(limit) => laying_out_stopped(limit),
        // a form the edit core does not support yet: refused as such, the call as the way out
        EditError::Unsupported(what) => ToolError::not_supported(&what, WayOut::call(call)).at(field),
        other => ToolError::internal(other.to_string(), "word:edit"),
    }
}

/// An edit stopped while laying a version out (docx-agent's deadline: `laying the document
/// out did not finish within <limit> s`, with the limit in `details`): nothing changed.
fn laying_out_stopped(limit: Option<std::time::Duration>) -> ToolError {
    let Some(limit) = limit else {
        return ToolError::failure(
            Failure::Timeout,
            "laying the document out was stopped before it finished (the call was cancelled); nothing changed",
        );
    };
    let seconds = limit.as_secs_f64();
    let message = format!(
        "laying the document out did not finish within {} s",
        crate::table_read::python_g(seconds)
    );
    ToolError::failure(Failure::Timeout, message).with_detail("timeout", serde_json::Value::from(seconds))
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
