//! The review tools' cases of `word-edits` (`op: "tool"`): `word_set_tracking`,
//! `word_changes` and `word_comments` called with JSON arguments on a document as opened,
//! each reply as the tool writes it (`to_json`) and every part whose content changed (its
//! bytes as the package writes them), for `python/word_edits_diff.py --tools` to compare
//! with docx-agent's toolbox.
//!
//! Must not: be used by any library; it is a test instrument.

use crate::edits::{Monotonic, Unlimited};
use ooxml_base::{Cancel, Limits};
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ArgType;
use ooxml_tools_word::{
    word_changes, word_comments, word_set_tracking, Editing, WordChanges, WordComments, WordSetTracking,
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
    let editing = Editing {
        editor: &editor,
        caller: &caller,
        opened: &opened,
        tracking: None,
        doc: "d1",
        cancel: &Never,
    };
    let arguments = case.get("arguments").cloned().unwrap_or(Value::Object(Map::new()));
    let tool = case.get("tool").and_then(Value::as_str).unwrap_or("");
    let reply = match tool {
        "word_changes" => word_changes(
            &WordChanges::from_arg(&arguments, "").map_err(|e| e.to_string())?,
            &editing,
        ),
        "word_comments" => word_comments(
            &WordComments::from_arg(&arguments, "").map_err(|e| e.to_string())?,
            &editing,
        ),
        "word_set_tracking" => {
            word_set_tracking(
                &WordSetTracking::from_arg(&arguments, "").map_err(|e| e.to_string())?,
                &editing,
            )
            .0
        }
        other => return Err(format!("no tool {other}")),
    };
    let head = editor.store().snapshot();
    let doc = head.doc();
    let mut parts = Map::new();
    for name in doc.package().part_names() {
        let same = matches!((package.part(&name), doc.package().part(&name)), (Some(a), Some(b)) if a.same(b));
        if same {
            continue;
        }
        let bytes = doc
            .package()
            .read(&name)
            .ok()
            .flatten()
            .unwrap_or_else(|| Arc::from(Vec::new()));
        parts.insert(name, json!(String::from_utf8_lossy(&bytes)));
    }
    Ok(json!({
        "reply": Value::Object(reply.to_json()),
        "unported": reply.unported(),
        "parts": parts,
        "order": doc.package().part_names(),
    }))
}
