//! Owns: the field update's part of the Word layout's differential -- what docx-agent's
//! `word_fields` update writes, read from docx2svg's layout as it reads it (the records'
//! `updates` and `references`, `python/docx2svg/tools/layout_differential.py`), against
//! the Rust layout's page numbers (`ooxml_word_layout::fields::update::PageNumbers`).
//!
//! Must not: change what is compared to make it pass.

use ooxml_word_layout::fields::argument;
use ooxml_word_layout::fields::update::PageNumbers;
use ooxml_word_layout::DocumentLayout;
use ooxml_word_model::model::Paragraph;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// What the update comparison found.
#[derive(Debug, Default)]
pub struct Updates {
    /// Results compared.
    pub compared: usize,
    /// Each difference: where, and the two results.
    pub differences: Vec<(String, String)>,
}

/// The paragraph a bookmark starts in, the first of a name winning.
fn bookmark_paths(paragraphs: &[Arc<Paragraph>]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for paragraph in paragraphs {
        for mark in paragraph.bookmarks.iter() {
            if let (true, Some(name)) = (mark.start, mark.name.as_deref().filter(|n| !n.is_empty())) {
                out.entry(name.to_owned()).or_insert_with(|| paragraph.path.to_string());
            }
        }
    }
    out
}

/// Compare the update records of `theirs` (a document's layout record) with `layout`.
pub fn compare_updates(layout: &DocumentLayout, theirs: &Value) -> Updates {
    let mut out = Updates::default();
    let numbers = PageNumbers::new(layout);
    let (paragraphs, _) = ooxml_word_layout::flow::body_paragraphs(&layout.document.body);
    let by_path: HashMap<&str, &Arc<Paragraph>> = paragraphs.iter().map(|p| (&*p.path, p)).collect();
    let marks = bookmark_paths(&paragraphs);
    let records = theirs.get("updates").and_then(Value::as_array).into_iter().flatten();
    for record in records {
        let path = record.get(0).and_then(Value::as_str).unwrap_or_default();
        let k = record
            .get(1)
            .and_then(Value::as_u64)
            .and_then(|k| usize::try_from(k).ok());
        let expected = record.get(3).and_then(Value::as_str);
        let field = by_path
            .get(path)
            .and_then(|p| p.runs.get(k.unwrap_or(usize::MAX)))
            .and_then(|r| r.field.as_ref());
        let Some(field) = field else {
            out.differences
                .push((format!("{path}/{k:?}"), "no such field in the Rust model".into()));
            continue;
        };
        let mine = if &*field.keyword == "PAGEREF" {
            argument(&field.instruction)
                .and_then(|name| marks.get(&name))
                .and_then(|target| numbers.page_reference(&field.instruction, target).ok())
        } else {
            numbers.page_field(&field.instruction, &field.keyword, path).ok()
        };
        out.compared += 1;
        if mine.as_deref() != expected {
            out.differences.push((
                format!("{path}/{k:?} {}", field.keyword),
                format!("{mine:?} vs {expected:?}"),
            ));
        }
    }
    let references = theirs.get("references").and_then(Value::as_array).into_iter().flatten();
    for record in references {
        let path = record.get(0).and_then(Value::as_str).unwrap_or_default();
        let expected = record.get(1).and_then(Value::as_str);
        let mine = numbers.page_reference(" PAGEREF x \\h ", path).ok();
        out.compared += 1;
        if mine.as_deref() != expected {
            out.differences
                .push((format!("{path} page"), format!("{mine:?} vs {expected:?}")));
        }
    }
    out
}
