//! Owns: `word_set_text` (docx-agent's `word_set_text`): whole paragraphs' text rewritten
//! -- a paragraph's, or a cell's; with `\n`, a paragraph's later lines new paragraphs after
//! it -- in a paragraph style where asked, `**strong**`, `*emphasis*` and
//! `[text]{Character style}` written in character styles: every item one
//! [`SetStyledText`] in one atomic batch (one history step: an item refused refuses the
//! call, and nothing changes); the reply as docx-agent's `outcome` makes it, with the styled
//! runs (`data.styled`) and the notes; the values the host wrote for references kept on
//! their paragraphs and listed (`data.facts`).
//!
//! Must not: write the document but through the command, or answer a call it does not
//! support yet, refused ([`Unported`]).

use super::{by_reference, Unported};
use crate::describe::short;
use crate::review::{edit_refusal, needed};
use crate::structure::common::{outcome, Outcome};
use crate::{written, Editing};
use ooxml_store::Batch;
use ooxml_tools::{Outcome as Replied, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{EditError, SetStyledText, StyleRefusal, StyleRefusalKind, StyledRun, StyledTextSet, TextKeep};
use ooxml_word_model::address::resolve::resolve;
use ooxml_word_model::ids::Entry;
use serde_json::{json, Map, Value};

/// How many styled runs a reply lists (rule 4); the rest are counted.
const STYLED_CAP: usize = 40;

/// One paragraph's new text.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
pub struct SetTextItem {
    /// Paragraph, or table cell `t:X/c<row>,<col>`.
    #[arg(refs, description = "Paragraph, or table cell t:X/c<row>,<col>.")]
    pub target: String,
    /// The whole new text; \n starts a paragraph; **strong**, *emphasis*, `[text]{Character style}`.
    #[arg(
        facts,
        description = "The whole new text; \\n starts a paragraph; **strong**, *emphasis*, [text]{Character style}."
    )]
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
    /// Paragraph, or table cell `t:X/c<row>,<col>`.
    #[arg(refs, description = "Paragraph, or table cell t:X/c<row>,<col>.")]
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
/// or with a span; the address resolver's refusal of a target; `refused` (nothing
/// changed) for what is not supported yet.
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
                // rule 1: a style beside items is each item's that names none (an empty name
                // names none, as docx-agent's `or` reads it)
                let style = named_style(&item.paragraph_style).or_else(|| named_style(&args.paragraph_style));
                (item.target.clone(), item.text.clone(), style, format!("items[{k}]."))
            })
            .collect(),
        None => {
            let Some(text) = &args.text else {
                return Err(needed("text", "", try_call(args)));
            };
            let target = args.target.clone().unwrap_or_default();
            vec![(target, text.clone(), named_style(&args.paragraph_style), String::new())]
        }
    };
    if work.iter().any(|(target, ..)| by_reference(target)) {
        return Err(Unported::Reference.refusal(try_call(args), &["text", "target"]));
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
    if let Some((target, prefix)) = table_named(editing, &work) {
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            format!("{target} is a table: word_set_text writes a paragraph or a cell (t:X/c<row>,<col>)"),
            WayOut::call(try_call(args)),
        )
        .at(format!("{prefix}target")));
    }
    write(args, editing, &work)
}

/// The items written in one atomic batch, the values written by reference kept, the reply.
fn write(
    args: &WordSetText,
    editing: &Editing<'_>,
    work: &[(String, String, Option<String>, String)],
) -> Result<ToolResult<Value>, ToolError> {
    let policy = editing.policy();
    let ops: Vec<SetStyledText> = work
        .iter()
        .map(|(target, text, style, _)| SetStyledText {
            target: target.clone(),
            text: text.clone(),
            paragraph_style: style.clone(),
            policy: policy.clone(),
        })
        .collect();
    // the values written by reference kept in the same step
    let finish = |out: &Vec<Result<StyledTextSet, _>>, after: &written::After| {
        let items: Vec<StyledTextSet> = out.iter().filter_map(|item| item.as_ref().ok().cloned()).collect();
        let wrote: Vec<Vec<String>> = items.iter().map(|item| item.written.clone()).collect();
        let mut result = reply(work.len(), items);
        let keeps = if editing.written.is_empty() {
            Vec::new()
        } else {
            keep_facts(args, editing, work, &wrote, after, &mut result)
        };
        (result, keeps)
    };
    let (_, result) = written::run_keeping(editing, &Batch::atomic(ops.clone()), finish)
        .map_err(|e| refusal(e, editing, &ops, work, args))?;
    Ok(result)
}

/// A style name docx-agent reads as given: a non-empty one.
fn named_style(style: &Option<String>) -> Option<String> {
    style.clone().filter(|s| !s.is_empty())
}

/// The batch's refusal as docx-agent's: a style an item names not found, at that item's
/// argument (`items[1].paragraph_style`, `text`) with the styles that would do -- the item
/// the first of `ops` whose styles are not known, as the batch checked them in turn; docx-agent
/// names no field when the document refuses an address or a text.
fn refusal(
    error: EditError,
    editing: &Editing<'_>,
    ops: &[SetStyledText],
    work: &[(String, String, Option<String>, String)],
    args: &WordSetText,
) -> ToolError {
    let EditError::Style(StyleRefusal {
        kind: StyleRefusalKind::NotFound,
        message,
        field: Some(field),
        options,
    }) = error
    else {
        return edit_refusal(error, "", try_call(args));
    };
    let head = editing.editor.store().snapshot();
    let k = head
        .doc()
        .docx()
        .ok()
        .and_then(|docx| ops.iter().position(|op| op.styles_known(&docx).is_err()))
        .unwrap_or(0);
    let prefix = work.get(k).map_or("", |(.., prefix)| prefix.as_str());
    let way = WayOut::names(options).unwrap_or_else(|| WayOut::call(try_call(args)));
    ToolError::refusal(Refusal::NotFound, message, way).at(format!("{prefix}{field}"))
}

