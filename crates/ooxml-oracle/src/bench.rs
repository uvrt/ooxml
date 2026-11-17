//! `word-bench <file.docx> [--rounds N] [--check]`: open a document in a fresh model, then
//! reopen a version with one body paragraph's text changed -- its main part put back as
//! bytes, as Python hands Rust a version's touched parts -- and report wall times (best of
//! N) and what the reopen read and resolved again, as one JSON line. `--check` also
//! compares the reopened version's export with a fresh open of the same version (reuse
//! must give what a full open gives).
//!
//! Must not: be used by any library; it is a measurement instrument.

use crate::export::{document_json, Unlimited, KEY};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_word_model::{Model, OpenStats};
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;

/// The main part's bytes with one character added to the text of the body paragraph
/// nearest the middle: the one-block edit.
fn edited(xml: &[u8]) -> Option<Vec<u8>> {
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
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).map_err(|e| format!("{path}: {e}"))?);
    let limits = Limits::default();
    let main = "word/document.xml";
    let (mut open_best, mut reopen_best) = (f64::MAX, f64::MAX);
    let (mut first, mut second) = (OpenStats::default(), OpenStats::default());
    for _ in 0..rounds.max(1) {
        let package = Package::open(Arc::clone(&bytes), &limits).map_err(|e| e.to_string())?;
        let xml = package
            .read(main)
            .map_err(|e| e.to_string())?
            .ok_or("no word/document.xml")?;
        let changed = edited(&xml).ok_or("no paragraph text to edit")?;
        let next = package.with_bytes(main, Arc::from(changed));
        let mut model = Model::new(Arc::new(Unlimited), KEY);
        let start = Instant::now();
        let opened = model.open(&package).map_err(|e| e.to_string())?;
        open_best = open_best.min(start.elapsed().as_secs_f64());
        first = model.stats();
        let start = Instant::now();
        let reopened = model.open(&next).map_err(|e| e.to_string())?;
        reopen_best = reopen_best.min(start.elapsed().as_secs_f64());
        second = model.stats();
        if check {
            let reused = document_json(&mut model, &reopened);
            let mut fresh_model = Model::new(Arc::new(Unlimited), KEY);
            let fresh = fresh_model.open(&next).map_err(|e| e.to_string())?;
            if reused != document_json(&mut fresh_model, &fresh) {
                return Err("the reopened version differs from a fresh open of it".into());
            }
        }
        drop(opened);
    }
    let line = json!({
        "file": path, "rounds": rounds, "open_s": open_best, "reopen_s": reopen_best,
        "open": stats_json(&first), "reopen": stats_json(&second), "checked": check,
    });
    println!("{line}");
    Ok(())
}
