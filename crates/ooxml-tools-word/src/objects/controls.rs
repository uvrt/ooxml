//! Owns: `word_controls`' handler (docx-agent's `tools/objects.py`, `word_controls`): the
//! content controls listed (id, kind, title, tag, value: `ContentControl.value`); one
//! inserted at a position or after text found once, filled as typing would (a check box
//! with true or false), or removed keeping its content -- each one command -- and the reply.
//!
//! Must not: change the document but through a command.

use super::drawings_args::{ControlAction, ControlType, WordControls};
use crate::review::Editing;
use crate::structure::common::{call, find_once, need, one_of, outcome, policy, refused, Outcome};
use crate::structure::notes::end_of;
use ooxml_tools::{Done, Failure, Refusal, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{ControlEdited, ControlKind, FillControl, Filling, InsertControl, RemoveControl};
use ooxml_word_model::address::resolve::control;
use ooxml_word_model::docx::controls::{controls, Control};
use ooxml_word_model::docx::Docx;
use serde_json::{json, Value};

impl From<ControlEdited> for Outcome {
    fn from(out: ControlEdited) -> Self {
        Outcome {
            changed: out.id.is_some(),
            id: out.id,
            created: out.created,
            removed: out.removed,
            renamed: out.renamed,
            warnings: out.warnings,
        }
    }
}

/// `word_controls` (docx-agent's `word_controls`): content controls listed, inserted,
/// filled or removed.
#[must_use]
pub fn word_controls(args: &WordControls, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let list = call("word_controls", &[("doc", json!(args.doc)), ("action", json!("list"))]);
    match run(args, editing, &list) {
        Ok(result) => result,
        Err(error) => ToolResult::failed(error),
    }
}

fn run(args: &WordControls, editing: &Editing<'_>, list: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    let editor = editing.editor;
    let reply = |result: Result<ooxml_word_edit::Edited<ControlEdited>, ooxml_word_edit::EditError>,
                 what: &dyn Fn(&ControlEdited) -> (String, Option<Value>)| match result {
        Ok(done) => {
            let (summary, data) = what(&done.out);
            outcome(&[done.out.into()], summary, data)
        }
        Err(error) => refused(error, list.clone()),
    };
    match args.action {
        ControlAction::List => listed(editing),
        ControlAction::Insert => {
            let command = InsertControl {
                at: position(args, editing, list)?,
                kind: kind(args.kind),
                items: args.items.clone().unwrap_or_default(),
                tag: args.tag.clone(),
                alias: args.title.clone(),
                policy: policy(editing),
            };
            Ok(reply(editor.run(editing.caller, &command), &|out| {
                let id = out.id.clone().unwrap_or_default();
                (format!("Inserted {id}"), Some(json!({"id": id})))
            }))
        }
        ControlAction::Remove => {
            let target = need(args.control.as_deref(), "control", list)?.to_owned();
            let command = RemoveControl {
                control: target.clone(),
                policy: policy(editing),
            };
            Ok(reply(editor.run(editing.caller, &command), &|_| {
                (format!("Removed {target}"), None)
            }))
        }
        ControlAction::Fill => {
            let target = need(args.control.as_deref(), "control", list)?.to_owned();
            let raw = need(args.value.as_deref(), "value", list)?;
            let value = if kind_of(editing, &target, list)? == "checkbox" {
                let said = raw.trim().to_lowercase();
                Filling::Checked(matches!(said.as_str(), "true" | "yes" | "1" | "checked"))
            } else {
                Filling::Text(raw.to_owned())
            };
            let command = FillControl {
                control: target.clone(),
                value,
                policy: policy(editing),
            };
            Ok(reply(editor.run(editing.caller, &command), &|_| {
                (format!("Filled {target}"), None)
            }))
        }
    }
}

/// The edit core's kind of control `given` (text when none is).
fn kind(given: Option<ControlType>) -> ControlKind {
    match given {
        None | Some(ControlType::Text) => ControlKind::Text,
        Some(ControlType::RichText) => ControlKind::RichText,
        Some(ControlType::DropDown) => ControlKind::DropDown,
        Some(ControlType::ComboBox) => ControlKind::ComboBox,
        Some(ControlType::Date) => ControlKind::Date,
        Some(ControlType::Checkbox) => ControlKind::Checkbox,
    }
}

/// Where a control goes (docx-agent's `_position`): `at` as given, or the end of the text
/// `find` names once.
fn position(args: &WordControls, editing: &Editing<'_>, list: &TryCall) -> Result<String, ToolError> {
    let key = one_of(&[("at", args.at.is_some()), ("find", args.find.is_some())], list)?;
    if key == "find" {
        let range = find_once(editing, args.find.as_deref().unwrap_or(""), list)?;
        return Ok(end_of(&range));
    }
    Ok(args.at.clone().unwrap_or_default())
}

/// The kind of the control `id` names (`content_control(...).kind`); `not_found` for none.
fn kind_of(editing: &Editing<'_>, id: &str, list: &TryCall) -> Result<&'static str, ToolError> {
    let docx = head_docx(editing)?;
    let found = control(&controls(&docx), id)
        .map_err(|e| ToolError::refusal(Refusal::NotFound, e.to_string(), WayOut::call(list.clone())))?;
    Ok(found.kind())
}

fn head_docx(editing: &Editing<'_>) -> Result<std::sync::Arc<Docx>, ToolError> {
    let head = editing.editor.store().snapshot();
    head.doc()
        .docx()
        .map_err(|e| ToolError::internal(e.to_string(), "word:model"))
}

/// Every content control (a hundred), as docx-agent lists them.
fn listed(editing: &Editing<'_>) -> Result<ToolResult<Value>, ToolError> {
    let docx = head_docx(editing)?;
    let mut rows = Vec::new();
    for found in controls(&docx).into_iter().take(100) {
        let value = value_of(&docx, &found)?;
        rows.push(json!({
            "id": found.id,
            "kind": found.kind(),
            "title": found.alias(),
            "tag": found.tag(),
            "value": value,
        }));
    }
    let mut done = Done::new("Content controls");
    done.data = Some(Value::Array(rows));
    Ok(ToolResult::done(done))
}

/// What a control holds (docx-agent's `control_value`, as `_plain` gives it): a check
/// box's state, a date control's date, a list's chosen item's value where its text is an
/// item's, else the text.
fn value_of(docx: &Docx, found: &Control) -> Result<Value, ToolError> {
    let kind = found.kind();
    if kind == "checkbox" {
        return Ok(json!(found.checked()));
    }
    let Some(part) = docx.part(&found.part) else {
        return Ok(Value::Null);
    };
    if kind == "date" {
        let full = ooxml_word_model::docx::controls::property(&found.element, ooxml_word_model::ids::W, "date")
            .and_then(|d| d.attribute(Some(ooxml_word_model::ids::W), "fullDate"))
            .filter(|f| !f.is_empty());
        return match full {
            None => Ok(Value::Null),
            Some(full) => {
                let day: String = full.chars().take(10).collect();
                ooxml_word_edit::Day::parse(&day).map_or_else(
                    || {
                        Err(ToolError::failure(
                            Failure::Unit,
                            format!("Invalid isoformat string: {}", ooxml_base::pyfmt::repr_str(&day)),
                        ))
                    },
                    |_| Ok(json!(day)),
                )
            }
        };
    }
    let text = found.text(part);
    if matches!(kind, "drop-down" | "combo-box") {
        if let Some((_, value)) = found.items().into_iter().find(|(d, _)| *d == text) {
            return Ok(json!(value));
        }
    }
    Ok(json!(text))
}

/// Registers `word_controls` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<WordControls>(word_controls, crate::Hosting::PLAIN);
}
