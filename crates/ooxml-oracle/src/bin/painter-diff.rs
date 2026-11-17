//! `painter-diff RECORDS.jsonl...`: the painter pieces' differential -- every call docx2svg
//! made to ooxml-common's DrawingML pieces, as `crates/ooxml-oracle/python/painter_record.py`
//! recorded it, made again through `ooxml-render` and compared: the answer (numbers by value)
//! and the resources declared. Prints each difference and a count per function; exits 1 when
//! a call differs.

use ooxml_oracle::painter::replay;
use serde_json::Value;
use std::collections::BTreeMap;

fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q)),
        _ => a == b,
    }
}

fn main() {
    let mut counts: BTreeMap<(String, &'static str), usize> = BTreeMap::new();
    let mut failed = false;
    for path in std::env::args().skip(1) {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            eprintln!("{path}: {e}");
            std::process::exit(2);
        });
        for (number, line) in text.lines().enumerate() {
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let name = record.get("fn").and_then(Value::as_str).unwrap_or("?").to_owned();
            let verdict = if record.get("skipped").is_some() {
                "skipped"
            } else {
                match replay(&record) {
                    None => "not replayed",
                    Some(answer) => {
                        let result = record.get("result").unwrap_or(&Value::Null);
                        let defs: Vec<String> = record
                            .get("defs")
                            .and_then(Value::as_array)
                            .map(|d| d.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
                            .unwrap_or_default();
                        if same(&answer.result, result) && answer.defs == defs {
                            "same"
                        } else {
                            failed = true;
                            println!("DIFF {path}:{} {name}", number + 1);
                            println!("  python: {result} {defs:?}");
                            println!("  rust:   {} {:?}", answer.result, answer.defs);
                            "different"
                        }
                    }
                }
            };
            *counts.entry((name, verdict)).or_default() += 1;
        }
    }
    for ((name, verdict), n) in &counts {
        println!("{n:6} {name}: {verdict}");
    }
    std::process::exit(i32::from(failed));
}
