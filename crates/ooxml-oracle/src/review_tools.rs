//! The tool cases of `word-edits` (`op: "tool"`): `word_set_tracking`, `word_changes`,
//! `word_comments`, `word_set_text`, `replace_text`, `word_insert_text` and `word_delete` called with JSON arguments on a document as opened,
//! each reply as the tool writes it (`to_json`) and every part whose content changed (its
//! bytes as the package writes them), for `python/word_edits_diff.py --tools` to compare
//! with docx-agent's toolbox.
//!
//! Must not: be used by any library; it is a test instrument.

use crate::edits::{Monotonic, Unlimited};
use ooxml_base::{Cancel, Limits};
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::{ArgType, ReadsWhen, ToolArgs};
use ooxml_tools_word::{
    replace_text, word_changes, word_changes_read, word_comments, word_delete, word_insert_markdown, word_insert_text,
    word_set_text, word_set_tracking, CallEditor, Editing, ReplaceText, WordChanges, WordComments, WordDelete,
    WordInsertMarkdown, WordInsertText, WordSetText, WordSetTracking,
};
use ooxml_word_edit::Editor;
use serde_json::{json, Map, Value};
use std::sync::Arc;

struct Never;

impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// One tool case on a freshly opened document: the reply and the parts that changed.
///
/// # Errors
/// A message when the document cannot be opened or the arguments read.
pub fn tool_case(bytes: &Arc<[u8]>, case: &Value) -> Result<Value, String> {
    let package = Package::open(Arc::clone(bytes), &Limits::default()).map_err(|e| e.to_string())?;
    let editor = Editor::open(
        package.clone(),
        StoreConfig::default(),
        Arc::new(Monotonic::new()),
        Arc::new(Unlimited),
    )
    .map_err(|e| e.to_string())?;
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let call_editor = CallEditor::new(&editor);
    let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-17T10:00:00Z");
    let arguments = case.get("arguments").cloned().unwrap_or(Value::Object(Map::new()));
    let tool = case.get("tool").and_then(Value::as_str).unwrap_or("");
    let (arguments, mut notes) = match accepted(tool, arguments) {
        Ok(taken) => taken,
        Err(mut reply) => {
            // nothing changed: the parts in the order the document has them
            if let Some(map) = reply.as_object_mut() {
                map.insert("order".into(), json!(package.part_names()));
            }
            return Ok(reply);
        }
    };
    let mut reply = run(tool, &arguments, &editing)?;
    notes.append(&mut reply.warnings);
    reply.warnings = notes;
    let head = editor.store().snapshot();
    let doc = head.doc();
    let mut parts = Map::new();
    // as saved: a part read with the ids given at open and not written is the input's
    for name in doc.package().part_names() {
        let same = matches!((package.part(&name), doc.package().part(&name)), (Some(a), Some(b)) if a.same(b.saved()));
        if same {
            continue;
        }
        let bytes = doc
            .package()
            .read_saved(&name)
            .ok()
            .flatten()
            .unwrap_or_else(|| Arc::from(Vec::new()));
        crate::part_texts::insert_part(&mut parts, name, &bytes);
    }
    Ok(json!({
        "reply": Value::Object(reply.to_json()),
        "not_supported": reply.not_supported_form(),
        "parts": parts,
        "order": doc.package().part_names(),
    }))
}

/// `tool`'s handler run with `arguments` (taken as meant).
fn run(tool: &str, arguments: &Value, editing: &Editing<'_>) -> Result<ooxml_tools::ToolResult<Value>, String> {
    Ok(match tool {
        "word_changes" => {
            let args = WordChanges::from_arg(arguments, "").map_err(|e| e.to_string())?;
            if args.reads() {
                // list and preview as a session serves them: on the version the call reads
                let last_check = ooxml_tools_word::LastCheck::default();
                let version = crate::structure_tools::opened_version(editing, &last_check);
                match word_changes_read(&args, &version, editing.cancel) {
                    Ok(done) => ooxml_tools::ToolResult::done(done),
                    Err(error) => ooxml_tools::ToolResult::failed(error),
                }
            } else {
                word_changes(&args, editing)
            }
        }
        "word_comments" => word_comments(
            &WordComments::from_arg(arguments, "").map_err(|e| e.to_string())?,
            editing,
        ),
        "word_set_tracking" => {
            word_set_tracking(
                &WordSetTracking::from_arg(arguments, "").map_err(|e| e.to_string())?,
                editing,
            )
            .0
        }
        "word_set_text" => word_set_text(
            &WordSetText::from_arg(arguments, "").map_err(|e| e.to_string())?,
            editing,
        ),
        "replace_text" => replace_text(
            &ReplaceText::from_arg(arguments, "").map_err(|e| e.to_string())?,
            editing,
        ),
        "word_insert_text" => word_insert_text(
            &WordInsertText::from_arg(arguments, "").map_err(|e| e.to_string())?,
            editing,
        ),
        "word_delete" => word_delete(
            &WordDelete::from_arg(arguments, "").map_err(|e| e.to_string())?,
            editing,
        ),
        "word_insert_markdown" => word_insert_markdown(
            &WordInsertMarkdown::from_arg(arguments, "").map_err(|e| e.to_string())?,
            editing,
        ),
        other => return Err(format!("no tool {other}")),
    })
}

/// The text tools' arguments taken as meant, as the session takes them (defaults filled),
/// with the notes saying so; a refusal as the case's reply.
fn accepted(tool: &str, arguments: Value) -> Result<(Value, Vec<String>), Value> {
    let spec = match tool {
        "word_set_text" => WordSetText::spec(),
        "replace_text" => ReplaceText::spec(),
        "word_insert_text" => WordInsertText::spec(),
        "word_delete" => WordDelete::spec(),
        "word_insert_markdown" => WordInsertMarkdown::spec(),
        _ => return Ok((arguments, Vec::new())),
    };
    match spec.accept(arguments) {
        Ok(accepted) => Ok((Value::Object(accepted.arguments), accepted.notes)),
        Err(error) => Err(json!({"reply": {"ok": false, "error": error.to_json()}, "parts": {}, "order": []})),
    }
}
