//! The Python host's frozen answers for the values a Word call reads from its sources
//! (`host/frozen/values.jsonl`, made by `python/freeze_values.py` over docx-agent's tests):
//! `word_insert_markdown`'s `from` naming one text field, `word_edit_table`'s `from` naming
//! rows, each cell's one-value `from` and its refusals -- each answered by the Rust host as
//! the Python host answered it, but where a known difference admits it. The cases the freezer
//! could not freeze are counted, with why.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_oracle::host_replay::{Outcome, Replayer};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

#[test]
fn every_frozen_answer_of_a_value_from_a_source_is_the_rust_hosts() {
    let replayer = Replayer::new(root()).unwrap();
    let path = root().join("crates/ooxml-oracle/host/frozen/values.jsonl");
    let text = std::fs::read_to_string(&path).unwrap();
    let cases: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(!cases.is_empty(), "the frozen answers are there");
    let (mut equal, mut known) = (0, 0);
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    let mut different = Vec::new();
    for case in &cases {
        match replayer.frozen(case) {
            Outcome::Equal => equal += 1,
            Outcome::Known(_) => known += 1,
            Outcome::Waiting(..) => {
                *skipped
                    .entry(case["skipped"].as_str().unwrap_or("settings").to_owned())
                    .or_default() += 1;
            }
            Outcome::Different(paths, want, have) => {
                different.push(serde_json::json!({"test": case["test"], "tool": case["tool"],
                    "arguments": case["arguments"], "paths": paths, "want": want, "have": have}));
            }
        }
    }
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("frozen-values.json");
    std::fs::write(&out, serde_json::to_string_pretty(&different).unwrap()).unwrap();
    eprintln!(
        "frozen values: {equal} equal, {known} known, {} different; not frozen: {skipped:?}",
        different.len()
    );
    let shown: Vec<&Value> = different.iter().take(5).collect();
    assert!(
        different.is_empty(),
        "answers that differ from the Python host's: {shown:#?}"
    );
}

/// The known differences marked "until R138" (`host/known-differences.toml`): the Rust host
/// checks a `from` naming rows before `word_insert_markdown` refuses it as not supported yet,
/// where today's adapter never resolved them. They hold only while the tool refuses rows:
/// when R138 writes them as a table, this test fails -- remove those entries with it.
#[test]
fn insert_markdown_from_rows_is_not_supported_until_r138() {
    let replayer = Replayer::new(root()).unwrap();
    let path = root().join("crates/ooxml-oracle/host/frozen/values.jsonl");
    let text = std::fs::read_to_string(&path).unwrap();
    let case: Value = text
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|c| {
            c["test"]
                .as_str()
                .unwrap_or_default()
                .contains("test_a_drawings_id_as_from_is_named_a_drawing")
        })
        .unwrap();
    // the same call with a dataset that is there: rows, still refused as not supported
    let mut rows = case.clone();
    rows["arguments"]["from"] = Value::from("d:stock");
    rows["datasets"] = serde_json::json!([{"id": "stock", "columns": ["item", "count"],
        "rows": [["Bolts", 3]], "sources": null, "keys": 1, "types": null, "markdown": [], "cache": null}]);
    rows["result"] = case["result"].clone();
    let outcome = replayer.frozen(&rows);
    assert!(
        matches!(outcome, Outcome::Equal | Outcome::Known(_)),
        "word_insert_markdown writes a from naming rows now (R138): remove the known differences marked \
         'until R138' in host/known-differences.toml, and this test ({outcome:?})"
    );
    let known = std::fs::read_to_string(root().join("crates/ooxml-oracle/host/known-differences.toml")).unwrap();
    assert!(known.contains("until R138"), "the entries this test guards are there");
}
