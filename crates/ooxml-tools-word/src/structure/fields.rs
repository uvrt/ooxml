//! Owns: `word_fields` (docx-agent's `tools/structure.py`, `word_fields`): its definition,
//! its list, and its edits through the edit core's field commands, their results read from
//! the layout the host hands in ([`Editing::layout`]).
//!
//! Must not: change the document but through a command, or lay out.

use super::common::{call, find_once, need, one_of, outcome, policy, refused};
use crate::describe::short;
use crate::review::Editing;
use ooxml_tools::{ArgEnum, Done, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{
    fields, Day, EditError, FieldEdited, InsertCaption, InsertCrossReference, InsertField, InsertToc, UpdateFields,
};
use serde_json::{json, Value};

/// What `word_fields` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldAction {
    /// List the fields.
    List,
    /// Insert a table of contents.
    InsertToc,
    /// Insert a caption.
    InsertCaption,
    /// Insert a cross-reference.
    InsertCrossReference,
    /// Insert a date.
    InsertDate,
    /// Insert a field.
    InsertField,
    /// Update fields' results.
    Update,
}

/// What a cross-reference shows.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    /// The bookmark's text.
    Text,
    /// Its page.
    Page,
}

/// `word_fields`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_fields",
    description = include_str!("word_fields.md"),
    group = "word_structure",
    mutates,
    places = "mark",
    place_own("after", "before", "at"),
    place_find
)]
pub struct WordFields {
    /// Document id.
    pub doc: String,
    /// What to do.
    pub action: FieldAction,
    /// insert_toc, insert_caption: the block before.
    #[arg(refs)]
    pub after: Option<String>,
    /// insert_toc, insert_caption: the block after.
    #[arg(refs)]
    pub before: Option<String>,
    /// Other inserts: position or range (p:A@5), or find.
    #[arg(refs)]
    pub at: Option<String>,
    /// Other inserts: text occurring once; goes after it.
    pub find: Option<String>,
    /// insert_toc: heading levels. Default 1-3.
    pub levels: Option<String>,
    /// insert_cross_reference: the bookmark.
    pub bookmark: Option<String>,
    /// insert_cross_reference: its text (default) or page.
    pub kind: Option<ReferenceKind>,
    /// insert_caption: Figure (default), Table...
    pub label: Option<String>,
    /// insert_caption: text after the number.
    pub text: Option<String>,
    /// insert_field: the instruction, e.g. NUMPAGES.
    pub instruction: Option<String>,
    /// update: only these fields (fld:...).
    #[arg(refs)]
    pub ids: Option<Vec<String>>,
}

/// `word_fields` (docx-agent's `word_fields`): list the fields; insert a table of
/// contents, a caption, a cross-reference, a date or another field; update every result
/// from the layout -- each one command.
#[must_use]
pub fn word_fields(args: &WordFields, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let list = call("word_fields", &[("doc", json!(args.doc)), ("action", json!("list"))]);
    match run(args, editing, &list) {
        Ok(result) => result,
        Err(error) => ToolResult::failed(error),
    }
}

/// The most fields a list shows.
const LISTED: usize = 100;

/// A field edit's reply: its outcome with `summary` and `data`, or its refusal.
fn respond(
    done: Result<FieldEdited, EditError>,
    summary: impl Fn(&FieldEdited) -> String,
    data: impl Fn(&FieldEdited) -> Option<Value>,
    list: &TryCall,
) -> ToolResult<Value> {
    match done {
        Ok(out) => {
            let text = summary(&out);
            let data = data(&out);
            outcome(&[out.into()], text, data)
        }
        Err(error) => refused(error, list.clone()),
    }
}

/// The day a call's date names (the epoch where it names none).
fn today(editing: &Editing<'_>) -> Day {
    Day::parse(editing.date).unwrap_or(Day {
        year: 1970,
        month: 1,
        day: 1,
    })
}

