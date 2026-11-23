//! Owns: `word_delete` (docx-agent's `word_delete`): text found once (in a scope, `within`
//! or the target) or a range's ([`DeletePlaced`], with the space it would leave doubled when asked), a note
//! ([`DeleteNote`]), or whole blocks -- a block, a span, a heading's range -- with the outline
//! checked first ([`DeleteBlocks`]) and the file's original text they took out of the final
//! view reported (`data.original_removed`, round 57).
//!
//! Must not: write the document but through the commands, or answer a call it does not
//! support yet, refused ([`Unported`]).

use super::{by_reference, not_supported, Unported};
use crate::describe::short;
use crate::originals::{add, Watch};
use crate::review::edit_refusal;
use crate::structure::common::{outcome, Outcome};
use crate::Editing;
use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{blocks_named, DeleteBlocks, DeleteNote, DeletePlaced, EditError, TextPlace};
use ooxml_word_model::address::expand_heading;
use serde_json::{Map, Value};

/// `word_delete`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_delete",
    description = include_str!("word_delete.md"),
    group = "word_text",
    mutates,
    paths_first("target")
)]
pub struct WordDelete {
    /// Document id.
    pub doc: String,
    /// Text occurring once (in within, if given).
    pub find: Option<String>,
    /// Block or span to search for find.
    #[arg(refs)]
    pub within: Option<String>,
    /// Range, block (a control cc: too), span p:A..t:B, picture d:, note fn:/en:.
    #[arg(refs)]
    pub target: Option<String>,
    /// Also delete the space the deletion would leave doubled. Default false.
    pub collapse_space: Option<bool>,
    /// Delete a heading though its content stays. Default false.
    pub force: Option<bool>,
}

/// `word_delete` on the document `editing` names. A find with a target searches within it
/// (said in a warning). A failed reply is `invalid_arguments`
/// for a call giving neither find nor target, or a within other than its target, and for
/// a heading whose content would stay; the find's, range's or block's refusal; `refused`
/// (nothing changed) for what is not supported yet.
#[must_use]
pub fn word_delete(args: &WordDelete, editing: &Editing<'_>) -> ToolResult<Value> {
    run(args, editing).unwrap_or_else(ToolResult::failed)
}

fn run(args: &WordDelete, editing: &Editing<'_>) -> Result<ToolResult<Value>, ToolError> {
    crate::review::stopped(editing.cancel)?;
    let call = try_call(args);
    if args.find.is_none() && args.target.is_none() {
        let way =
            WayOut::names(["find", "target"]).map_or_else(|| WayOut::call(call.clone()), |w| w.and_call(call.clone()));
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            "give find (text to delete) or target (what to delete)",
            way,
        )
        .at("find"));
    }
    if let Some(refusal) = not_supported_form(args, &call) {
        return Err(refusal);
    }
    let policy = editing.policy();
    let collapse_space = args.collapse_space.unwrap_or(false);
    let refuse = |e| edit_refusal(e, "", call.clone());
    if let Some(find) = &args.find {
        let (within, note) = searched(args, editing, &call)?;
        let command = DeletePlaced {
            place: TextPlace::Once {
                text: find.clone(),
                within,
            },
            collapse_space,
            policy,
        };
        let done = editing.editor.run(editing.caller, &command).map_err(refuse)?;
        let summary = format!("Deleted {}", repr_str(&short(&done.out.text, 60)));
        let mut result = outcome(&[placed(done.out.edited)], summary, None);
        result.warnings.extend(note);
        return Ok(result);
    }
    let target = args.target.clone().unwrap_or_default();
    if target.contains('@') && !target.starts_with("p@") && !target.starts_with("cc@") {
        let command = DeletePlaced {
            place: TextPlace::Range(target.clone()),
            collapse_space,
            policy,
        };
        let done = editing.editor.run(editing.caller, &command).map_err(refuse)?;
        return Ok(outcome(&[placed(done.out.edited)], format!("Deleted {target}"), None));
    }
    if target.starts_with("fn:") || target.starts_with("en:") {
        let command = DeleteNote {
            note: target.clone(),
            policy,
        };
        let done = editing.editor.run(editing.caller, &command).map_err(refuse)?;
        return Ok(outcome(&[done.out.into()], format!("Deleted note {target}"), None));
    }
    blocks(args, editing, &target, &call)
}

/// What of the call is not supported yet, refused: a picture, a heading's range without its
/// end, an address by reference.
fn not_supported_form(args: &WordDelete, call: &TryCall) -> Option<ToolError> {
    let target = args.target.as_deref().unwrap_or_default();
    if args.target.as_deref().is_some_and(by_reference) || args.within.as_deref().is_some_and(by_reference) {
        return Some(Unported::Reference.refusal(call.clone(), &["target", "within"]));
    }
    if args.find.is_none() && target.starts_with("d:") {
        return Some(not_supported("word_delete a picture (d:)", call.clone(), &["target"]));
    }
    if args.find.is_none() && target.starts_with("h:") && !target.contains("..") {
        return Some(not_supported(
            "word_delete a heading without its end",
            call.clone(),
            &["target"],
        ));
    }
    None
}

