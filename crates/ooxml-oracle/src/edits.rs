//! `word-edits <manifest.json>`: Word edits made by the Rust core, as JSON in the form
//! `python/word_edits_diff.py` writes docx-agent's: per case, the edit's result (the
//! paragraph's id, whether it changed, the renames), every part whose content changed
//! (its bytes as the package writes them), the archive's part order, `touched`, the
//! validation delta, and the next id each of two seeds would give.
//!
//! The manifest is a JSON list of `{"file": path, "cases": [{"op", "target", "text",
//! "tracked": {"author", "date"} or null, ...}]}` (`op`: `set_text` when absent,
//! `set_text_paragraphs` (set_text in lines), `insert_paragraph` with `after` and `style`,
//! `set_paragraph_style` with `style` (null: the default),
//! `insert_text`, `delete_text`, `replace` with `find`, `case`, `count`, `keep`); each case
//! runs on the document as opened.
//!
//! Must not: be used by any library; it is a test instrument.

use ooxml_base::Limits;
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, StoreConfig};
use ooxml_word_edit::find::{Literal, Pattern};
use ooxml_word_edit::touched::Touched;
use ooxml_word_edit::validate::problems;
use ooxml_word_edit::{
    DeleteText, EditError, Edited, Editor, InsertParagraph, InsertText, Keep, RangesEdited, ReplaceText,
    RevisionPolicy, SetParagraphStyle, SetText, TextEdited, Tracking, WordDoc,
};
use serde_json::{json, Map, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The process's monotonic clock.
pub struct Monotonic(Instant);

impl Monotonic {
    /// A clock whose origin is now.
    #[must_use]
    pub fn new() -> Self {
        Monotonic(Instant::now())
    }
}

impl Default for Monotonic {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for Monotonic {
    fn monotonic(&self) -> Duration {
        self.0.elapsed()
    }
}

/// A budget with no limit.
pub struct Unlimited;

impl CacheBudget for Unlimited {
    fn reserve(&self, _bytes: usize) -> bool {
        true
    }
    fn release(&self, _bytes: usize) {}
    fn used(&self) -> usize {
        0
    }
}

/// Run over a manifest: one JSON line per case on stdout.
///
/// # Errors
/// A message when the manifest or a document cannot be read.
pub fn run(args: &[String]) -> Result<(), String> {
    let manifest = args.first().ok_or("usage: word-edits <manifest.json>")?;
    let text = std::fs::read_to_string(manifest).map_err(|e| e.to_string())?;
    let items: Vec<Value> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    for item in items {
        let file = item
            .get("file")
            .and_then(Value::as_str)
            .ok_or("an item without a file")?;
        let bytes: Arc<[u8]> = Arc::from(std::fs::read(file).map_err(|e| format!("{file}: {e}"))?);
        let cases = item.get("cases").and_then(Value::as_array).cloned().unwrap_or_default();
        for (k, case) in cases.iter().enumerate() {
            let out = edit(&bytes, case).unwrap_or_else(|e| json!({"error": e}));
            println!("{}", json!({"file": file, "case": k, "rust": out}));
        }
    }
    Ok(())
}

/// The policy a case asks for.
fn policy(case: &Value) -> RevisionPolicy {
    match case.get("tracked").filter(|t| !t.is_null()) {
        Some(t) => RevisionPolicy::Tracked(Tracking {
            author: t
                .get("author")
                .and_then(Value::as_str)
                .unwrap_or("docx-agent")
                .to_owned(),
            date: t.get("date").and_then(Value::as_str).unwrap_or("").to_owned(),
        }),
        None => RevisionPolicy::Untracked,
    }
}

/// One case on a freshly opened document.
fn edit(bytes: &Arc<[u8]>, case: &Value) -> Result<Value, String> {
    let package = Package::open(Arc::clone(bytes), &Limits::default()).map_err(|e| e.to_string())?;
    let editor = Editor::open(
        package.clone(),
        StoreConfig::default(),
        Arc::new(Monotonic::new()),
        Arc::new(Unlimited),
    )
    .map_err(|e| e.to_string())?;
    let text = |key: &str| case.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
    let caller = Caller::anonymous();
    let done: Result<Value, EditError> = match case.get("op").and_then(Value::as_str).unwrap_or("set_text") {
        "insert_text" => editor
            .run(
                &caller,
                &InsertText {
                    at: text("target"),
                    text: text("text"),
                    policy: policy(case),
                },
            )
            .map(|done| report(&package, &done, ranges_result(&done.out))),
        "delete_text" => editor
            .run(
                &caller,
                &DeleteText {
                    range: text("target"),
                    policy: policy(case),
                },
            )
            .map(|done| report(&package, &done, ranges_result(&done.out))),
        "insert_paragraph" => insert_paragraph(&editor, &caller, &package, case),
        "set_paragraph_style" => set_paragraph_style(&editor, &caller, &package, case),
        "replace" => {
            let pattern = match pattern(case, &text("find"), &text("text")) {
                Ok(p) => p,
                Err(error) => return Ok(json!({"refused": error})),
            };
            let command = ReplaceText {
                pattern,
                across: case.get("across").and_then(Value::as_bool).unwrap_or(false),
                count: case
                    .get("count")
                    .and_then(Value::as_u64)
                    .and_then(|c| usize::try_from(c).ok()),
                keep: if text("keep") == "characters" {
                    Keep::Characters
                } else {
                    Keep::First
                },
                policy: policy(case),
            };
            editor
                .run(&caller, &command)
                .map(|done| report(&package, &done, ranges_result(&done.out)))
        }
        op => {
            let lines = op == "set_text_paragraphs";
            let command = SetText {
                target: text("target"),
                text: text("text"),
                policy: policy(case),
            };
            editor
                .run(&caller, &command)
                .map(|done| report(&package, &done, text_result(&done.out, lines)))
        }
    };
    Ok(done.unwrap_or_else(|error| json!({"refused": error.to_string()})))
}

/// An insert_paragraph case.
fn insert_paragraph(editor: &Editor, caller: &Caller, package: &Package, case: &Value) -> Result<Value, EditError> {
    let text = |key: &str| case.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
    let command = InsertParagraph {
        anchor: text("target"),
        after: case.get("after").and_then(Value::as_bool).unwrap_or(true),
        text: text("text"),
        style: case.get("style").and_then(Value::as_str).map(str::to_owned),
        policy: policy(case),
    };
    editor.run(caller, &command).map(|done| {
        let out = &done.out;
        let result = json!({"id": out.id, "changed": true, "renamed": renamed(&out.renamed), "created": [out.id]});
        report(package, &done, result)
    })
}

/// A set_paragraph_style case.
fn set_paragraph_style(editor: &Editor, caller: &Caller, package: &Package, case: &Value) -> Result<Value, EditError> {
    let command = SetParagraphStyle {
        target: case.get("target").and_then(Value::as_str).unwrap_or("").to_owned(),
        style: case.get("style").and_then(Value::as_str).map(str::to_owned),
        policy: policy(case),
    };
    editor.run(caller, &command).map(|done| {
        let result = json!({"id": done.out.id, "renamed": renamed(&done.out.renamed)});
        report(package, &done, result)
    })
}

/// Renames as the harness writes them.
fn renamed(renamed: &[(String, String)]) -> Map<String, Value> {
    renamed.iter().map(|(old, new)| (old.clone(), json!(new))).collect()
}

/// A replace's find: plain text (white space collapsed or not), or a Python regular
/// expression matched by the tools' translation, its template expanded.
fn pattern(case: &Value, find: &str, with: &str) -> Result<Arc<dyn Pattern>, String> {
    let sensitive = case.get("case").and_then(Value::as_bool).unwrap_or(true);
    if case.get("regex").and_then(Value::as_bool).unwrap_or(false) {
        return crate::find_regex::Regex::new(find, !sensitive, with).map(|r| Arc::new(r) as Arc<dyn Pattern>);
    }
    let collapse = case.get("whitespace").and_then(Value::as_str) == Some("collapse");
    Literal::new(find, sensitive, collapse, with)
        .map(|l| Arc::new(l) as Arc<dyn Pattern>)
        .map_err(|e| e.to_string())
}

/// A set_text's result, as the harness writes docx-agent's.
fn text_result(out: &TextEdited, lines: bool) -> Value {
    let renamed: Map<String, Value> = out.renamed.iter().map(|(old, new)| (old.clone(), json!(new))).collect();
    if lines {
        return json!({"id": out.id, "changed": out.changed, "renamed": renamed, "created": out.created});
    }
    json!({"id": out.id, "changed": out.changed, "renamed": renamed})
}

/// A range edit's result.
fn ranges_result(out: &RangesEdited) -> Value {
    let renamed: Map<String, Value> = out.renamed.iter().map(|(old, new)| (old.clone(), json!(new))).collect();
    json!({"id": out.id, "count": out.count, "renamed": renamed})
}

/// What a case reports.
fn report<O>(opened: &Package, done: &Edited<O>, result: Value) -> Value {
    let doc: &WordDoc = done.version.doc();
    let mut parts = Map::new();
    for name in doc.package().part_names() {
        let same = matches!((opened.part(&name), doc.package().part(&name)), (Some(a), Some(b)) if a.same(b));
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
    let main = doc.docx().map(|d| d.main.clone()).unwrap_or_default();
    let next: Map<String, Value> = [main.clone(), format!("{main}\0text")]
        .into_iter()
        .map(|seed| {
            let values = doc.next_ids(&seed, 1);
            (seed, json!(values))
        })
        .collect();
    json!({
        "result": result,
        "parts": parts,
        "order": doc.package().part_names(),
        "touched": done.touched.iter().map(touched_json).collect::<Vec<_>>(),
        "validation": {"new": done.validation.new, "fixed": done.validation.fixed,
                       "baseline": done.validation.baseline},
        "problems": problems(doc).map(|p| p.iter().map(ToString::to_string).collect::<Vec<_>>())
            .unwrap_or_default(),
        "next": next,
    })
}

fn touched_json(touched: &Touched) -> Value {
    match &touched.section {
        None => json!({"part": touched.part}),
        Some(s) => json!({"part": touched.part, "heading": s.heading, "level": s.level, "blocks": s.blocks}),
    }
}
