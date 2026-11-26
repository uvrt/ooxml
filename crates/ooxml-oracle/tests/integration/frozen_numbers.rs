//! The Python host's frozen answers for `check`'s numbers and scale (`frozen/numbers.jsonl`,
//! made by `python/freeze_numbers.py` over ooxml-edit's `test_tools_numbers.py` and
//! `test_tools_scales.py` and docx-agent's `test_numbers.py`, `test_toc_numbers.py` and
//! `test_scale_styles.py`): each answered by the Rust host (a Word document's check) or the
//! session's service (a made-up format's) as the Python host answered it, but where a known
//! difference admits it. The cases the freezer could not freeze are counted, with why.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_oracle::host_replay::{Outcome, Replayer};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

#[test]
fn every_frozen_answer_of_a_check_of_numbers_is_the_rust_hosts() {
    let replayer = Replayer::new(root()).unwrap();
    let path = root().join("crates/ooxml-oracle/frozen/numbers.jsonl");
    let text = std::fs::read_to_string(&path).unwrap();
    let cases: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(!cases.is_empty(), "the frozen answers are there");
    let (mut equal, mut known) = (0, 0);
    let mut waiting: BTreeMap<String, usize> = BTreeMap::new();
    let mut different = Vec::new();
    for case in &cases {
        match replayer.frozen_numbers(case) {
            Outcome::Equal => equal += 1,
            Outcome::Known(_) => known += 1,
            Outcome::Waiting(waits, _) => {
                let why = case["skipped"]
                    .as_str()
                    .map_or_else(|| waits.name().to_owned(), str::to_owned);
                *waiting.entry(why).or_default() += 1;
            }
            Outcome::Different(paths, want, have) => {
                different.push(serde_json::json!({"test": case["test"], "arguments": case["arguments"],
                    "items": case["items"], "paths": paths, "want": want, "have": have}));
            }
        }
    }
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("frozen-numbers.json");
    std::fs::write(&out, serde_json::to_string_pretty(&different).unwrap()).unwrap();
    eprintln!(
        "frozen numbers: {equal} equal, {known} known, {} different; waiting or not frozen: {waiting:?}",
        different.len()
    );
    let shown: Vec<&Value> = different.iter().take(5).collect();
    assert!(
        different.is_empty(),
        "answers that differ from the Python host's: {shown:#?}"
    );
}
