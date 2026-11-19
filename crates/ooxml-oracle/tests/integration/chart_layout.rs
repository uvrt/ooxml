//! The chart layout's differential, as a test: every case in `data/chart-layout.json.gz`
//! (ooxml-common's layout of the repository's charts -- the golden fixtures under their
//! rules, the committed packages' chart parts under PowerPoint's and Word's -- with its
//! elements, chart data and notes; `python/chart_diff.py --freeze` writes it) is laid out
//! in Rust and must equal Python's, field by field, numbers as numbers.
//!
//! The answers were frozen on macOS. Pies and radars go through `sin` and `cos`, which the C
//! library computes: Python and Rust on one machine call the same one and agree to the bit
//! (the live differential, `python/chart_diff.py --layout`), but glibc's and Apple's differ
//! in the last place for some arguments. So the comparison is exact where the answers were
//! frozen and within a few units in the last place elsewhere.

use flate2::read::GzDecoder;
use serde_json::Value;
use std::io::Read;

/// Whether two numbers agree: exactly on the platform the answers were frozen on, else
/// within 16 units in the last place (another C library's `sin` and `cos`).
fn same(a: f64, b: f64) -> bool {
    if a == b || cfg!(target_os = "macos") {
        return a == b;
    }
    (a - b).abs() <= 16.0 * f64::EPSILON * a.abs().max(b.abs())
}

/// Where `a` and `b` differ (the first few), as JSON paths; numbers compare by value.
fn diff(a: &Value, b: &Value, path: &str, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            if !same(x.as_f64().unwrap_or(f64::NAN), y.as_f64().unwrap_or(f64::NAN)) {
                out.push(format!("{path}: python {x} rust {y}"));
            }
        }
        (Value::Object(x), Value::Object(y)) => {
            for key in x.keys().chain(y.keys().filter(|k| !x.contains_key(*k))) {
                match (x.get(key), y.get(key)) {
                    (Some(p), Some(r)) => diff(p, r, &format!("{path}.{key}"), out),
                    _ => out.push(format!("{path}.{key}: only on one side")),
                }
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                out.push(format!("{path}: python {} item(s), rust {}", x.len(), y.len()));
            }
            for (i, (p, r)) in x.iter().zip(y).enumerate() {
                diff(p, r, &format!("{path}[{i}]"), out);
            }
        }
        _ if a != b => out.push(format!("{path}: python {a} rust {b}")),
        _ => {}
    }
}

#[test]
fn every_frozen_chart_lays_out_as_ooxml_common_lays_it_out() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/chart-layout.json.gz");
    let mut text = String::new();
    GzDecoder::new(std::fs::File::open(path).unwrap())
        .read_to_string(&mut text)
        .unwrap();
    let records: Vec<Value> = serde_json::from_str(&text).unwrap();
    let mut failures = Vec::new();
    for record in &records {
        let mut case = record["case"].clone();
        let file = format!("{root}/{}", case["file"].as_str().unwrap());
        case["file"] = Value::String(file);
        let rust = ooxml_oracle::chart::layout(&case).unwrap();
        let mut out = Vec::new();
        diff(&record["python"], &rust, "", &mut out);
        if !out.is_empty() {
            out.truncate(5);
            failures.push(format!(
                "{} {}: {}",
                record["case"]["file"],
                record["case"]["rules"],
                out.join("; ")
            ));
        }
    }
    assert!(records.len() >= 144, "the bundle holds {} cases", records.len());
    assert!(
        failures.is_empty(),
        "{} of {} differ:\n{}",
        failures.len(),
        records.len(),
        failures.join("\n")
    );
}
