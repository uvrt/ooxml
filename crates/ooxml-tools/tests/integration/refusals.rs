//! Rule 2: a refusal carries the way out. The types make it so for every refusal site
//! (`ToolError::refusal` takes a `WayOut`, which cannot be empty); this test walks every
//! problem the contract refuses ([`Problem::ALL`]) and checks each reply names the options or
//! a call that would work, and that the call is right where a value is known.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::ported;

use ooxml_tools::{ErrorCode, Failure, Problem, Refusal, ToolArgs, ToolError, WayOut};
use ported::{ReplaceText, WordFormat, WordSetText};
use serde_json::{json, Value};
use std::collections::HashSet;

/// A call that hits each problem: (problem, tool, arguments).
fn walk() -> Vec<(Problem, ooxml_tools::ToolSpec, Value)> {
    let format = WordFormat::spec;
    vec![
        (Problem::NotJson, format(), json!("{\"doc\": ")),
        (Problem::NotObject, format(), json!(["d1"])),
        (Problem::UnknownArgument, format(), json!({"doc": "d1", "bolld": true})),
        (Problem::Required, format(), json!({"bold": true})),
        (
            Problem::AliasTwice,
            format(),
            json!({"doc": "d1", "top": 1, "space_before": 2}),
        ),
        (
            Problem::NotExactlyOne,
            exactly_one(),
            json!({"doc": "d1", "target": "p:A", "items": []}),
        ),
        (Problem::NotArray, format(), json!({"doc": "d1", "targets": 3})),
        (Problem::TooFewItems, bounded(), json!({"doc": "d1", "names": []})),
        (
            Problem::TooManyItems,
            bounded(),
            json!({"doc": "d1", "names": ["a", "b", "c"]}),
        ),
        (Problem::NotString, format(), json!({"doc": 1})),
        (
            Problem::NotInEnum,
            format(),
            json!({"doc": "d1", "alignment": "centre"}),
        ),
        (Problem::TooShort, bounded(), json!({"doc": "d1", "code": ""})),
        (Problem::TooLong, bounded(), json!({"doc": "d1", "code": "abcdef"})),
        (Problem::NotFormat, bounded(), json!({"doc": "d1", "day": "7 Oct"})),
        (Problem::NotBoolean, format(), json!({"doc": "d1", "bold": "yes"})),
        (Problem::NotNumber, format(), json!({"doc": "d1", "size": "12"})),
        (
            Problem::NotWhole,
            format(),
            json!({"doc": "d1", "find": [{"text": "x", "occurrence": 1.5}]}),
        ),
        (
            Problem::BelowMinimum,
            ReplaceText::spec(),
            json!({"doc": "d1", "find": "a", "replace": "b", "expect": "one", "slides": [0]}),
        ),
        (Problem::AboveMaximum, format(), json!({"doc": "d1", "size": 2000})),
    ]
}

/// A tool whose text and items are exactly one of the two.
fn exactly_one() -> ooxml_tools::ToolSpec {
    let mut spec = WordSetText::spec();
    spec.exactly_one.push(vec!["target".into(), "items".into()]);
    spec
}

/// A tool with the bounds today's tools rarely use.
#[derive(ToolArgs)]
#[tool(name = "toy_bounds", description = "Bounds.")]
#[allow(dead_code)]
struct Bounded {
    /// Document id.
    doc: String,
    /// Two names at most, one at least.
    #[arg(min_items = 1, max_items = 2)]
    names: Option<Vec<String>>,
    /// A code.
    #[arg(min_length = 2, max_length = 4)]
    code: Option<String>,
    /// A day.
    #[arg(format = "date")]
    day: Option<String>,
}

fn bounded() -> ooxml_tools::ToolSpec {
    Bounded::spec()
}

#[test]
fn every_refusal_site_carries_the_way_out() {
    let mut hit = HashSet::new();
    for (problem, spec, arguments) in walk() {
        let error = spec
            .accept(arguments.clone())
            .expect_err(&format!("{problem:?} refuses {arguments}"));
        assert_eq!(error.code(), ErrorCode::Refusal(Refusal::InvalidArguments));
        let way_out = error.way_out().expect("a refusal has a way out");
        assert!(
            !way_out.valid_options().is_empty() || way_out.try_call().is_some(),
            "{problem:?}: {}",
            error.message()
        );
        if let Some(call) = way_out.try_call() {
            assert_eq!(call.tool, spec.name, "{problem:?}");
            // The call that would work passes, once its examples are filled in.
            if call.fill.is_empty() {
                spec.accept(Value::Object(call.arguments.clone()))
                    .unwrap_or_else(|e| panic!("{problem:?}: {e}"));
            }
        }
        hit.insert(problem);
    }
    let all: HashSet<Problem> = Problem::ALL.into_iter().collect();
    assert_eq!(hit, all, "the walk misses a problem");
}

#[test]
fn the_way_out_is_the_call_that_works_where_the_value_is_known() {
    let error = WordFormat::spec()
        .accept(json!({"doc": "d1", "size": 2000, "bold": true}))
        .unwrap_err();
    let call = error.way_out().unwrap().try_call().unwrap();
    assert_eq!(
        Value::Object(call.arguments.clone()),
        json!({"doc": "d1", "size": 1638, "bold": true})
    );
    assert!(call.fill.is_empty());

    let error = WordFormat::spec().accept(json!({"bold": true})).unwrap_err();
    let call = error.way_out().unwrap().try_call().unwrap();
    assert_eq!(call.fill, ["doc"]);
    assert_eq!(
        error.to_json()["try_call"],
        json!({"tool": "word_format", "arguments": {"bold": true, "doc": "<doc>"}, "fill": ["doc"]})
    );

    let error = WordFormat::spec()
        .accept(json!({"doc": "d1", "find": [{"text": "x", "occurrence": 0}]}))
        .unwrap_err();
    let call = error.way_out().unwrap().try_call().unwrap();
    assert_eq!(call.arguments["find"], json!([{"text": "x", "occurrence": 1}]));
}

#[test]
fn only_refusals_must_carry_a_way_out() {
    let refusals: Vec<&str> = ErrorCode::ALL
        .iter()
        .filter(|code| code.needs_way_out())
        .map(|code| code.as_str())
        .collect();
    assert_eq!(refusals, ["not_found", "ambiguous", "refused", "invalid_arguments"]);
    assert_eq!(ErrorCode::ALL.len(), 11);
    // Options cannot be empty: an empty list is no way out.
    assert!(WayOut::options(Vec::new()).is_none());
    let timeout = ToolError::failure(Failure::Timeout, "the deadline passed");
    assert_eq!(
        timeout.to_json(),
        json!({"code": "timeout", "message": "the deadline passed", "valid_options": []})
    );
    let internal = ToolError::internal("boom", "t-17");
    assert_eq!(internal.to_json()["details"], json!({"trace": "t-17"}));
}
