//! `word-edits <manifest.json>`: Word edits made by the Rust core, as JSON in the form
//! `python/word_edits_diff.py` writes docx-agent's: per case, the edit's result (the
//! paragraph's id, whether it changed, the renames), every part whose content changed
//! (its bytes as the package writes them), the archive's part order, `touched`, the
//! validation delta, and the next id each of two seeds would give.
//!
//! The manifest is a JSON list of `{"file": path, "cases": [{"target", "text", "tracked":
//! {"author", "date"} or null}]}`; each case runs on the document as opened.
//!
//! Must not: be used by any library; it is a test instrument.

use ooxml_base::Limits;
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, StoreConfig};
use ooxml_word_edit::touched::Touched;
use ooxml_word_edit::validate::problems;
use ooxml_word_edit::{Edited, Editor, RevisionPolicy, SetText, TextEdited, Tracking, WordDoc};
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
    let command = SetText {
        target: case.get("target").and_then(Value::as_str).unwrap_or("").to_owned(),
        text: case.get("text").and_then(Value::as_str).unwrap_or("").to_owned(),
        policy: policy(case),
    };
    match editor.run(&Caller::anonymous(), &command) {
        Ok(done) => Ok(report(&package, &done)),
        Err(error) => Ok(json!({"refused": error.to_string()})),
    }
}

/// What a case reports.
fn report(opened: &Package, done: &Edited<TextEdited>) -> Value {
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
    let renamed: Map<String, Value> = done
        .out
        .renamed
        .iter()
        .map(|(old, new)| (old.clone(), json!(new)))
        .collect();
    let main = doc.docx().map(|d| d.main.clone()).unwrap_or_default();
    let next: Map<String, Value> = [main.clone(), format!("{main}\0text")]
        .into_iter()
        .map(|seed| {
            let values = doc.next_ids(&seed, 1);
            (seed, json!(values))
        })
        .collect();
    json!({
        "result": {"id": done.out.id, "changed": done.out.changed, "renamed": renamed},
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