/// Round 45, R65: the keeps of each value written by reference where it went (docx-agent's
/// `_set_facts`, then `keep_written`), each listed in the reply's `facts` -- a value that is
/// an item's whole text with the paragraphs it went into (`blocks`), kept as their source.
fn keep_facts(
    args: &WordSetText,
    editing: &Editing<'_>,
    work: &[(String, String, Option<String>, String)],
    wrote: &[Vec<String>],
    after: &written::After,
    result: &mut ToolResult<Value>,
) -> Vec<TextKeep> {
    let mut said = Vec::new();
    let mut keeps = Vec::new();
    for fact in editing.written {
        let k = if args.items.is_some() { fact.item() } else { Some(0) };
        let text = k.and_then(|k| work.get(k)).map(|(_, text, ..)| text.as_str());
        let blocks = match (k, text) {
            (Some(k), Some(text)) if fact.is_whole(text) => wrote.get(k).cloned(),
            _ => None,
        };
        if let Some(blocks) = &blocks {
            keeps.extend(written::whole_keeps(after, fact, blocks));
        }
        said.push(written::brief(fact, blocks.as_deref()));
    }
    if let Replied::Done(done) = &mut result.outcome {
        let targets: Vec<&str> = work.iter().map(|(target, ..)| target.as_str()).collect();
        let argument = |field: &str| argument(args, field);
        keeps.extend(written::written_keeps(after, editing, done, &targets, &argument));
        let mut data = match done.data.take() {
            Some(Value::Object(data)) => data,
            _ => Map::new(),
        };
        data.insert("facts".into(), Value::Array(said));
        done.data = Some(Value::Object(data));
    }
    keeps
}

/// The text of the argument `field` names (`text`, `items[0].text`).
fn argument(args: &WordSetText, field: &str) -> Option<String> {
    if field == "text" {
        return args.text.clone();
    }
    let k: usize = field.strip_prefix("items[")?.strip_suffix("].text")?.parse().ok()?;
    args.items.as_ref()?.get(k).map(|item| item.text.clone())
}

/// The first item whose target names a table (on the head: what an address names is no
/// edit's to change), with its field's prefix.
fn table_named<'w>(
    editing: &Editing<'_>,
    work: &'w [(String, String, Option<String>, String)],
) -> Option<(&'w str, &'w str)> {
    let head = editing.editor.store().snapshot();
    let docx = head.doc().docx().ok()?;
    work.iter().find_map(|(target, _, _, prefix)| {
        let followed = head.doc().ids().follow(target);
        matches!(resolve(&docx, &followed), Ok((_, Entry::Table(_)))).then_some((target.as_str(), prefix.as_str()))
    })
}

/// The reply, as docx-agent's `outcome` makes it of the items' edits, with the styled
/// runs (the first [`STYLED_CAP`]) and the notes.
fn reply(paragraphs: usize, items: Vec<StyledTextSet>) -> ToolResult<Value> {
    let mut edits = Vec::new();
    let mut styled: Vec<StyledRun> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for item in items {
        edits.extend(item.edits.into_iter().map(|e| Outcome {
            id: e.id,
            changed: e.changed,
            created: e.created,
            removed: e.removed,
            renamed: e.renamed,
            warnings: Vec::new(),
        }));
        styled.extend(item.styled);
        notes.extend(item.notes);
    }
    let made: usize = edits.iter().map(|e| e.created.len()).sum();
    let mut summary = format!("Set the text of {paragraphs} paragraph(s)");
    if made > 0 {
        summary.push_str(&format!(", {made} new after them"));
    }
    let mut data = None;
    if !styled.is_empty() {
        summary.push_str(&format!("; {} run(s) styled", styled.len()));
        let listed: Vec<Value> = styled.iter().take(STYLED_CAP).map(styled_run).collect();
        let mut out = Map::new();
        out.insert("styled".into(), Value::Array(listed));
        if styled.len() > STYLED_CAP {
            let left = styled.len() - STYLED_CAP;
            out.insert("left_out".into(), json!(left));
            notes.push(format!(
                "{left} more styled run(s) not listed: word_read the paragraphs with detail to see each run's style"
            ));
        }
        data = Some(Value::Object(out));
    }
    let mut result = outcome(&edits, summary, data);
    for note in notes {
        if !result.warnings.contains(&note) {
            result.warnings.push(note);
        }
    }
    result
}

/// A styled run as the reply lists it: range, text (cut), style, direct emphasis.
fn styled_run(styled: &StyledRun) -> Value {
    let mut entry = Map::new();
    entry.insert("range".into(), json!(styled.range));
    entry.insert("text".into(), json!(short(&styled.text, 40)));
    entry.insert("style".into(), json!(styled.style));
    if styled.bold {
        entry.insert("bold".into(), json!(true));
    }
    if styled.italic {
        entry.insert("italic".into(), json!(true));
    }
    Value::Object(entry)
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

/// Registers `word_set_text` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<WordSetText>(word_set_text, crate::Hosting::PLAIN.keeps_written());
}
