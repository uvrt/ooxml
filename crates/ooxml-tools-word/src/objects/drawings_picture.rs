//! Owns: `word_drawings insert_picture` (docx-agent's `word_drawings` and `_insert_picture`):
//! the picture's bytes from the session's blob, checked as docx-agent checks the call
//! (an image blob, alt text, a float only with wrap, a width alone), a paragraph of its own
//! for `after:`/`before:` a block (aligned when the picture stays in the line), the picture
//! inserted in the line and floated when `wrap` is given, and the reply (its id and size,
//! the paragraph made). An SVG (its PNG fallback is drawn by a later step), a tint and a
//! name given with `ref` are left to the host, as is a call when the host hands over no
//! blobs.
//!
//! Must not: change the document but through a command, or change it at all before every
//! check the call can fail on has passed.

use super::drawings_args::{WordDrawings, Wrap};
use crate::blobs::blob;
use crate::review::{needed, Editing};
use crate::structure::common::{outcome, policy, refused, Outcome};
use crate::style::round2;
use ooxml_store::Command;
use ooxml_tools::{ArgEnum, Refusal, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{
    picture_size, written_size, DrawingValues, EditError, FloatDrawing, FormatEdit, FormatPlan, InsertParagraph,
    InsertPicture, Property, Setting, Value as SettingValue,
};
use serde_json::{json, Value};

fn unported(what: &str, retry: &TryCall) -> ToolResult<Value> {
    refused(EditError::Unsupported(what.to_owned()), retry.clone())
}

fn invalid(message: &str, field: &str, way_out: WayOut) -> ToolError {
    ToolError::refusal(Refusal::InvalidArguments, message.to_owned(), way_out).at(field)
}

/// Whether `data` is meant as an SVG (ooxml-edit's `looks_like_svg`): markup with an
/// `<svg` tag in its first 4 KiB.
fn looks_like_svg(data: &[u8]) -> bool {
    let head = data.get(..4096).unwrap_or(data);
    let start = head
        .iter()
        .position(|b| !matches!(b, 0xEF | 0xBB | 0xBF | b' ' | b'\t' | b'\r' | b'\n'))
        .unwrap_or(head.len());
    let head = head.get(start..).unwrap_or_default();
    head.starts_with(b"<")
        && head.windows(5).any(|w| {
            w.starts_with(b"<svg") && matches!(w.get(4), Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'>' | b'/'))
        })
}

/// A paragraph of the picture's own beside a block (`after:`/`before:` with no `find`).
fn own_paragraph(args: &WordDrawings) -> Option<(bool, String)> {
    let at = args.at.as_deref().filter(|_| args.find.is_none())?;
    let (side, block) = at.split_once(':')?;
    matches!(side, "after" | "before").then(|| (side == "after", block.to_owned()))
}

/// What a float takes (docx-agent's `_floating` for a picture): `None` without `wrap`,
/// refused when a place is given without it.
fn floating(args: &WordDrawings, align_taken: bool, list: &TryCall) -> Result<Option<DrawingValues>, ToolError> {
    let align = args.align.filter(|_| !align_taken);
    let Some(wrap) = args.wrap else {
        if args.x.is_some() || args.y.is_some() || align.is_some() || args.against.is_some() {
            let wraps = [Wrap::Square, Wrap::Tight, Wrap::TopAndBottom, Wrap::Front, Wrap::Behind];
            let options = WayOut::names(wraps.iter().map(|w| w.as_str())).unwrap_or_else(|| WayOut::call(list.clone()));
            return Err(invalid(
                "a picture floats with wrap: give wrap to place it",
                "wrap",
                options,
            ));
        }
        return Ok(None);
    };
    Ok(Some(DrawingValues {
        wrap: Some(wrap.as_str().to_owned()),
        horizontal_from: Some(args.against.map_or("column", |a| a.as_str()).to_owned()),
        x_align: align.map(|a| a.as_str().to_owned()),
        x: if align.is_some() { None } else { args.x },
        y: args.y,
        ..DrawingValues::default()
    }))
}

/// The bytes of the image blob `image` names, refused when it is not an image.
fn image_bytes(
    args: &WordDrawings,
    editing: &Editing<'_>,
    list: &TryCall,
) -> Result<Option<std::sync::Arc<[u8]>>, ToolError> {
    let Some(blobs) = editing.blobs else { return Ok(None) };
    let Some(handle) = args.image.as_deref() else {
        return Err(needed("image", "", list.clone()));
    };
    let found = blob(blobs, handle, list)?;
    if !found.mime.starts_with("image/") {
        let message = format!("{handle} is {}, not an image", found.mime);
        return Err(invalid(&message, "image", WayOut::call(list.clone())));
    }
    Ok(Some(found.data))
}

/// A call checked as docx-agent checks it: what it makes.
struct Plan {
    /// The paragraph of the picture's own.
    paragraph: Option<InsertParagraph>,
    /// Whether that paragraph takes `align` (the picture stays in the line).
    align_taken: bool,
    /// The position, without a paragraph of its own.
    at: Option<String>,
    data: std::sync::Arc<[u8]>,
    alt_text: String,
    float: Option<DrawingValues>,
    size: (f64, f64),
}