fn run(args: &WordFields, editing: &Editing<'_>, list: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    let (today, layout, editor) = (today(editing), editing.layout, editing.editor);
    Ok(match args.action {
        FieldAction::List => listed(editing)?,
        FieldAction::Update => {
            let command = UpdateFields {
                ids: args.ids.clone(),
                today,
                layout,
                locale: editing.locale,
            };
            respond(
                editor.run(editing.caller, &command).map(|d| d.out),
                |out| format!("Updated fields: {} result(s) changed", out.count),
                |out| Some(json!({"changed_results": out.count})),
                list,
            )
        }
        FieldAction::InsertToc => {
            let levels = levels(args.levels.as_deref())?;
            let (after, before) = beside(args, editing)?;
            let command = InsertToc {
                after,
                before,
                levels,
                policy: policy(editing),
                today,
                layout,
                locale: editing.locale,
            };
            respond(
                editor.run(editing.caller, &command).map(|d| d.out),
                |out| format!("Inserted a table of contents {}", out.id.as_deref().unwrap_or("")),
                |_| None,
                list,
            )
        }
        FieldAction::InsertCaption => {
            let (after, before) = beside(args, editing)?;
            let command = InsertCaption {
                anchor: after.clone().or(before).unwrap_or_default(),
                after: after.is_some(),
                label: args.label.clone().unwrap_or_else(|| "Figure".to_owned()),
                text: args.text.clone().unwrap_or_default(),
                policy: policy(editing),
                today,
                layout,
            };
            respond(
                editor.run(editing.caller, &command).map(|d| d.out),
                |_| "Inserted a caption".to_owned(),
                |_| None,
                list,
            )
        }
        FieldAction::InsertCrossReference | FieldAction::InsertDate | FieldAction::InsertField => {
            insert(args, editing, list)?
        }
    })
}

/// A cross-reference, a date or another field put at a position.
fn insert(args: &WordFields, editing: &Editing<'_>, list: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    let (today, layout, editor) = (today(editing), editing.layout, editing.editor);
    let at = position(args, editing, list)?;
    let summary = |out: &FieldEdited| format!("Inserted field {}", out.id.as_deref().unwrap_or(""));
    let done = match args.action {
        FieldAction::InsertCrossReference => {
            let command = InsertCrossReference {
                at,
                bookmark: need(args.bookmark.as_deref(), "bookmark", list)?.to_owned(),
                page: args.kind == Some(ReferenceKind::Page),
                policy: policy(editing),
                layout,
            };
            editor.run(editing.caller, &command).map(|d| d.out)
        }
        FieldAction::InsertDate => {
            let picture = "d MMMM yyyy";
            let command = InsertField {
                at,
                instruction: format!(" DATE \\@ \"{picture}\" "),
                result: Some(ooxml_word_edit::format_date(picture, today)),
                simple: false,
                policy: policy(editing),
                today,
                layout,
                locale: editing.locale,
            };
            editor.run(editing.caller, &command).map(|d| d.out)
        }
        _ => {
            let command = InsertField {
                at,
                instruction: need(args.instruction.as_deref(), "instruction", list)?.to_owned(),
                result: None,
                simple: false,
                policy: policy(editing),
                today,
                layout,
                locale: editing.locale,
            };
            editor.run(editing.caller, &command).map(|d| d.out)
        }
    };
    Ok(respond(done, summary, |_| None, list))
}

/// The fields as the list shows them, the first [`LISTED`].
fn listed(editing: &Editing<'_>) -> Result<ToolResult<Value>, ToolError> {
    let head = editing.editor.store().snapshot();
    let found = fields(head.doc()).map_err(|e| ToolError::internal(e.to_string(), "word:fields"))?;
    let data: Vec<Value> = found
        .into_iter()
        .take(LISTED)
        .map(|f| {
            json!({"id": f.id, "keyword": f.keyword, "instruction": f.instruction,
                   "result": short(&f.result, 160), "paragraph": f.paragraph})
        })
        .collect();
    let mut done = Done::new("Fields".to_owned());
    done.data = Some(Value::Array(data));
    Ok(ToolResult::done(done))
}

