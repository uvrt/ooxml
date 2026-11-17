//! `svg-diff RECORDS.jsonl[.gz]...`: the Word layout's SVG differential against docx2svg
//! (records written by `python/docx2svg/tools/layout_differential.py --svg`). Prints every
//! cause of a page that differs, with a count and an example, and a JSON summary; exits 1
//! when a page differs.

use flate2::read::GzDecoder;
use ooxml_oracle::layout::read_layout_records;
use ooxml_oracle::layout_svg::{run_svg_document, SvgOutcome};
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
    let mut outcome = SvgOutcome::default();
    for path in std::env::args().skip(1) {
        let result = open(&path)
            .and_then(|reader| read_layout_records(reader, |document| run_svg_document(&document, &mut outcome)));
        if let Err(e) = result {
            eprintln!("{path}: {e}");
            std::process::exit(2);
        }
    }
    for (cause, (count, example)) in &outcome.causes {
        println!("CAUSE {count} [{cause}]: {example}");
    }
    let causes: std::collections::BTreeMap<&str, usize> =
        outcome.causes.iter().map(|(c, (n, _))| (c.as_str(), *n)).collect();
    let summary = serde_json::json!({
        "documents": outcome.documents,
        "pages": outcome.pages,
        "same": outcome.same,
        "different": outcome.pages - outcome.same,
        "causes": causes,
        "paint_seconds": outcome.time.as_secs_f64(),
        "total_seconds": started.elapsed().as_secs_f64(),
    });
    println!("{summary}");
    if outcome.same != outcome.pages {
        std::process::exit(1);
    }
}
