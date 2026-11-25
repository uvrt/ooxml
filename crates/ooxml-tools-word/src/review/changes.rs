//! Owns: `word_changes`' definition and its dispatch by action (docx-agent's
//! `word_changes`): `list` ([`super::listing`]) and `preview` only read -- in a session on
//! the version the call pins ([`word_changes_read`]), outside one on the editor's head --
//! `accept` and `reject` settle ([`super::settle`]); [`Reading`], what the reading actions
//! read; the refusals of arguments an action does not take.
//!
//! Must not: settle or list itself.

use super::Editing;
use crate::Version;
use ooxml_base::Cancel;
use ooxml_tools::{ArgEnum, Done, Limits, ReadsWhen, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::WordDoc;
use serde_json::{Map, Value};

/// What `word_changes` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeAction {
    /// List the changes; nothing changes.
    List,
    /// Accept them.
    Accept,
    /// Reject them.
    Reject,
    /// Say what accepting and rejecting them would do; nothing changes.
    Preview,
}

/// A logical change's kind.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Inserted.
    Insertion,
    /// Deleted.
    Deletion,
    /// Replaced: deleted and inserted at once.
    Replacement,
    /// Moved.
    Move,
    /// Formatting changed.
    Formatting,
    /// A table's structure or properties.
    Table,
    /// A section's.
    Section,
}

impl ChangeKind {
    /// Its name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            ChangeKind::Insertion => "insertion",
            ChangeKind::Deletion => "deletion",
            ChangeKind::Replacement => "replacement",
            ChangeKind::Move => "move",
            ChangeKind::Formatting => "formatting",
            ChangeKind::Table => "table",
            ChangeKind::Section => "section",
        }
    }
}

/// How `list` lists.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeDetail {
    /// As a reviewer reads them: a replacement is one change.
    Grouped,
    /// As Word's revision records.
    Records,
}

/// `word_changes`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(name = "word_changes", description = include_str!("word_changes.md"), group = "word_review", mutates)]
pub struct WordChanges {
    /// Document id.
    pub doc: String,
    /// What to do; list changes nothing.
    pub action: ChangeAction,
    /// Not list: these changes (rev: ids).
    #[arg(refs)]
    pub ids: Option<Vec<String>>,
    /// Only this author's.
    pub author: Option<String>,
    /// Only this kind.
    pub kind: Option<ChangeKind>,
    /// Only in this block or span.
    #[arg(refs)]
    pub within: Option<String>,
    /// Not list: every change.
    pub all: Option<bool>,
    /// list. Default grouped.
    pub detail: Option<ChangeDetail>,
    /// next_cursor of the previous page.
    pub cursor: Option<String>,
}

impl ReadsWhen for WordChanges {
    /// `list` and `preview` change nothing.
    fn reads(&self) -> bool {
        matches!(self.action, ChangeAction::List | ChangeAction::Preview)
    }
}

/// What `list` and `preview` read: the document at the version read and as the session
/// opened it (the records its own edits wrote are told from that), its id for the calls a
/// reply offers, the host's limits and the call's cancellation.
pub(crate) struct Reading<'a> {
    /// The document at the version read.
    pub(crate) doc: &'a WordDoc,
    /// The document as the session opened it.
    pub(crate) opened: &'a WordDoc,
    /// The document's id, for the calls a reply offers.
    pub(crate) name: &'a str,
    /// The host's limits on the reply (a listing's items per page).
    pub(crate) limits: Limits,
    /// The call's cancellation.
    pub(crate) cancel: &'a dyn Cancel,
}

/// `word_changes` on the document `editing` names (`list` and `preview` on the editor's
/// head); a failed reply is `invalid_arguments` for arguments the action does not take, a
/// selection missing or both by ids and by filters; `not_found` for ids the document has
/// no change or record of (the listed ids the way out); the address resolver's refusal of
/// `within`; `timeout` when cancelled.
#[must_use]
pub fn word_changes(args: &WordChanges, editing: &Editing<'_>) -> ToolResult<Value> {
    run(args, editing).unwrap_or_else(ToolResult::failed)
}

