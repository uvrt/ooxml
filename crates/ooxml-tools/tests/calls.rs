//! The intent rules and the validator, call by call, against today's Python dispatcher:
//! `tests/golden/calls.json` holds calls of the ported tools and what today's dispatcher
//! makes of them (`python/docx-agent/tests/test_rust_tool_specs.py` holds the file to
//! today's Python). The refusals here also carry a `try_call` (rule 2), which today's do
//! not; the comparison is of code, message, field and valid options.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod ported;

use ooxml_tools::ToolSpec;
use serde_json::{json, Value};

fn outcome(spec: &ToolSpec, arguments: Value, kind: Option<&str>) -> Value {
    match spec.accept(arguments) {
        Ok(mut accepted) => {
            let ignored = spec.ignored_for(kind, &mut accepted.arguments);
            accepted.notes.extend(ignored);
            json!({"arguments": accepted.arguments, "notes": accepted.notes})
        }
        Err(error) => {
            let way_out = error.way_out().expect("a refusal carries its way out");
            assert!(
                !way_out.valid_options().is_empty() || way_out.try_call().is_some(),
                "{}: no way out",
                error.message()
            );
            let mut error = error.to_json();
            let map = error.as_object_mut().unwrap();
            map.remove("try_call");
            map.entry("field").or_insert(Value::Null);
            json!({"error": error})
        }
    }
}

#[test]
fn every_call_is_read_as_todays_dispatcher_reads_it() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/calls.json");
    let cases: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let specs: Vec<ToolSpec> = ported::ported().into_iter().map(|(spec, _, _)| spec).collect();
    assert!(cases.len() >= 30);
    for case in cases {
        let spec = specs.iter().find(|s| s.name == case["tool"]).unwrap();
        let got = outcome(spec, case["arguments"].clone(), case["kind"].as_str());
        assert_eq!(got, case["outcome"], "{case}");
    }
}
