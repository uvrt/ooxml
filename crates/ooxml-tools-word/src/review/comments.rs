//! Owns: `word_comments`' definition and its `list` (docx-agent's `word_comments`,
//! `comment_json`): each thread's first comment with what it is attached to and its
//! replies, unresolved ones only when asked, a page at a time; the edit actions run
//! through the edit core's comment commands ([`super::comment_edits`]).
//!
//! Must not: change a comment but through a command.

use super::Editing;
use crate::describe::short;
use crate::paging::page_list;
use crate::word_inspect::comments_with_anchors;
use ooxml_tools::{ArgEnum, Done, ToolArgs, ToolError, ToolResult, TryCall};
use serde_json::{json, Map, Value};

/// What `word_comments` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentAction {
    /// List the threads.
    List,
    /// Add comments.
    Add,
    /// Reply in threads.
    Reply,
    /// Resolve threads.
    Resolve,
    /// Reopen threads.
    Reopen,
    /// Rewrite comments.
    Edit,
    /// Delete comments with their replies.
    Delete,
}

impl CommentAction {
    /// Its name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            CommentAction::List => "list",
            CommentAction::Add => "add",
            CommentAction::Reply => "reply",
            CommentAction::Resolve => "resolve",
            CommentAction::Reopen => "reopen",
            CommentAction::Edit => "edit",
            CommentAction::Delete => "delete",
        }
    }
}

/// One comment to act on.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
pub struct CommentItem {
    /// add: range, paragraph, table, row or cc:.
    #[arg(refs)]
    pub target: Option<String>,
    /// add: or text occurring once.
    pub find: Option<String>,
    /// add: block or span to search for find.
    #[arg(refs)]
    pub within: Option<String>,
    /// The comment (c:...), except for add.
    #[arg(refs)]
    pub comment: Option<String>,
    /// add, reply, edit: the text.
    pub text: Option<String>,
    /// reply: also resolve the thread.
    pub resolve: Option<bool>,
    /// add, reply: ref name for the new comment.
    #[arg(rename = "ref")]
    pub ref_name: Option<String>,
}

/// `word_comments`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(name = "word_comments", description = include_str!("word_comments.md"), group = "word_review", mutates)]
pub struct WordComments {
    /// Document id.
    pub doc: String,
    /// What to do.
    pub action: CommentAction,
    /// The comments to act on (not list).
    pub items: Option<Vec<CommentItem>>,
    /// add, reply: author. Default the tracking author.
    pub author: Option<String>,
    /// list: only unresolved threads.
    pub open_only: Option<bool>,
    /// next_cursor of the previous page.
    pub cursor: Option<String>,
}

/// `word_comments` on the document `editing` names; a failed reply is its refusal.
#[must_use]
pub fn word_comments(args: &WordComments, editing: &Editing<'_>) -> ToolResult<Value> {
    run(args, editing).unwrap_or_else(ToolResult::failed)
}

fn run(args: &WordComments, editing: &Editing<'_>) -> Result<ToolResult<Value>, ToolError> {
    super::stopped(editing.cancel)?;
    if args.action == CommentAction::List {
        return list(args, editing).map(ToolResult::done);
    }
    super::comment_edits::act(args, editing)
}

/// `list`: each thread's first comment.
pub(super) fn list(args: &WordComments, editing: &Editing<'_>) -> Result<Done<Value>, ToolError> {
    let snapshot = editing.editor.store().snapshot();
    let docx = snapshot
        .doc()
        .docx()
        .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
    let all = comments_with_anchors(&docx);
    let open_only = args.open_only == Some(true);
    let items: Vec<Value> = all
        .iter()
        .filter(|(c, _)| c.parent.is_none() && !(open_only && c.done))
        .map(|(c, anchor)| {
            let mut data = Map::new();
            data.insert("id".into(), json!(c.id));
            data.insert("author".into(), json!(c.author));
            data.insert("done".into(), json!(c.done));
            data.insert("text".into(), json!(short(&c.says, 300)));
            if let Value::Object(on) = anchor {
                let id = on.get("id").cloned().unwrap_or(Value::Null);
                let text = on.get("t").and_then(Value::as_str).unwrap_or("");
                data.insert("on".into(), json!({"id": id, "text": short(text, 120)}));
            }
            let replies: Vec<Value> = all
                .iter()
                .filter(|(r, _)| r.parent.as_deref() == Some(c.id.as_str()))
                .map(|(r, _)| json!({"id": r.id, "author": r.author, "text": short(&r.says, 200)}))
                .collect();
            if !replies.is_empty() {
                data.insert("replies".into(), Value::Array(replies));
            }
            Value::Object(data)
        })
        .collect();
    let mut first = args.clone();
    first.cursor = None;
    let (shown, total, next) = page_list(
        &items,
        args.cursor.as_deref(),
        editing.limits.list_items,
        try_call(&first),
    )?;
    let mut done = Done::new(format!("{total} thread(s)"));
    done.data = Some(Value::Array(shown));
    done.next_cursor = next;
    done.total = Some(total as u64);
    Ok(done)
}

/// `word_comments` called with `args`.
pub(crate) fn try_call(args: &WordComments) -> TryCall {
    let mut arguments = Map::new();
    arguments.insert("doc".into(), Value::from(args.doc.as_str()));
    arguments.insert("action".into(), Value::from(args.action.name()));
    if let Some(author) = &args.author {
        arguments.insert("author".into(), Value::from(author.as_str()));
    }
    if let Some(open) = args.open_only {
        arguments.insert("open_only".into(), Value::from(open));
    }
    if let Some(cursor) = &args.cursor {
        arguments.insert("cursor".into(), Value::from(cursor.as_str()));
    }
    TryCall {
        tool: "word_comments".to_owned(),
        arguments,
        fill: Vec::new(),
    }
}

/// Registers `word_comments` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<WordComments>(word_comments, crate::Hosting::SERVED);
}
