//! `layout-diff RECORDS.jsonl[.gz]...`: the Word layout's differential against docx2svg
//! (records written by `python/docx2svg/tools/layout_differential.py`). Prints every
//! difference and a JSON summary; exits 1 when there is a difference.

use flate2::read::GzDecoder;
use ooxml_oracle::layout::{read_layout_records, run_document, Outcome};
use std::io::{BufRead, BufReader};
use std::time::Instant;

fn open(path: &str) -> Result<Box<dyn BufRead>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
    Ok(if path.ends_with(".gz") {
        Box::new(BufReader::new(GzDecoder::new(file)))
    } else {
        Box::new(BufReader::new(file))
    })
}

fn main() {
    let started = Instant::now();
    let mut outcome = Outcome::default();
    for path in std::env::args().skip(1) {
        let result = open(&path)
            .and_then(|reader| read_layout_records(reader, |document| run_document(&document, &mut outcome)));
        if let Err(e) = result {
            eprintln!("{path}: {e}");
            std::process::exit(2);
        }
    }
    let mut causes: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for d in &outcome.differences {
        *causes.entry(d.cause.as_str()).or_default() += 1;
        println!("DIFF {} {} [{}]: {}", d.document, d.at, d.cause, d.detail);
    }
    for (document, why) in &outcome.cut {
        println!("CUT {document}: {why}");
    }
    let summary = serde_json::json!({
        "documents": outcome.documents,
        "whole": outcome.whole,
        "cut": outcome.cut.len(),
        "paragraphs": outcome.paragraphs,
        "lines": outcome.lines,
        "pages": outcome.pages,
        "placed_lines": outcome.placed,
        "fields": outcome.fields,
        "differences": outcome.differences.len(),
        "causes": causes,
        "layout_seconds": outcome.layout_time.as_secs_f64(),
        "total_seconds": started.elapsed().as_secs_f64(),
    });
    println!("{summary}");
    if !outcome.differences.is_empty() {
        std::process::exit(1);
    }
}
