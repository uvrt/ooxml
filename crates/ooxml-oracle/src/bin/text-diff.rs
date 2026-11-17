//! `text-diff RECORDS.jsonl[.gz]...`: the text engine's differential against docx2svg
//! (records written by `python/docx2svg/tools/text_differential.py`). Prints a JSON summary
//! and every difference; exits 1 when there is one.

use flate2::read::GzDecoder;
use ooxml_oracle::text::{read_records, run_document, Outcome};
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
        let result =
            open(&path).and_then(|reader| read_records(reader, |document| run_document(&document, &mut outcome)));
        if let Err(e) = result {
            eprintln!("{path}: {e}");
            std::process::exit(2);
        }
    }
    let mut causes: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for d in &outcome.differences {
        *causes.entry(d.cause.as_str()).or_default() += 1;
        println!(
            "DIFF {} #{} [{}] {:?}: {}",
            d.document, d.paragraph, d.cause, d.text, d.detail
        );
    }
    for r in &outcome.report_differences {
        println!("REPORT {r}");
    }
    let summary = serde_json::json!({
        "documents": outcome.documents,
        "paragraphs_compared": outcome.compared,
        "measured_both": outcome.measured,
        "unmeasurable_both": outcome.unmeasurable,
        "pieces": outcome.pieces,
        "lines": outcome.lines,
        "differences": outcome.differences.len(),
        "causes": causes,
        "report_differences": outcome.report_differences.len(),
        "engine_seconds": outcome.engine_time.as_secs_f64(),
        "total_seconds": started.elapsed().as_secs_f64(),
    });
    println!("{summary}");
    if !outcome.differences.is_empty() || !outcome.report_differences.is_empty() {
        std::process::exit(1);
    }
}
