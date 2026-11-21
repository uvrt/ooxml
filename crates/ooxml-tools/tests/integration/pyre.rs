//! `ooxml_tools::pyre` against Python's `re`: `tests/golden/pyre_cases.json` holds random
//! patterns, texts and replacement templates with what Python 3.12 answers (written by
//! `tests/pyre_cases.py`, docx-agent's semantics: `re.finditer`, empty matches left out,
//! `match.expand` on each match). Every pattern Python refuses is refused with Python's
//! words; every one Python reads is matched as Python matches it, or refused as not read
//! here only for the constructs the module names. `PYRE_CASES=<file>` checks another file
//! the script wrote (a larger run).

use ooxml_tools::pyre::{PatternError, PyRegex};
use serde_json::Value;

fn cases() -> Value {
    let path = std::env::var("PYRE_CASES")
        .unwrap_or_else(|_| format!("{}/tests/golden/pyre_cases.json", env!("CARGO_MANIFEST_DIR")));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Why a pattern Python reads may be refused here (the module's own list).
fn may_be_unported(why: &str) -> bool {
    why.contains("\\N{") || why.contains("the flags a and i") || why.contains("the flag t")
}

/// What Rust answers for one text: the matches, or the template's refusal.
fn answer(regex: &PyRegex, text: &str, template: &str) -> Result<Value, PatternError> {
    let found = regex.find_expanded(text, template)?;
    Ok(Value::Array(
        found
            .into_iter()
            .map(|(s, e, with)| serde_json::json!([s, e, with]))
            .collect(),
    ))
}

fn check(case: &Value) -> Result<(), String> {
    let pattern = case["pattern"].as_str().unwrap_or_default();
    let ignore = case["ignore_case"].as_bool().unwrap_or(false);
    let template = case["template"].as_str().unwrap_or_default();
    let python = &case["python"];
    let compiled = PyRegex::new(pattern, ignore);
    if let Some(error) = python.get("error").and_then(Value::as_str) {
        let exception = python["exception"].as_str().unwrap_or("error");
        return match compiled {
            Err(e) if e.python_exception() == Some(exception) && e.message() == error => Ok(()),
            Err(e) => Err(format!("Python: {exception}: {error}; here: {e:?}")),
            Ok(_) => Err(format!("Python refuses ({error}); here it is matched")),
        };
    }
    let regex = match compiled {
        Ok(regex) => regex,
        Err(PatternError::Unported(why)) if may_be_unported(&why) => return Ok(()),
        Err(e) => return Err(format!("Python reads it; here: {e:?}")),
    };
    let texts = case["texts"].as_array().cloned().unwrap_or_default();
    for (text, wanted) in texts.iter().zip(python["matches"].as_array().into_iter().flatten()) {
        let text = text.as_str().unwrap_or_default();
        match (answer(&regex, text, template), wanted.get("template")) {
            (Ok(got), None) if &got == wanted => {}
            (Err(PatternError::Invalid(_)), Some(_)) => {}
            (got, _) => return Err(format!("text {text:?}: Python {wanted}; here {got:?}")),
        }
    }
    Ok(())
}

#[test]
fn patterns_are_read_and_matched_as_python_does() {
    let cases = cases();
    let mut failures = Vec::new();
    let all = cases["cases"].as_array().cloned().unwrap_or_default();
    for case in &all {
        if let Err(why) = check(case) {
            failures.push(format!(
                "{:?} (i={}, template {:?}): {why}",
                case["pattern"].as_str().unwrap_or_default(),
                case["ignore_case"],
                case["template"].as_str().unwrap_or_default()
            ));
        }
    }
    let shown: Vec<&String> = failures.iter().take(40).collect();
    assert!(
        failures.is_empty(),
        "{} of {} cases differ from Python:\n{}",
        failures.len(),
        all.len(),
        shown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n")
    );
}