/// `levels` as first-last (default 1-3).
fn levels(levels: Option<&str>) -> Result<(i64, i64), ToolError> {
    let Some(text) = levels else { return Ok((1, 3)) };
    let parsed = match text.as_bytes() {
        [a @ b'1'..=b'9', b'-', b @ b'1'..=b'9'] if a <= b => Some((i64::from(a - b'0'), i64::from(b - b'0'))),
        _ => None,
    };
    parsed.ok_or_else(|| {
        ToolError::refusal(
            Refusal::InvalidArguments,
            "levels is first-last, e.g. 1-3",
            WayOut::names(["1-3", "1-2", "2-4"]).unwrap_or_else(|| WayOut::call(call("word_fields", &[]))),
        )
        .at("levels")
    })
}

/// A table of contents' or caption's block (docx-agent's `_beside` and `_place`): `after`
/// or `before` -- `at` read as `after` --, `after` saying its side as `at` does
/// (`before:X`, `after:X`, `end`).
fn beside(args: &WordFields, editing: &Editing<'_>) -> Result<(Option<String>, Option<String>), ToolError> {
    let (after, before) = match (&args.at, &args.after, &args.before) {
        (Some(at), None, None) => (Some(at.clone()), None),
        _ => (args.after.clone(), args.before.clone()),
    };
    if after.is_some() == before.is_some() {
        let way_out = WayOut::names(["after", "before"]).unwrap_or_else(|| WayOut::call(call("word_fields", &[])));
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            "give exactly one of after, before",
            way_out,
        ));
    }
    let Some(after) = after else { return Ok((None, before)) };
    if after == "end" {
        let head = editing.editor.store().snapshot();
        let docx = head
            .doc()
            .docx()
            .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
        return Ok((super::sections::body_blocks(&docx).last().cloned(), None));
    }
    if let Some(anchor) = after.strip_prefix("before:") {
        return Ok((None, Some(anchor.to_owned())));
    }
    Ok((Some(after.strip_prefix("after:").unwrap_or(&after).to_owned()), None))
}

/// The position another insert goes at (`_position`): `at`, `after`/`before` as a mark's
/// place (a block's end, or its start), or the end of text found once.
fn position(args: &WordFields, editing: &Editing<'_>, list: &TryCall) -> Result<String, ToolError> {
    let mut at = args.at.clone();
    if at.is_none() && args.find.is_none() {
        if let Some((side, anchor)) = args
            .after
            .as_ref()
            .map(|a| ("after", a))
            .or_else(|| args.before.as_ref().map(|b| ("before", b)))
        {
            at = mark_place(side, anchor);
            if at.is_none() {
                return Err(ToolError::refusal(
                    Refusal::InvalidArguments,
                    format!(
                        "{} goes at a position: at p:<id>@<offset>, or a range (at its end)",
                        args.action.as_str()
                    ),
                    WayOut::call(list.clone()),
                )
                .at("at"));
            }
        }
    }
    match one_of(&[("at", at.is_some()), ("find", args.find.is_some())], list)?.as_str() {
        "find" => {
            let found = find_once(editing, args.find.as_deref().unwrap_or(""), list)?;
            let (base, span) = found.rsplit_once('@').unwrap_or((&found, ""));
            Ok(format!("{base}@{}", span.rsplit(':').next().unwrap_or("")))
        }
        _ => Ok(at.unwrap_or_default()),
    }
}

/// A mark's own place for `side` of `anchor` (docx-agent's `positions._write("mark")`):
/// after it as it is; before it, a position at its start.
fn mark_place(side: &str, anchor: &str) -> Option<String> {
    if side == "after"
        || ["after:", "before:", "replace:", "end"]
            .iter()
            .any(|s| anchor.starts_with(s))
    {
        return Some(anchor.to_owned());
    }
    let Some((head, span)) = anchor.split_once('@') else {
        return (anchor.starts_with("p:") && !anchor.contains("..")).then(|| format!("{anchor}@0"));
    };
    let start = span.split(':').next().unwrap_or("").split("..").next().unwrap_or("");
    Some(format!("{head}@{start}"))
}

/// Registers `word_fields` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<WordFields>(word_fields, crate::Hosting::SERVED.layout());
}
