//! Owns: `word_set_text` (docx-agent's `word_set_text`): whole paragraphs' text rewritten
//! -- a paragraph's, or a cell's first paragraph's; with `\n`, a paragraph's later lines new
//! paragraphs after it -- every item one [`SetText`] in one atomic batch (one history
//! step: an item refused refuses the call, and nothing changes); the reply as docx-agent's
//! `outcome` makes it.
//!
//! Must not: write the document but through the command, or answer a call it does not
//! port otherwise than by saying so ([`Unported`]).

use super::{by_reference, merge_renamed, text_unported, Unported};
use crate::review::{edit_refusal, needed};
use crate::Editing;
use ooxml_store::Batch;
use ooxml_tools::{Done, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{SetText, TextEdited};
use serde_json::{Map, Value};

/// One paragraph's new text.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
pub struct SetTextItem {
    /// Paragraph, or table cell t:X/c<row>,<col>.
    #[arg(refs)]
    pub target: String,
    /// The whole new text; \n starts a paragraph; **strong**, *emphasis*, [text]{Character style}.
    #[arg(facts)]
    pub text: String,
    /// Paragraph style for it (name or id).
    pub paragraph_style: Option<String>,
}

/// `word_set_text`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(name = "word_set_text", description = include_str!("word_set_text.md"), mutates)]
pub struct WordSetText {
    /// Document id.
    pub doc: String,
    /// Paragraph, or table cell t:X/c<row>,<col>.
    #[arg(refs)]
    pub target: Option<String>,
    /// The whole new text, as items' text.
    #[arg(facts)]
    pub text: Option<String>,
    /// Its paragraph style.
    pub paragraph_style: Option<String>,
    /// Or several paragraphs.
    pub items: Option<Vec<SetTextItem>>,
}

/// `word_set_text` on the document `editing` names. A failed reply is
/// `invalid_arguments` for a call without exactly one of target and items, without text,
/// or with a span; the address resolver's refusal of a target; `unported` (nothing
/// changed) for what the Rust core does not write yet.
#[must_use]
pub fn word_set_text(args: &WordSetText, editing: &Editing<'_>) -> ToolResult<Value> {
    run(args, editing).unwrap_or_else(ToolResult::failed)
}

fn run(args: &WordSetText, editing: &Editing<'_>) -> Result<ToolResult<Value>, ToolError> {
    crate::review::stopped(editing.cancel)?;
    let given: Vec<&str> = [("target", args.target.is_some()), ("items", args.items.is_some())]
        .into_iter()
        .filter_map(|(name, set)| set.then_some(name))
        .collect();
    if given.len() != 1 {
        let mut message = "give exactly one of target, items".to_owned();
        if !given.is_empty() {
            message.push_str(&format!("; got {}", given.join(", ")));
        }
        let field = given.get(1).copied().unwrap_or("target");
        let options = WayOut::options(vec![Value::from("target"), Value::from("items")]);
        let mut call = args.clone();
        call.items = None;
        let way = options.map_or_else(|| WayOut::call(try_call(&call)), |w| w.and_call(try_call(&call)));
        return Err(ToolError::refusal(Refusal::InvalidArguments, message, way).at(field));
    }
    if args.items.as_ref().is_some_and(Vec::is_empty) {
        return Ok(ToolResult::nothing_given("written", "items"));
    }
    let work: Vec<(String, String, Option<String>, String)> = match &args.items {
        Some(items) => items
            .iter()
            .enumerate()
            .map(|(k, item)| {
                let style = item.paragraph_style.clone().or_else(|| args.paragraph_style.clone());
                (item.target.clone(), item.text.clone(), style, format!("items[{k}]."))
            })
            .collect(),
        None => {
            let Some(text) = &args.text else {
                return Err(needed("text", "", try_call(args)));
            };
            let target = args.target.clone().unwrap_or_default();
            vec![(target, text.clone(), args.paragraph_style.clone(), String::new())]
        }
    };
    if let Some(unported) = work.iter().find_map(|(target, text, style, _)| unported(target, text, style.as_ref())) {
        return Ok(unported.reply());
    }
    if let Some((target, _, _, prefix)) = work.iter().find(|(target, ..)| target.contains("..")) {
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            format!(
                "{target} is a span: word_set_text writes one paragraph or cell per target; give items, one per paragraph"
            ),
            WayOut::call(try_call(args)),
        )
        .at(format!("{prefix}target")));
    }
    let policy = editing.policy();
    let ops = work
        .iter()
        .map(|(target, text, ..)| SetText {
            target: target.clone(),
            text: text.clone(),
            policy: policy.clone(),
        })
        .collect();
    let field = if args.items.is_some() { "items" } else { "target" };
    let done = editing
        .editor
        .run(editing.caller, &Batch::atomic(ops))
        .map_err(|e| edit_refusal(e, field, try_call(args)))?;
    let edits: Vec<TextEdited> = done.out.into_iter().filter_map(Result::ok).collect();
    Ok(ToolResult::done(reply(work.len(), &edits)))
}

