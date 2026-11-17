//! `metafile-diff CASES_DIR...`: the metafile players' differential against ooxml-common
//! (cases written by `crates/ooxml-oracle/python/metafile_record.py`). Prints a summary and
//! every difference; exits 1 when there is one (a PNG whose deflate bytes alone differ is
//! counted, not a difference).

use ooxml_oracle::metafile::{compare, read_case, Verdict};
use std::collections::BTreeMap;

fn main() {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut failed = false;
    for dir in std::env::args().skip(1) {
        let mut paths: Vec<_> = match std::fs::read_dir(&dir) {
            Ok(entries) => entries.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(e) => {
                eprintln!("{dir}: {e}");
                std::process::exit(2);
            }
        };
        paths.sort();
        for path in paths.iter().filter(|p| p.extension().is_some_and(|e| e == "case")) {
            let case = std::fs::read(path)
                .map_err(|e| e.to_string())
                .and_then(|b| read_case(&b));
            let verdict = match case {
                Ok(case) => compare(&case),
                Err(e) => Verdict::Different("case".into(), e),
            };
            let key = match &verdict {
                Verdict::Same => "same".to_owned(),
                Verdict::SamePixels => "same pixels (png deflate bytes differ)".to_owned(),
                Verdict::Different(cause, detail) => {
                    failed = true;
                    println!("DIFF {} [{cause}] {detail}", path.display());
                    format!("different: {cause}")
                }
            };
            *counts.entry(key).or_default() += 1;
        }
    }
    for (k, v) in &counts {
        println!("{v:6} {k}");
    }
    std::process::exit(i32::from(failed));
}
