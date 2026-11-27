//! The host goldens replayed against the Rust host (Phase 3 D1, design section 7.2): every
//! docx stream's calls the Rust host serves answer as the Python host recorded them, but
//! where a known difference admits it; the calls that wait for a later PR (D2, D3, G1) are
//! counted. The report (`host-replay.json` in the test's target folder) lists each stream's
//! counts and every difference.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_oracle::host_replay::{Outcome, Replayer};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::Path;

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

#[derive(Default)]
struct Counts {
    equal: u64,
    known: u64,
    different: u64,
    waiting: BTreeMap<String, u64>,
}

/// One stream's cases replayed: its counts and its differences.
fn replay_stream(replayer: &Replayer, name: &str, path: &Path, every: usize) -> (Counts, Vec<Value>) {
    let mut counts = Counts::default();
    let mut differences = Vec::new();
    for (case, events) in Replayer::cases(path).unwrap().into_iter().step_by(every) {
        for (event, outcome) in replayer.replay(name, &events) {
            let call = matches!(event["event"].as_str(), Some("call" | "calls" | "change"));
            match outcome {
                Outcome::Equal if call => counts.equal += 1,
                Outcome::Known(_) if call => counts.known += 1,
                Outcome::Waiting(waits, after) if call => {
                    let key = if after {
                        format!("{} (after)", waits.name())
                    } else {
                        waits.name().to_owned()
                    };
                    *counts.waiting.entry(key).or_default() += 1;
                }
                Outcome::Different(paths, want, have) => {
                    counts.different += 1;
                    differences.push(json!({"stream": name, "case": case, "at": event["at"],
                        "tool": event["tool"], "arguments": event["arguments"], "paths": paths,
                        "want": want, "have": have}));
                }
                _ => {}
            }
        }
    }
    (counts, differences)
}

#[test]
fn every_call_the_rust_host_serves_answers_as_recorded() {
    let replayer = Replayer::new(root()).unwrap();
    let streams = replayer.streams();
    assert!(streams.len() > 100, "the docx streams are there");
    let only = std::env::var("HOST_REPLAY_ONLY").ok();
    // every case with HOST_REPLAY_EVERY=1 (E1's gate, about 15 minutes); a tenth by default
    let every: usize = std::env::var("HOST_REPLAY_EVERY")
        .ok()
        .and_then(|e| e.parse().ok())
        .filter(|e| *e > 0)
        .unwrap_or(10);
    let mut report = Map::new();
    let mut totals = Counts::default();
    let mut differences = Vec::new();
    let streams: Vec<_> = streams
        .into_iter()
        // (several streams: their names' parts, comma-separated)
        .filter(|(name, _)| only.as_deref().is_none_or(|o| o.split(',').any(|o| name.contains(o))))
        .collect();
    // each stream's cases replayed on one of a few threads (the replay is the host's, and
    // a session is its own: nothing is shared but the faces)
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done: std::sync::Mutex<Vec<(String, Counts, Vec<Value>)>> = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..3 {
            scope.spawn(|| loop {
                let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some((name, path)) = streams.get(k) else { break };
                let (counts, found) = replay_stream(&replayer, name, path, every);
                done.lock().unwrap().push((name.clone(), counts, found));
            });
        }
    });
    let mut done = done.into_inner().unwrap();
    done.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, counts, mut found) in done {
        differences.append(&mut found);
        totals.equal += counts.equal;
        totals.known += counts.known;
        totals.different += counts.different;
        for (key, n) in &counts.waiting {
            *totals.waiting.entry(key.clone()).or_default() += n;
        }
        report.insert(
            name,
            json!({"equal": counts.equal, "known": counts.known, "different": counts.different,
                   "waiting": counts.waiting}),
        );
    }
    let summary = json!({"equal": totals.equal, "known": totals.known, "different": totals.different,
                         "waiting": totals.waiting});
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("host-replay.json");
    let written = json!({"summary": summary, "streams": report, "differences": differences});
    std::fs::write(&out, serde_json::to_string_pretty(&written).unwrap()).unwrap();
    eprintln!("host replay: {summary} (report: {})", out.display());
    let shown: Vec<&Value> = differences.iter().take(5).collect();
    assert_eq!(totals.different, 0, "calls that differ from the recording: {shown:#?}");
}

/// The switch (Phase 3 E1a; user decision 2026-11-25, structured problems): `check`'s
/// `data.validate`, recorded as today's text, is answered by the Rust host as
/// `{code, part, element, message, severity}` objects -- each such call a known difference of
/// the decision's (its release-notes line), never an unlisted one.
#[test]
fn checks_data_validate_says_each_problem_in_the_one_shape() {
    let replayer = Replayer::new(root()).unwrap();
    let (_, path) = replayer
        .streams()
        .into_iter()
        .find(|(name, _)| name == "docx/tests/test_save_gate_check")
        .unwrap();
    let mut seen = 0;
    for (_, events) in Replayer::cases(&path).unwrap() {
        for (event, outcome) in replayer.replay("docx/tests/test_save_gate_check", &events) {
            let Some(validate) = event["result"]["data"].get("validate") else {
                continue;
            };
            if validate["new"].as_array().is_none_or(Vec::is_empty) {
                continue;
            }
            assert!(validate["new"][0].is_string(), "recorded as today's text: {event}");
            assert!(
                matches!(outcome, Outcome::Known(_)),
                "the decision's known difference: {event}"
            );
            seen += 1;
        }
    }
    assert!(seen > 0, "a check whose validate found new problems replayed");
}
