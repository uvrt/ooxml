//! Owns: `word_drawings`' handler (docx-agent's `tools/objects.py`, `word_drawings`): the
//! drawings listed (pictures, then charts: id, kind, size, inline, alt text); a text box
//! or a shape inserted at a position or after text found once; a drawing moved, resized,
//! floated, put in the line or set (wrapping, order, place, a picture's alt text) -- each
//! one command -- and the reply. A picture inserted from a blob is `drawings_picture`'s; a
//! name given with `ref` is left to the host.
//!
//! Must not: change the document but through a command.

use super::drawings_args::{DrawingAction, WordDrawings, Wrap, ZOrder};
use crate::describe::short;
use crate::review::Editing;
use crate::structure::common::{call, find_once, need, one_of, outcome, policy, refused, Outcome};
use crate::structure::notes::end_of;
use ooxml_base::pyfmt::round2;
use ooxml_tools::{ArgEnum, Done, Refusal, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::units::emu;
use ooxml_word_edit::{
    DrawingEdited, DrawingValues, EditError, Edited, FloatDrawing, InlineDrawing, InsertShape, SetDrawing,
};
use ooxml_word_model::docx::drawings::{drawing_kind, graphics, pictures, Drawing};
use ooxml_word_model::ids::WP;
use serde_json::{json, Map, Value};

impl From<DrawingEdited> for Outcome {
    fn from(out: DrawingEdited) -> Self {
        Outcome {
            id: Some(out.id),
            changed: out.changed,
            created: out.created,
            removed: Vec::new(),
            renamed: ooxml_word_model::ids::renames_text(&out.renamed),
            warnings: out.warnings,
        }
    }
}

/// `word_drawings` (docx-agent's `word_drawings`): drawings listed, inserted, moved,
/// resized, floated, put in the line or set.
#[must_use]
pub fn word_drawings(args: &WordDrawings, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let list = call("word_drawings", &[("doc", json!(args.doc)), ("action", json!("list"))]);
    match run(args, editing, &list) {
        Ok(result) => result,
        Err(error) => ToolResult::failed(error),
    }
}

fn not_supported(what: &str, retry: &TryCall) -> ToolResult<Value> {
    refused(EditError::Unsupported(what.to_owned()), retry.clone())
}

fn wrap_name(wrap: Option<Wrap>) -> Option<String> {
    wrap.map(|w| w.as_str().to_owned())
}

/// What a reply says of a command's outcome.
struct Said {
    summary: String,
    data: Option<Value>,
    /// Whether the renames the stamping gave are reported (docx-agent's alt text, an edit
    /// of its own, reports none).
    renames: bool,
}

/// A command's outcome as the tool's reply.
fn reply(
    result: Result<Edited<DrawingEdited>, EditError>,
    said: &dyn Fn(&DrawingEdited) -> Said,
    list: &TryCall,
) -> ToolResult<Value> {
    match result {
        Ok(done) => {
            let Said { summary, data, renames } = said(&done.out);
            let mut edit: Outcome = done.out.into();
            if !renames {
                edit.renamed.clear();
            }
            outcome(&[edit], summary, data)
        }
        Err(error) => refused(error, list.clone()),
    }
}

fn run(args: &WordDrawings, editing: &Editing<'_>, list: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    match args.action {
        DrawingAction::List => listed(editing, list),
        DrawingAction::InsertPicture => super::drawings_picture::insert_picture(args, editing, list),
        DrawingAction::InsertTextBox | DrawingAction::InsertShape => {
            if args.reference.is_some() {
                return Ok(not_supported("word_drawings ref", list));
            }
            let command = insert_shape(args, editing, list)?;
            let said = |out: &DrawingEdited| Said {
                summary: format!("Inserted {}", out.id),
                data: Some(json!({"id": out.id})),
                renames: true,
            };
            Ok(reply(editing.editor.run(editing.caller, &command), &said, list))
        }
        DrawingAction::Set => set_action(args, editing, list),
        DrawingAction::Move | DrawingAction::Resize | DrawingAction::Float | DrawingAction::Inline => {
            frame_action(args, editing, list)
        }
    }
}

/// `insert_text_box` and `insert_shape` with docx-agent's defaults.
fn insert_shape(args: &WordDrawings, editing: &Editing<'_>, list: &TryCall) -> Result<InsertShape, ToolError> {
    let text_box = args.action == DrawingAction::InsertTextBox;
    Ok(InsertShape {
        at: ooxml_word_model::address::Address::from(position(args, editing, list)?),
        text_box,
        preset: match &args.preset {
            Some(preset) if !text_box => preset.clone(),
            _ => "rect".to_owned(),
        },
        text: if text_box {
            Some(args.text.clone().unwrap_or_default())
        } else {
            args.text.clone()
        },
        width: emu(args.width.unwrap_or(if text_box { 144.0 } else { 72.0 })),
        height: emu(args.height.unwrap_or(72.0)),
        x: emu(args.x.unwrap_or(0.0)),
        y: emu(args.y.unwrap_or(0.0)),
        wrap: wrap_name(args.wrap).unwrap_or_else(|| (if text_box { "square" } else { "front" }).to_owned()),
        policy: policy(editing),
    })
}

