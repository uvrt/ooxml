//! The structure tools' cases of `word-edits` (`op: "structure"`): `word_links` (and the
//! other structure tools as they are ported) called with JSON arguments on a document as
//! opened -- its arguments taken as the tool's intent rules take them -- each reply as the
//! tool writes it, every part whose content changed, the archive's part order, `touched`,
//! the validation delta, the problems and the next ids, for
//! `python/word_edits_diff.py --structure` to compare with docx-agent's toolbox.
//!
//! Must not: be used by any library; it is a test instrument.

use crate::edits::{Monotonic, Unlimited};
use ooxml_base::{Cancel, Limits};
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::{ToolArgs, ToolResult};
use ooxml_tools_word::{
    word_headers_footers, word_links, word_move, word_notes, word_sections, Editing, WordHeadersFooters, WordLinks,
    WordMove, WordNotes, WordSections,
};
use ooxml_word_edit::touched::{touched, Touched};
use ooxml_word_edit::validate::{problems, Delta};
use ooxml_word_edit::{Editor, Tracking, WordDoc};
use serde_json::{json, Map, Value};
use std::sync::Arc;

struct Never;

impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// One structure tool case on a freshly opened document.
///
/// # Errors
/// A message when the document cannot be opened.
pub fn case(bytes: &Arc<[u8]>, case: &Value) -> Result<Value, String> {
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
    let tracking = case.get("tracked").and_then(Value::as_object).map(|t| Tracking {
        author: t.get("author").and_then(Value::as_str).unwrap_or("").to_owned(),
        date: t.get("date").and_then(Value::as_str).unwrap_or("").to_owned(),
    });
    let editing = Editing {
        editor: &editor,
        caller: &caller,
        opened: &opened,
        tracking: tracking.as_ref(),
        doc: "d1",
        cancel: &Never,
    };
    let arguments = case.get("arguments").cloned().unwrap_or(Value::Object(Map::new()));
    let tool = case.get("tool").and_then(Value::as_str).unwrap_or("");
    let reply = match tool {
        "word_links" => run(&arguments, |args: &WordLinks| word_links(args, &editing)),
        "word_notes" => run(&arguments, |args: &WordNotes| word_notes(args, &editing)),
        "word_move" => run(&arguments, |args: &WordMove| word_move(args, &editing)),
        "word_sections" => run(&arguments, |args: &WordSections| word_sections(args, &editing)),
        "word_headers_footers" => run(&arguments, |args: &WordHeadersFooters| {
            word_headers_footers(args, &editing)
        }),
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
    let found = touched(&opened, doc).map_err(|e| e.to_string())?;
    let now = problems(doc).map_err(|e| e.to_string())?;
    let delta = Delta::of(editor.baseline(), &now);
    Ok(json!({
        "reply": Value::Object(reply.to_json()),
        "unported": reply.unported(),
        "parts": parts,
        "order": doc.package().part_names(),
        "touched": found.iter().map(touched_json).collect::<Vec<_>>(),
        "validation": {"new": delta.new, "fixed": delta.fixed, "baseline": delta.baseline},
        "problems": now.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "next": next_ids(doc),
    }))
}

/// The tool's reply to `arguments` taken as its intent rules take them (a refusal of the
/// arguments is its reply), the notes on what was read as what first among its warnings.
fn run<A: ToolArgs>(arguments: &Value, handler: impl FnOnce(&A) -> ToolResult<Value>) -> ToolResult<Value> {
    match A::accept(arguments.clone(), Some("docx")) {
        Ok((args, notes)) => {
            let mut reply = handler(&args);
            if !notes.is_empty() {
                let mut warnings = notes;
                warnings.append(&mut reply.warnings);
                reply.warnings = warnings;
            }
            reply
        }
        Err(error) => ToolResult::failed(error),
    }
}

fn touched_json(touched: &Touched) -> Value {
    match &touched.section {
        None => json!({"part": touched.part}),
        Some(s) => json!({"part": touched.part, "heading": s.heading, "level": s.level, "blocks": s.blocks}),
    }
}

/// The value the main part's sequence and its text-id sequence give next.
fn next_ids(doc: &WordDoc) -> Map<String, Value> {
    let main = doc.docx().map(|d| d.main.clone()).unwrap_or_default();
    [main.clone(), format!("{main}\0text")]
        .into_iter()
        .map(|seed| {
            let values = doc.next_ids(&seed, 1);
            (seed, json!(values))
        })
        .collect()
}
