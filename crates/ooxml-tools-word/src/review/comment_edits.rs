//! Owns: `word_comments`' edit actions (docx-agent's `word_comments`): `resolve`, `reopen`
//! and `delete` run as one [`CommentEdits`] command, `edit` as one [`EditCommentText`];
//! `add` and `reply` are not ported yet (they write the reference's size and the modern
//! comment parts' entries), and say so in `unported` for the host to answer its own way.
//!
//! Must not: change a comment but through a command.

use super::comments::{try_call, CommentAction, WordComments};
use super::Editing;
use ooxml_tools::{Done, Refusal, ToolError, ToolResult, WayOut};
use ooxml_word_edit::{CommentEdit, CommentEdits, EditCommentText};
use serde_json::Value;

/// What `unported` says of a comment's `add`, `reply` or `edit`.
pub const COMMENT_WRITES: &str = "comment add and reply (the reference's size and the comment parts' entries)";

/// An edit action.
pub(crate) fn act(args: &WordComments, editing: &Editing<'_>) -> Result<ToolResult<Value>, ToolError> {
    let action = args.action.name();
    let Some(items) = &args.items else {
        let mut call = args.clone();
        call.action = CommentAction::List;
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            format!("{action} needs items"),
            WayOut::call(try_call(&call)),
        )
        .at("items"));
    };
    if items.is_empty() {
        return Ok(ToolResult::nothing_given("changed", "items"));
    }
    if args.action == CommentAction::Edit {
        return edit_text(args, editing, items);
    }
    let edit = match args.action {
        CommentAction::Resolve => CommentEdit::Resolve,
        CommentAction::Reopen => CommentEdit::Reopen,
        CommentAction::Delete => CommentEdit::Delete,
        _ => {
            let mut done = Done::new(String::new());
            done.unported.push(COMMENT_WRITES.to_owned());
            return Ok(ToolResult::done(done));
        }
    };
    let mut ids = Vec::new();
    for item in items {
        match &item.comment {
            Some(id) => ids.push(id.clone()),
            None => {
                let mut call = args.clone();
                call.action = CommentAction::List;
                call.items = None;
                return Err(super::needed("items[].comment", "", try_call(&call)));
            }
        }
    }
    let mut list = args.clone();
    list.action = CommentAction::List;
    list.items = None;
    let done = editing
        .editor
        .run(editing.caller, &CommentEdits { edit, ids })
        .map_err(|e| super::edit_refusal(e, "", try_call(&list)))?;
    let mut reply = Done::new(format!("{action}: {} comment(s)", items.len()));
    for outcome in &done.out {
        if let Some(id) = outcome.id.as_ref().filter(|_| outcome.changed) {
            if !reply.changed.contains(id) {
                reply.changed.push(id.clone());
            }
        }
        for gone in &outcome.removed {
            if !reply.removed.contains(gone) {
                reply.removed.push(gone.clone());
            }
        }
    }
    Ok(ToolResult::done(reply))
}

/// `edit`: each item's comment given its text, in one command.
fn edit_text(
    args: &WordComments,
    editing: &Editing<'_>,
    items: &[super::CommentItem],
) -> Result<ToolResult<Value>, ToolError> {
    let mut list = args.clone();
    list.action = CommentAction::List;
    list.items = None;
    let mut edits = Vec::new();
    for item in items {
        let Some(id) = &item.comment else {
            return Err(super::needed("items[].comment", "", try_call(&list)));
        };
        let Some(text) = &item.text else {
            return Err(super::needed("items[].text", "", try_call(&list)));
        };
        edits.push((id.clone(), text.clone()));
    }
    let done = editing
        .editor
        .run(editing.caller, &EditCommentText { edits })
        .map_err(|e| super::edit_refusal(e, "", try_call(&list)))?;
    let mut reply = Done::new(format!("edit: {} comment(s)", items.len()));
    for outcome in &done.out {
        if let Some(id) = outcome.id.as_ref() {
            if !reply.changed.contains(id) {
                reply.changed.push(id.clone());
            }
        }
    }
    Ok(ToolResult::done(reply))
}
