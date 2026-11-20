//! The chart goldens, as a test: every chart ooxml-common's golden harness draws
//! (`python/ooxml-common/tests/fixtures/charts.json`, each under its rules) is laid out by
//! `ooxml-chart`, painted by `ooxml-render` and written as the harness writes it, and must
//! equal its golden (`tests/golden/charts`) byte for byte.

use serde_json::Value;

#[test]
fn every_chart_golden_is_drawn_byte_for_byte() {
    let common = concat!(env!("CARGO_MANIFEST_DIR"), "/../../python/ooxml-common/tests");
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{common}/fixtures/charts.json")).unwrap()).unwrap();
    let mut failures = Vec::new();
    let mut count = 0;
    for (name, entry) in manifest.as_object().unwrap() {
        for rules in entry["rules"].as_array().unwrap() {
            let rules = rules.as_str().unwrap();
            let mut spec = entry.clone();
            spec["file"] = Value::String(format!("{common}/fixtures/charts/{name}.xml"));
            spec["rules"] = Value::String(rules.to_owned());
            let produced = ooxml_oracle::chart::drawn::golden(&spec).unwrap();
            let expected = std::fs::read_to_string(format!("{common}/golden/charts/{name}-{rules}.svg")).unwrap();
            count += 1;
            if produced != expected {
                let at = produced
                    .bytes()
                    .zip(expected.bytes())
                    .position(|(a, b)| a != b)
                    .unwrap_or(0);
                let from = at.saturating_sub(80);
                failures.push(format!(
                    "{name}-{rules} at byte {at}:\n  rust:   {}\n  python: {}",
                    produced.get(from..(at + 120).min(produced.len())).unwrap_or(""),
                    expected.get(from..(at + 120).min(expected.len())).unwrap_or("")
                ));
            }
        }
    }
    assert_eq!(count, 62, "the manifest holds {count} cases");
    assert!(
        failures.is_empty(),
        "{} of {count} differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