/// `move`, `resize`, `float` and `inline`: a drawing already as asked is no change.
fn frame_action(args: &WordDrawings, editing: &Editing<'_>, list: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    let target = need(args.target.as_deref(), "target", list)?.to_owned();
    let summary = format!("{}: {target}", args.action.as_str());
    let said = |out: &DrawingEdited| Said {
        summary: if out.changed {
            summary.clone()
        } else {
            format!("No change to {target}")
        },
        data: None,
        renames: true,
    };
    let set = |values: DrawingValues| SetDrawing {
        drawing: target.clone(),
        alt_text: None,
        values: Some(values),
        policy: policy(editing),
    };
    let (editor, caller) = (editing.editor, editing.caller);
    let result = match args.action {
        DrawingAction::Move => {
            let x = *need(args.x.as_ref(), "x", list)?;
            let y = *need(args.y.as_ref(), "y", list)?;
            editor.run(
                caller,
                &set(DrawingValues {
                    x: Some(emu(x)),
                    y: Some(emu(y)),
                    ..DrawingValues::default()
                }),
            )
        }
        DrawingAction::Resize => editor.run(
            caller,
            &set(DrawingValues {
                width: args.width.map(emu),
                height: args.height.map(emu),
                ..DrawingValues::default()
            }),
        ),
        DrawingAction::Float => {
            let values = DrawingValues {
                wrap: wrap_name(args.wrap),
                horizontal_from: args.against.map(|a| a.as_str().to_owned()),
                x_align: args.align.map(|a| a.as_str().to_owned()),
                x: if args.align.is_some() { None } else { args.x.map(emu) },
                y: args.y.map(emu),
                ..DrawingValues::default()
            };
            let command = FloatDrawing {
                drawing: target.clone(),
                values,
                policy: policy(editing),
            };
            editor.run(caller, &command)
        }
        _ => {
            let command = InlineDrawing {
                drawing: target.clone(),
                policy: policy(editing),
            };
            editor.run(caller, &command)
        }
    };
    Ok(reply(result, &said, list))
}

/// `set`: a picture's alt text first, then wrapping, order and place.
fn set_action(args: &WordDrawings, editing: &Editing<'_>, list: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    let target = need(args.target.as_deref(), "target", list)?.to_owned();
    let values = DrawingValues {
        wrap: wrap_name(args.wrap),
        z_order: args.z_order.map(|z| match z {
            ZOrder::Front => ooxml_word_edit::ZOrder::Front,
            ZOrder::Back => ooxml_word_edit::ZOrder::Back,
            ZOrder::Forward => ooxml_word_edit::ZOrder::Forward,
            ZOrder::Backward => ooxml_word_edit::ZOrder::Backward,
        }),
        x: args.x.map(emu),
        y: args.y.map(emu),
        ..DrawingValues::default()
    };
    let given = !values.is_empty();
    if args.alt_text.is_none() && !given {
        let options =
            WayOut::names(["wrap", "z_order", "x", "y", "alt_text"]).unwrap_or_else(|| WayOut::call(list.clone()));
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            "set needs something to set".to_owned(),
            options,
        ));
    }
    let command = SetDrawing {
        drawing: target.clone(),
        alt_text: args.alt_text.clone(),
        values: given.then_some(values),
        policy: policy(editing),
    };
    let said = |_: &DrawingEdited| Said {
        summary: format!("Set {target}"),
        data: None,
        renames: args.alt_text.is_none(),
    };
    Ok(reply(editing.editor.run(editing.caller, &command), &said, list))
}

/// Where a text box or shape goes (docx-agent's `_position`): `at` as given, or the end
/// of the text `find` names once.
pub(super) fn position(args: &WordDrawings, editing: &Editing<'_>, list: &TryCall) -> Result<String, ToolError> {
    let key = one_of(&[("at", args.at.is_some()), ("find", args.find.is_some())], list)?;
    if key == "find" {
        let range = find_once(editing, args.find.as_deref().unwrap_or(""), list)?;
        return Ok(end_of(&range));
    }
    Ok(args.at.clone().unwrap_or_default())
}

/// The pictures, then the charts not among them, as docx-agent lists them.
fn listed(editing: &Editing<'_>, list: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    let head = editing.editor.current();
    let docx = head
        .doc()
        .docx()
        .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
    let mut seen: Vec<Drawing> = pictures(&docx);
    for chart in graphics(&docx).into_iter().filter(|d| d.kind == "chart") {
        if !seen.iter().any(|d| d.id == chart.id) {
            seen.push(chart);
        }
    }
    let mut rows = Vec::new();
    for drawing in &seen {
        if drawing.member {
            // docx-agent's `Member` has no size: its listing fails
            return Ok(not_supported("word_drawings list of a chart in a group", list));
        }
        let extent = drawing.element.find(Some(WP), "extent");
        let size = |k: &str| {
            let v: f64 = extent
                .and_then(|e| e.attribute(None, k))
                .and_then(|t| t.trim().parse().ok())
                .unwrap_or(0.0);
            round2(v / 12700.0)
        };
        let mut row = Map::new();
        row.insert("id".into(), json!(drawing.id));
        // what the frame holds (a group holding a picture is listed among the pictures, as a group)
        row.insert("kind".into(), json!(drawing_kind(&drawing.element)));
        row.insert("size".into(), json!([size("cx"), size("cy")]));
        row.insert("inline".into(), json!(drawing.element.name().is(Some(WP), "inline")));
        if let Some(alt) = drawing.alt_text().filter(|a| !a.is_empty()) {
            row.insert("alt_text".into(), json!(short(alt, 80)));
        }
        rows.push(Value::Object(row));
    }
    let mut done = Done::new(format!("{} drawing(s)", rows.len()));
    done.data = Some(Value::Array(rows));
    Ok(ToolResult::done(done))
}

/// Registers `word_drawings` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<WordDrawings>(word_drawings, crate::Hosting::PLAIN.blobs());
}