/// The call checked in docx-agent's order (`Ok(Err(reply))`: a reply ending the call before
/// anything is changed).
fn checked(
    args: &WordDrawings,
    editing: &Editing<'_>,
    list: &TryCall,
) -> Result<Result<Plan, ToolResult<Value>>, ToolError> {
    let unported_blobs = || unported("word_drawings insert_picture without the session's blobs", list);
    if args.reference.is_some() {
        return Ok(Err(unported("word_drawings ref", list)));
    }
    let own = own_paragraph(args);
    let paragraph = own.as_ref().map(|(after, block)| InsertParagraph {
        anchor: block.clone(),
        after: *after,
        text: String::new(),
        style: None,
        policy: policy(editing),
    });
    // docx-agent makes the paragraph first: its refusal comes before the others
    if let Some(command) = &paragraph {
        if let Err(error) = command.validate(editing.editor.store().snapshot().doc()) {
            return Ok(Err(refused(error, list.clone())));
        }
    }
    let align_taken = own.is_some() && args.wrap.is_none() && args.align.is_some();
    let Some(data) = image_bytes(args, editing, list)? else {
        return Ok(Err(unported_blobs()));
    };
    let alt_text = args
        .alt_text
        .clone()
        .ok_or_else(|| needed("alt_text", " for a picture", list.clone()))?;
    let float = floating(args, align_taken, list)?;
    if args.height.is_some() {
        let message = "a picture keeps its aspect ratio: give width";
        return Err(invalid(message, "height", WayOut::call(list.clone())));
    }
    let at = match &paragraph {
        Some(_) => None,
        None => Some(super::drawings::position(args, editing, list)?),
    };
    let svg = looks_like_svg(&data);
    if args.color.is_some() && !svg {
        let message = "only an SVG picture can be tinted: this one is a bitmap; leave color out, or give an SVG";
        return Err(invalid(message, "color", WayOut::call(list.clone())));
    }
    if svg {
        return Ok(Err(unported("word_drawings insert_picture of an SVG", list)));
    }
    let size = match picture_size(&data, args.width) {
        Ok(size) => size,
        Err(error) => return Ok(Err(refused(error, list.clone()))),
    };
    Ok(Ok(Plan {
        paragraph,
        align_taken,
        at,
        data,
        alt_text,
        float,
        size,
    }))
}

/// `insert_picture`: every check first, then the paragraph of its own (aligned), the
/// picture, its float -- each one command.
pub(super) fn insert_picture(
    args: &WordDrawings,
    editing: &Editing<'_>,
    list: &TryCall,
) -> Result<ToolResult<Value>, ToolError> {
    let plan = match checked(args, editing, list)? {
        Ok(plan) => plan,
        Err(reply) => return Ok(reply),
    };
    Ok(match made(args, editing, plan) {
        Ok((edits, data)) => {
            let summary = format!(
                "Inserted picture {}",
                data.get("id").and_then(Value::as_str).unwrap_or("")
            );
            outcome(&edits, summary, Some(data))
        }
        Err(error) => refused(error, list.clone()),
    })
}

/// The commands a checked call runs: their outcomes and the reply's data.
fn made(args: &WordDrawings, editing: &Editing<'_>, plan: Plan) -> Result<(Vec<Outcome>, Value), EditError> {
    let (editor, caller) = (editing.editor, editing.caller);
    let mut edits: Vec<Outcome> = Vec::new();
    let mut own = None;
    if let Some(command) = &plan.paragraph {
        let done = editor.run(caller, command)?;
        let id = done.out.id.clone();
        edits.push(Outcome {
            id: Some(id.clone()),
            changed: true,
            created: vec![id.clone()],
            removed: Vec::new(),
            renamed: done.out.renamed,
            warnings: Vec::new(),
        });
        if plan.align_taken {
            align(editing, &id, args, &mut edits)?;
        }
        own = Some(id);
    }
    let command = InsertPicture {
        at: own
            .as_ref()
            .map_or_else(|| plan.at.clone().unwrap_or_default(), |p| format!("{p}@0")),
        image: plan.data,
        width: args.width,
        alt_text: Some(plan.alt_text),
        policy: policy(editing),
    };
    let inserted = editor.run(caller, &command)?.out;
    let id = inserted.id.clone();
    edits.push(inserted.into());
    if let Some(values) = plan.float {
        let command = FloatDrawing {
            drawing: id.clone(),
            values,
            policy: policy(editing),
        };
        edits.push(editor.run(caller, &command)?.out.into());
    }
    let (width, height) = written_size(plan.size);
    let mut data = serde_json::Map::new();
    data.insert("id".into(), json!(id));
    data.insert("size".into(), json!([round2(width), round2(height)]));
    if let Some(paragraph) = own {
        data.insert("paragraph".into(), json!(paragraph));
    }
    Ok((edits, Value::Object(data)))
}

/// The paragraph of the picture's own aligned (docx-agent's `format_paragraph`).
fn align(editing: &Editing<'_>, id: &str, args: &WordDrawings, edits: &mut Vec<Outcome>) -> Result<(), EditError> {
    let value = args.align.map_or("left", |a| a.as_str());
    let command = FormatEdit {
        targets: vec![id.to_owned()],
        plan: FormatPlan {
            paragraph: vec![Setting {
                property: Property::Alignment,
                value: Some(SettingValue::Text(value.to_owned())),
            }],
            ..FormatPlan::default()
        },
        cleared: false,
        policy: policy(editing),
    };
    let done = editing.editor.run(editing.caller, &command)?;
    for step in done.out.steps {
        edits.push(Outcome {
            id: Some(step.id),
            changed: step.changed,
            created: Vec::new(),
            removed: step.removed,
            renamed: step.renamed,
            warnings: step.warnings,
        });
    }
    Ok(())
}
