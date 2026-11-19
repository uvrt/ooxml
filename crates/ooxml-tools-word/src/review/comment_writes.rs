//! Owns: `word_comments`' `add` and `reply` (docx-agent's `word_comments`): each item's
//! arguments checked as docx-agent checks them, the text found once where `find` is given,
//! and every item written in one [`AddComments`] command (one history step, all or none);
//! the reply as docx-agent's `outcome` writes it, with the new comments (`data.comments`)
//! and the refs naming them. Text found `within` a block or span is not ported yet: the
//! host answers it its own way (`unported`).
//!
//! Must not: change a comment but through a command.

use super::comments::{try_call, CommentAction, CommentItem, WordComments};
use super::{edit_refusal, Editing};
use crate::structure::common::{find_once, one_of, outcome, Outcome};
use ooxml_tools::{Done, ToolError, ToolResult};
use ooxml_word_edit::{AddComments, CommentMade, CommentWrite, Who};
use serde_json::{json, Value};

/// What `unported` says of text found within a block or span.
const NOT_YET: &str = "a comment on text found within a block or span";

/// `add` or `reply` of `items`.
pub(crate) fn write(
    args: &WordComments,
    editing: &Editing<'_>,
    items: &[CommentItem],
) -> Result<ToolResult<Value>, ToolError> {
    let mut list = args.clone();
    list.action = CommentAction::List;
    list.items = None;
    let writes = match planned(args, editing, items, &list)? {
        Ok(writes) => writes,
        Err(unported) => return Ok(unported),
    };
    // docx-agent's `author_of`: the one given, else the tracking author
    let author = args
        .author
        .as_deref()
        .filter(|a| !a.is_empty())
        .or_else(|| editing.tracking.map(|t| t.author.as_str()));
    let command = AddComments {
        items: writes,
        who: Who::new(author, editing.date),
    };
    let done = editing
        .editor
        .run(editing.caller, &command)
        .map_err(|e| edit_refusal(e, "", try_call(&list)))?;
    Ok(reply(args, items, done.out))
}

/// Each item's write, its arguments checked as docx-agent checks them; `Err`: the reply
/// leaving the call to the host.
fn planned(
    args: &WordComments,
    editing: &Editing<'_>,
    items: &[CommentItem],
    list: &WordComments,
) -> Result<Result<Vec<CommentWrite>, ToolResult<Value>>, ToolError> {
    let call = try_call(args);
    let mut writes = Vec::with_capacity(items.len());
    for item in items {
        if args.action == CommentAction::Add {
            let text = item
                .text
                .as_ref()
                .ok_or_else(|| super::needed("items[].text", "", call.clone()))?;
            let found = one_of(
                &[("find", item.find.is_some()), ("target", item.target.is_some())],
                &call,
            )? == "find";
            if found && item.within.is_some() {
                let mut done = Done::new(String::new());
                done.unported.push(NOT_YET.to_owned());
                return Ok(Err(ToolResult::done(done)));
            }
            let place = match (&item.find, &item.target) {
                (Some(text), _) if found => find_once(editing, text, &call)?,
                (_, Some(target)) => target.clone(),
                _ => String::new(),
            };
            writes.push(CommentWrite::Add {
                place,
                text: text.clone(),
            });
        } else {
            let comment = item
                .comment
                .as_ref()
                .ok_or_else(|| super::needed("items[].comment", "", try_call(list)))?;
            let text = item
                .text
                .as_ref()
                .ok_or_else(|| super::needed("items[].text", "", try_call(list)))?;
            writes.push(CommentWrite::Reply {
                comment: comment.clone(),
                text: text.clone(),
                resolve: item.resolve == Some(true),
            });
        }
    }
    Ok(Ok(writes))
}

/// The reply, as docx-agent's `outcome` writes it of the comments made.
fn reply(args: &WordComments, items: &[CommentItem], out: Vec<CommentMade>) -> ToolResult<Value> {
    let mut edits = Vec::new();
    let mut made = Vec::new();
    let mut refs = Vec::new();
    for (item, out) in items.iter().zip(out) {
        made.push(out.id.clone());
        if let Some(name) = item.ref_name.as_ref().filter(|n| !n.is_empty()) {
            refs.push((name.clone(), out.id.clone()));
        }
        edits.push(Outcome {
            id: Some(out.id.clone()),
            changed: true,
            created: vec![out.id],
            renamed: out.renamed,
            ..Outcome::default()
        });
        if let Some(resolved) = out.resolved {
            edits.push(Outcome {
                id: resolved.id,
                changed: resolved.changed,
                ..Outcome::default()
            });
        }
    }
    let data = (!made.is_empty()).then(|| json!({"comments": made}));
    let mut result = outcome(
        &edits,
        format!("{}: {} comment(s)", args.action.name(), items.len()),
        data,
    );
    if let ooxml_tools::Outcome::Done(reply) = &mut result.outcome {
        reply.refs = refs;
    }
    result
}