/// Where the find searches, and the note saying so: `within`, or the target given with it
/// (docx-agent's round 31, R5a: the call meant find within it). Both given, they must name
/// the same blocks (a heading's range as its span).
fn searched(
    args: &WordDelete,
    editing: &Editing<'_>,
    call: &TryCall,
) -> Result<(Option<String>, Option<String>), ToolError> {
    let Some(target) = &args.target else {
        return Ok((args.within.clone(), None));
    };
    if let Some(within) = &args.within {
        if !same_scope(editing, within, target, call)? {
            let way = WayOut::names(["within", "target"])
                .map_or_else(|| WayOut::call(call.clone()), |w| w.and_call(call.clone()));
            return Err(ToolError::refusal(
                Refusal::InvalidArguments,
                "find searches within one block or span: give within or target with it, not both",
                way,
            )
            .at("target"));
        }
    }
    let note = format!("find was searched within target {target}: within names where to search");
    Ok((Some(target.clone()), Some(note)))
}

/// Whether `within` and `target` name the same blocks on the current version, a heading's
/// range taken as the span it is (docx-agent's `resolve` of each).
fn same_scope(editing: &Editing<'_>, within: &str, target: &str, call: &TryCall) -> Result<bool, ToolError> {
    if within == target {
        return Ok(true);
    }
    let head = editing.editor.store().snapshot();
    let refuse = |e| edit_refusal(e, "", call.clone());
    let docx = head.doc().docx().map_err(refuse)?;
    let expand = |address: &str| expand_heading(&docx, address).map_err(|e| refuse(EditError::Address(e)));
    Ok(expand(within)? == expand(target)?)
}

/// Whole blocks deleted, the file's original text they held watched.
fn blocks(
    args: &WordDelete,
    editing: &Editing<'_>,
    target: &str,
    call: &TryCall,
) -> Result<ToolResult<Value>, ToolError> {
    let before = editing.editor.store().snapshot();
    let mut watch = watch(editing, before.doc(), target);
    let command = DeleteBlocks {
        target: target.to_owned(),
        force: args.force.unwrap_or(false),
        policy: editing.policy(),
    };
    let done = editing
        .editor
        .run(editing.caller, &command)
        .map_err(|e| edit_refusal(e, "", call.clone()))?;
    let out = done.out;
    let summary = if target.contains("..") {
        format!("Deleted {} block(s)", out.ids.len())
    } else {
        format!("Deleted {target}")
    };
    let edits: Vec<Outcome> = out
        .deleted
        .into_iter()
        .map(|d| Outcome {
            changed: d.id.is_some(),
            id: d.id,
            created: Vec::new(),
            removed: d.removed,
            renamed: d.renamed,
            warnings: d.warnings,
        })
        .collect();
    let mut result = outcome(&edits, summary, None);
    result.warnings.extend(out.headings);
    if let Some(watch) = watch.as_mut() {
        let after = editing.editor.store().snapshot();
        add(&mut result, watch.report(after.doc(), editing.doc));
    }
    Ok(result)
}

/// The watch on the file's original text the blocks `target` names hold, on `now`.
fn watch<'a>(editing: &Editing<'_>, now: &'a ooxml_word_edit::WordDoc, target: &str) -> Option<Watch<'a>> {
    let opened = editing.opened.docx().ok()?;
    let docx = now.docx().ok()?;
    let ids = if target.contains("..") {
        blocks_named(now, target).ok()?
    } else {
        vec![target.to_owned()]
    };
    let mut watch = Watch::new(opened, now);
    for id in &ids {
        watch.block(&docx, id, &now.ids().follow(id));
    }
    Some(watch)
}

/// A text edit's outcome.
fn placed(edited: ooxml_word_edit::RangesEdited) -> Outcome {
    Outcome {
        changed: edited.count > 0,
        id: edited.id,
        renamed: edited.renamed,
        ..Outcome::default()
    }
}

/// `word_delete` called with `args`.
fn try_call(args: &WordDelete) -> TryCall {
    let mut arguments = Map::new();
    arguments.insert("doc".into(), Value::from(args.doc.as_str()));
    for (key, value) in [("find", &args.find), ("within", &args.within), ("target", &args.target)] {
        if let Some(v) = value {
            arguments.insert(key.into(), Value::from(v.as_str()));
        }
    }
    for (key, value) in [("collapse_space", args.collapse_space), ("force", args.force)] {
        if let Some(v) = value {
            arguments.insert(key.into(), Value::from(v));
        }
    }
    TryCall {
        tool: "word_delete".to_owned(),
        arguments,
        fill: Vec::new(),
    }
}

/// Registers `word_delete` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<WordDelete>(word_delete, crate::Hosting::PLAIN);
}
