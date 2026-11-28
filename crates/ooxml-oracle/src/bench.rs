//! `word-bench <file.docx> [--rounds N] [--check] [--list-edit]`: open a document in a fresh
//! model, then reopen a version with one body paragraph's text (`--list-edit`: one list
//! item's level) changed -- its main part put back as
//! bytes, as Python hands Rust a version's touched parts -- and report wall times (best of
//! N), the time to resolve every body paragraph's and table's formats after the open, and
//! what the reopen read and resolved again, as one JSON line. `--check` also
//! compares the reopened version's export with a fresh open of the same version (reuse
//! must give what a full open gives).
//!
//! Must not: be used by any library; it is a measurement instrument.

use crate::export::{document_json, Unlimited, KEY};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::model::Block;
use ooxml_word_model::{Model, OpenStats, WordDocument};
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;

/// The main part's bytes with one character added to the text of the body paragraph
/// nearest the middle: the one-block edit.
pub(crate) fn edited(xml: &[u8]) -> Option<Vec<u8>> {
    let middle = xml.len() / 2;
    let mut from = middle;
    loop {
        let at = xml
            .get(from..)?
            .windows(5)
            .position(|w| w == b"<w:t>" || w == b"<w:t ")?
            + from;
        let close = xml.get(at..)?.iter().position(|b| *b == b'>')? + at;
        if xml.get(close.checked_sub(1)?) != Some(&b'/') {
            let mut out = xml.get(..=close)?.to_vec();
            out.push(b'X');
            out.extend_from_slice(xml.get(close + 1..)?);
            return Some(out);
        }
        from = close;
    }
}

/// Resolve every paragraph (its own, its mark's and its runs' formats) and table of the
/// body, as a layout asks for them.
fn resolve_all(model: &mut Model, document: &WordDocument) -> usize {
    fn walk(model: &mut Model, document: &WordDocument, block: &Block) -> usize {
        match block {
            Block::Paragraph(p) => model.resolve_paragraph(document, p).runs.len() + 1,
            Block::Table(t) => {
                model.resolve_table(document, t);
                t.rows
                    .iter()
                    .flat_map(|r| r.iter())
                    .flat_map(|c| c.iter())
                    .map(|b| walk(model, document, b))
                    .sum()
            }
        }
    }
    document.body.iter().map(|b| walk(model, document, b)).sum()
}

/// The main part's bytes with the list level of the list item nearest the middle changed
/// (`w:ilvl` 0 and 1 swapped): a one-block edit that changes the labels after it.
fn relisted(xml: &[u8]) -> Option<Vec<u8>> {
    let middle = xml.len() / 2;
    let needle = b"<w:ilvl w:val=\"";
    let at = xml.get(middle..)?.windows(needle.len()).position(|w| w == needle)? + middle + needle.len();
    let level = *xml.get(at)?;
    let mut out = xml.to_vec();
    *out.get_mut(at)? = if level == b'0' { b'1' } else { b'0' };
    Some(out)
}

fn stats_json(s: &OpenStats) -> serde_json::Value {
    json!({
        "elements": s.elements, "elements_read": s.elements_read, "label_blocks": s.label_blocks,
        "label_edits": s.label_edits, "resolutions": s.resolutions, "context_changed": s.context_changed,
    })
}

/// Run the benchmark.
pub fn run(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("give a .docx")?;
    let rounds: usize = args
        .iter()
        .position(|a| a == "--rounds")
        .and_then(|k| args.get(k + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(3);
    let check = args.iter().any(|a| a == "--check");
    let list_edit = args.iter().any(|a| a == "--list-edit");
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).map_err(|e| format!("{path}: {e}"))?);
    let limits = Limits::default();
    let main = "word/document.xml";
    let (mut open_best, mut reopen_best, mut resolve_best, mut resolved) = (f64::MAX, f64::MAX, f64::MAX, 0);
    let (mut labels_best, mut labels) = (f64::MAX, 0);
    let (mut first, mut second) = (OpenStats::default(), OpenStats::default());
    for _ in 0..rounds.max(1) {
        let package = Package::open(Arc::clone(&bytes), &limits).map_err(|e| e.to_string())?;
        let xml = package
            .read(main)
            .map_err(|e| e.to_string())?
            .ok_or("no word/document.xml")?;
        let changed = if list_edit {
            relisted(&xml).ok_or("no list item to edit")?
        } else {
            edited(&xml).ok_or("no paragraph text to edit")?
        };
        let package = ooxml_word_model::WordPackage::open(package).0;
        let next =
            ooxml_word_model::WordPackage::open_after(package.with_bytes(main, Arc::from(changed)), Some(&package)).0;
        let mut model = Model::new(Arc::new(Unlimited), KEY);
        let start = Instant::now();
        let reading = Docx::open(&package, Arc::new(Unlimited)).map_err(|e| e.to_string())?;
        let opened = model.project(&reading, None).map_err(|e| e.to_string())?;
        open_best = open_best.min(start.elapsed().as_secs_f64());
        first = model.stats();
        let start = Instant::now();
        resolved = resolve_all(&mut model, &opened);
        resolve_best = resolve_best.min(start.elapsed().as_secs_f64());
        let start = Instant::now();
        let next_reading = Docx::open_after(&next, &reading).map_err(|e| e.to_string())?;
        let reopened = model.project(&next_reading, None).map_err(|e| e.to_string())?;
        reopen_best = reopen_best.min(start.elapsed().as_secs_f64());
        second = model.stats();
        let start = Instant::now();
        labels = reopened.body_labels().iter().filter(|l| l.is_some()).count();
        labels_best = labels_best.min(start.elapsed().as_secs_f64());
        if check {
            let reused = document_json(&mut model, &reopened);
            let mut fresh_model = Model::new(Arc::new(Unlimited), KEY);
            let fresh_reading = Docx::open(&next, Arc::new(Unlimited)).map_err(|e| e.to_string())?;
            let fresh = fresh_model.project(&fresh_reading, None).map_err(|e| e.to_string())?;
            if reused != document_json(&mut fresh_model, &fresh) {
                return Err("the reopened version differs from a fresh open of it".into());
            }
        }
        drop(opened);
    }
    let line = json!({
        "file": path, "rounds": rounds, "open_s": open_best, "reopen_s": reopen_best,
        "resolve_all_s": resolve_best, "resolved_formats": resolved, "edit": if list_edit { "list level" } else { "text" },
        "all_labels_after_reopen_s": labels_best, "labels": labels,
        "open": stats_json(&first), "reopen": stats_json(&second), "checked": check,
    });
    println!("{line}");
    Ok(())
}