/// What of an item is not ported: a ref, a paragraph style, styled or spaced text, an
/// address other than a paragraph's or (one line) a cell's.
fn unported(target: &str, text: &str, style: Option<&String>) -> Option<Unported> {
    if by_reference(target) {
        return Some(Unported::Reference);
    }
    if let Some(found) = text_unported(text) {
        return Some(found);
    }
    if style.is_some() {
        return Some(Unported::ParagraphStyle);
    }
    let paragraph = target.starts_with("p:") && !target.contains('@') && !target.contains("..");
    let lines = text.contains('\n') || text.contains('\r');
    if paragraph || (cell(target) && !lines) || target.contains("..") {
        return None;
    }
    Some(Unported::Form)
}

/// Whether `target` is a cell's address, `t:X/c<row>,<col>`.
fn cell(target: &str) -> bool {
    let Some((table, place)) = target.split_once("/c") else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    table.starts_with("t:")
        && !table.contains('/')
        && place.split_once(',').is_some_and(|(r, c)| digits(r) && digits(c))
}

/// The reply, as docx-agent's `outcome` makes it of the edits.
fn reply(paragraphs: usize, edits: &[TextEdited]) -> Done<Value> {
    let made: usize = edits.iter().map(|e| e.created.len()).sum();
    let mut summary = format!("Set the text of {paragraphs} paragraph(s)");
    if made > 0 {
        summary.push_str(&format!(", {made} new after them"));
    }
    let mut done = Done::new(summary);
    for edit in edits {
        for id in &edit.created {
            if !done.created.contains(id) {
                done.created.push(id.clone());
            }
        }
        if edit.changed && !done.created.contains(&edit.id) && !done.changed.contains(&edit.id) {
            done.changed.push(edit.id.clone());
        }
        merge_renamed(&mut done.renamed, &edit.renamed);
    }
    done
}

/// `word_set_text` called with `args`.
fn try_call(args: &WordSetText) -> TryCall {
    let mut arguments = Map::new();
    arguments.insert("doc".into(), Value::from(args.doc.as_str()));
    for (key, value) in [
        ("target", &args.target),
        ("text", &args.text),
        ("paragraph_style", &args.paragraph_style),
    ] {
        if let Some(v) = value {
            arguments.insert(key.into(), Value::from(v.as_str()));
        }
    }
    if let Some(items) = &args.items {
        let items = items
            .iter()
            .map(|item| {
                let mut one = Map::new();
                one.insert("target".into(), Value::from(item.target.as_str()));
                one.insert("text".into(), Value::from(item.text.as_str()));
                if let Some(style) = &item.paragraph_style {
                    one.insert("paragraph_style".into(), Value::from(style.as_str()));
                }
                Value::Object(one)
            })
            .collect();
        arguments.insert("items".into(), Value::Array(items));
    }
    TryCall {
        tool: "word_set_text".to_owned(),
        arguments,
        fill: Vec::new(),
    }
}
