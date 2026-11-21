//! Owns: `word_template` (docx-agent's `tools/style.py`, `word_template`): its definition
//! (byte-equal to today's) and its handler: Word's Convert through the edit core's
//! [`UpgradeToModern`] (one history step, never tracked), the legacy options it removed
//! said, and the reflow between the version it was applied to and the one it made, from
//! the host's layout through the layout port ([`ooxml_word_edit::FieldLayout::reflow`]).
//!
//! Must not: lay out but through the port, or change the document but through the command.

use crate::review::{edit_refusal, Editing};
use crate::structure::common::{call, outcome, refused, Outcome};
use ooxml_tools::{ArgEnum, ToolArgs, ToolResult};
use ooxml_word_edit::{compatibility_mode, EditError, PageReflow, UpgradeToModern, MODERN_MODE};
use serde_json::{json, Map, Value};

/// What `word_template` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateAction {
    /// Word's Convert, to compatibility mode 15.
    UpgradeToModern,
}

/// `word_template`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_template",
    description = include_str!("word_template.md"),
    group = "word_style",
    mutates,
    batchable = false
)]
pub struct WordTemplate {
    /// Document id.
    pub doc: String,
    /// What to do.
    pub action: TemplateAction,
}

/// The blocks whose first page moved that a reply lists.
const MOVED_SHOWN: usize = 20;

/// A reflow as docx-agent's tools report it (`reflow_json`): the pages changed, the page
/// counts, and where present the first blocks that moved and the pages not compared.
fn reflow_json(reflow: &PageReflow) -> Value {
    let mut out = Map::new();
    out.insert("changed".into(), json!(reflow.changed));
    out.insert("pages".into(), json!([reflow.page_count.0, reflow.page_count.1]));
    if !reflow.moved.is_empty() {
        let moved: Map<String, Value> = reflow
            .moved
            .iter()
            .take(MOVED_SHOWN)
            .map(|(id, (was, now))| (id.clone(), json!([was, now])))
            .collect();
        out.insert("moved".into(), Value::Object(moved));
    }
    if !reflow.unknown.is_empty() {
        out.insert("unknown".into(), json!(reflow.unknown));
    }
    Value::Object(out)
}

/// `word_template` (docx-agent's): the document brought into compatibility mode 15, with
/// the reflow; a document already in it is left as it is. Without the host's layout, a
/// document to convert is left to the host (`unported`).
#[must_use]
pub fn word_template(args: &WordTemplate, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let again = call(
        "word_template",
        &[("doc", json!(args.doc)), ("action", json!(args.action.as_str()))],
    );
    let head = editing.editor.store().snapshot();
    let modern = match compatibility_mode(head.doc()) {
        Ok(mode) => mode >= MODERN_MODE,
        Err(error) => return ToolResult::failed(edit_refusal(error, "", again)),
    };
    if editing.layout.is_none() && !modern {
        let why = EditError::Unsupported("the reflow without the layout".to_owned());
        return refused(why, again);
    }
    converted(editing, &again, editing.layout)
}

/// Convert run, and its reply with the reflow `layout` reports.
fn converted(
    editing: &Editing<'_>,
    again: &ooxml_tools::TryCall,
    layout: Option<&dyn ooxml_word_edit::FieldLayout>,
) -> ToolResult<Value> {
    let done = match editing.editor.run(editing.caller, &UpgradeToModern) {
        Ok(done) => done,
        Err(error) => return ToolResult::failed(edit_refusal(error, "", again.clone())),
    };
    let mode = compatibility_mode(done.version.doc()).unwrap_or(MODERN_MODE);
    let mut warnings: Vec<String> = done
        .out
        .removed
        .iter()
        .map(|name| format!("removed compatibility option {name}"))
        .collect();
    let reflow = match (&done.previous, layout) {
        (Some(previous), Some(layout)) if done.changed => {
            match layout.reflow(previous.doc().package(), done.version.doc().package()) {
                Ok(found) => reflow_json(&found),
                Err(why) => {
                    warnings.push(format!("the reflow ({why}): the layout cannot say"));
                    Value::Null
                }
            }
        }
        _ => Value::Null,
    };
    let step = Outcome {
        changed: done.changed,
        warnings,
        ..Outcome::default()
    };
    outcome(
        &[step],
        format!("Compatibility mode {mode}"),
        Some(json!({"mode": mode, "reflow": reflow})),
    )
}

/// Registers `word_template` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<WordTemplate>(word_template, crate::Hosting::SERVED.layout());
}
