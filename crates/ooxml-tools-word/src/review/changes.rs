//! Owns: `word_changes`' definition and its dispatch by action (docx-agent's
//! `word_changes`): `list` reads ([`super::listing`]), `accept`, `reject` and `preview`
//! settle ([`super::settle`]); the refusals of arguments an action does not take.
//!
//! Must not: settle or list itself.

use super::Editing;
use ooxml_tools::{ArgEnum, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
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

/// `word_changes` on the document `editing` names; a failed reply is
/// `invalid_arguments` for arguments the action does not take, a selection missing or
/// both by ids and by filters; `not_found` for ids the document has no change or record
/// of (the listed ids the way out); the address resolver's refusal of `within`;
/// `timeout` when cancelled.
#[must_use]
pub fn word_changes(args: &WordChanges, editing: &Editing<'_>) -> ToolResult<Value> {
    run(args, editing).unwrap_or_else(ToolResult::failed)
}

fn run(args: &WordChanges, editing: &Editing<'_>) -> Result<ToolResult<Value>, ToolError> {
    super::stopped(editing.cancel)?;
    let given = args.ids.as_ref().is_some_and(|i| !i.is_empty());
    if args.action == ChangeAction::List {
        if given || args.all == Some(true) {
            let field = if given { "ids" } else { "all" };
            let mut call = args.clone();
            call.ids = None;
            call.all = None;
            return Err(invalid("list filters by author, kind and within", field, &call));
        }
        return super::listing::list(args, editing).map(ToolResult::done);
    }
    let detail = args.detail.unwrap_or(ChangeDetail::Grouped);
    if detail != ChangeDetail::Grouped || args.cursor.is_some() {
        let field = if detail == ChangeDetail::Grouped {
            "cursor"
        } else {
            "detail"
        };
        let mut call = args.clone();
        call.detail = None;
        call.cursor = None;
        return Err(invalid("detail and cursor are for list", field, &call));
    }
    super::settle::settle(args, editing)
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
    r.edit::<WordChanges>(word_changes, crate::Hosting::SERVED);
}