/// `word_changes`' `list` and `preview` on the version a session's call pins (the
/// session's [`ReadsWhen`] calls), refused as [`word_changes`] refuses them, and as not
/// supported yet on a package the session does not own as a Word document.
///
/// # Errors
/// As [`word_changes`]'.
pub fn word_changes_read(
    args: &WordChanges,
    version: &Version<'_>,
    cancel: &dyn Cancel,
) -> Result<Done<Value>, ToolError> {
    super::stopped(cancel)?;
    let Some(owned) = version.owned else {
        return Err(crate::refusal::not_supported(
            "word_changes of a document the session does not own",
            "word_changes",
            serde_json::json!({"doc": args.doc, "action": "list"}),
            &["doc"],
        ));
    };
    let reading = Reading {
        doc: owned.doc,
        opened: owned.opened,
        name: &args.doc,
        limits: version.limits,
        cancel,
    };
    read(args, &reading)
}

fn run(args: &WordChanges, editing: &Editing<'_>) -> Result<ToolResult<Value>, ToolError> {
    super::stopped(editing.cancel)?;
    if args.reads() {
        let head = editing.editor.current();
        let reading = Reading {
            doc: head.doc(),
            opened: editing.opened,
            name: editing.doc,
            limits: editing.limits,
            cancel: editing.cancel,
        };
        return read(args, &reading).map(ToolResult::done);
    }
    only_list_fields(args)?;
    super::settle::settle(args, editing)
}

/// `list` or `preview`.
fn read(args: &WordChanges, reading: &Reading<'_>) -> Result<Done<Value>, ToolError> {
    if args.action == ChangeAction::List {
        let given = args.ids.as_ref().is_some_and(|i| !i.is_empty());
        if given || args.all == Some(true) {
            let field = if given { "ids" } else { "all" };
            let mut call = args.clone();
            call.ids = None;
            call.all = None;
            return Err(invalid("list filters by author, kind and within", field, &call));
        }
        return super::listing::list(args, reading);
    }
    only_list_fields(args)?;
    super::settle::preview(args, reading)
}

/// The refusal of `detail` and `cursor` given to an action but `list`.
fn only_list_fields(args: &WordChanges) -> Result<(), ToolError> {
    let detail = args.detail.unwrap_or(ChangeDetail::Grouped);
    if detail == ChangeDetail::Grouped && args.cursor.is_none() {
        return Ok(());
    }
    let field = if detail == ChangeDetail::Grouped {
        "cursor"
    } else {
        "detail"
    };
    let mut call = args.clone();
    call.detail = None;
    call.cursor = None;
    Err(invalid("detail and cursor are for list", field, &call))
}

/// `invalid_arguments` at `field`, `call` the call that would work.
pub(crate) fn invalid(message: &str, field: &str, call: &WordChanges) -> ToolError {
    ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(try_call(call))).at(field)
}

/// `word_changes` called with `args`.
pub(crate) fn try_call(args: &WordChanges) -> TryCall {
    let mut arguments = Map::new();
    arguments.insert("doc".into(), Value::from(args.doc.as_str()));
    let action = match args.action {
        ChangeAction::List => "list",
        ChangeAction::Accept => "accept",
        ChangeAction::Reject => "reject",
        ChangeAction::Preview => "preview",
    };
    arguments.insert("action".into(), Value::from(action));
    if let Some(ids) = &args.ids {
        arguments.insert("ids".into(), Value::from(ids.clone()));
    }
    for (key, value) in [
        ("author", &args.author),
        ("within", &args.within),
        ("cursor", &args.cursor),
    ] {
        if let Some(v) = value {
            arguments.insert(key.into(), Value::from(v.as_str()));
        }
    }
    if let Some(kind) = args.kind {
        arguments.insert("kind".into(), Value::from(kind.name()));
    }
    if let Some(all) = args.all {
        arguments.insert("all".into(), Value::from(all));
    }
    if args.detail == Some(ChangeDetail::Records) {
        arguments.insert("detail".into(), Value::from("records"));
    }
    TryCall {
        tool: "word_changes".to_owned(),
        arguments,
        fill: Vec::new(),
    }
}

/// Registers `word_changes` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.reading_edit::<WordChanges>(word_changes_read, word_changes, crate::Hosting::PLAIN);
}
