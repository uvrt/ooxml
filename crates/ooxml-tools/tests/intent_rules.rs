//! Every intent rule, declared once on a field, with a test (tool design rule 1). The
//! rules and where today's Python applies each are listed in the PR; the calls golden
//! (`tests/calls.rs`) compares the ported tools' calls with today's dispatcher.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod ported;

use ooxml_tools::{ArgEnum, IntOrStr, NumOrStr, ToolArgs};
use ported::{Expect, ReplaceText, Stories, WordFormat, WordSetText};
use serde_json::{json, Map, Value};

/// How a list continues.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
enum Lists {
    /// Restart.
    Restart,
    /// Continue.
    RunOn,
}

/// A pair of a table column and a data column.
#[derive(ToolArgs, Debug, PartialEq)]
struct Match {
    table: String,
    data: String,
}

/// A tool for the rules today's three ported tools do not use.
#[derive(ToolArgs, Debug, PartialEq)]
#[tool(name = "toy_rules", description = "Exercise the intent rules.", mutates)]
struct Rules {
    /// Document id.
    doc: String,
    /// Columns to take; empty means all.
    #[arg(empty_is_absent)]
    columns: Option<Vec<String>>,
    /// Date pattern.
    #[arg(empty_is_absent)]
    date_format: Option<String>,
    /// Pairs of a table column and a data column.
    #[arg(map_as_pairs)]
    pairs: Option<Vec<Match>>,
    /// How lists continue.
    #[arg(value_alias("continue", "run_on"), value_alias("run on", "run_on"))]
    lists: Option<Lists>,
    /// Which stories, decks and documents.
    #[arg(kinds("pptx", "docx"), kind_neutral("body", "all"))]
    stories: Option<String>,
    /// A grade: text or a number.
    grade: Option<NumOrStr>,
    /// Values, null for a blank.
    values: Option<Vec<Option<f64>>>,
    /// Slides.
    slides: Option<Vec<IntOrStr<1>>>,
    /// Free-form options.
    options: Option<Map<String, Value>>,
}

fn accept<T: ToolArgs>(arguments: Value, kind: Option<&str>) -> (T, Vec<String>) {
    T::accept(arguments, kind).unwrap_or_else(|e| panic!("{}", e.to_json()))
}

#[test]
fn empty_is_absent() {
    let (call, notes) = accept::<Rules>(json!({"doc": "d1", "columns": [], "date_format": ""}), None);
    assert_eq!((call.columns, call.date_format), (None, None));
    assert_eq!(
        notes,
        ["columns ignored: it is empty", "date_format ignored: it is empty"]
    );
    // A null optional is absent everywhere, silently (the OpenAI adapters send every one).
    let (call, notes) = accept::<WordFormat>(json!({"doc": "d1", "bold": null}), None);
    assert_eq!((call.bold, notes.len()), (None, 0));
}

#[test]
fn single_as_list() {
    let (call, notes) = accept::<WordFormat>(json!({"doc": "d1", "targets": "p:A"}), None);
    assert_eq!(call.targets, Some(vec!["p:A".to_owned()]));
    assert!(notes.is_empty());
    let (call, _) = accept::<WordFormat>(json!({"doc": "d1", "find": {"text": "x"}}), None);
    assert_eq!(call.find.unwrap()[0].text, "x");
}

#[test]
fn map_as_pairs() {
    let (call, notes) = accept::<Rules>(
        json!({"doc": "d1", "pairs": {"Region": "region", "Total": "sum"}}),
        None,
    );
    let pairs = call.pairs.unwrap();
    assert_eq!(
        pairs[1],
        Match {
            table: "Total".into(),
            data: "sum".into()
        }
    );
    assert_eq!(notes, ["pairs given as an object: read as 2 pair(s) {table, data}"]);
}

#[test]
fn value_alias() {
    let (call, notes) = accept::<Rules>(json!({"doc": "d1", "lists": "Run On"}), None);
    assert_eq!(call.lists, Some(Lists::RunOn));
    assert_eq!(notes, ["lists 'Run On' read as 'run_on'"]);
    // Never sent: the canonical schema has no valueAliases; the validation schema has.
    let spec = Rules::spec();
    assert!(spec.schema().validation()["properties"]["lists"]["valueAliases"].is_object());
    assert!(spec.schema().canonical()["properties"]["lists"]
        .get("valueAliases")
        .is_none());
}

#[test]
fn argument_alias() {
    let (call, notes) = accept::<WordFormat>(
        json!({"doc": "d1", "top": 6, "find": [{"text": "x", "range": "p:A"}]}),
        None,
    );
    assert_eq!(call.space_before, Some(6.0));
    assert_eq!(call.find.unwrap()[0].within.as_deref(), Some("p:A"));
    // The arguments' own aliases first, then each nested object's, as today's order.
    assert_eq!(notes, ["top read as space_before", "find[0].range read as within"]);
    let aliases: Vec<String> = WordFormat::spec()
        .aliases()
        .iter()
        .map(ooxml_tools::Alias::key)
        .collect();
    assert_eq!(aliases, ["top", "bottom", "find[].range"]);
}

#[test]
fn default_for_a_required_argument() {
    let (call, notes) = accept::<ReplaceText>(json!({"doc": "d1", "find": "a", "replace": "b"}), None);
    assert_eq!(call.expect, Expect::One);
    assert_eq!(notes, ["expect not given: read as 'one'"]);
    // Still required in what is sent, so a strict decoder sends it.
    assert_eq!(
        ReplaceText::spec().schema().canonical()["required"],
        json!(["doc", "find", "replace", "expect"])
    );
}

#[test]
fn kind_neutral_fields() {
    let (call, notes) = accept::<Rules>(json!({"doc": "d1", "stories": "all"}), Some("xlsx"));
    assert_eq!(call.stories, None);
    assert_eq!(
        notes,
        ["stories ignored: it applies to pptx and docx, not to this xlsx document"]
    );
    // Another value asks something of the kind: kept, for the handler to refuse.
    let (call, notes) = accept::<Rules>(json!({"doc": "d1", "stories": "layouts"}), Some("xlsx"));
    assert_eq!((call.stories.as_deref(), notes.len()), (Some("layouts"), 0));
    // The empty values are neutral for every kind field.
    let (call, notes) = accept::<ReplaceText>(
        json!({"doc": "d1", "find": "a", "replace": "b", "expect": "all", "range": ""}),
        Some("pptx"),
    );
    assert_eq!((call.range, notes.len()), (None, 1));
}

#[test]
fn kind_fields_values_and_descriptions_in_what_is_sent() {
    let spec = ReplaceText::spec();
    let word = spec.canonical_for(Some(&["docx"]));
    assert!(word["properties"].get("slides").is_none());
    assert_eq!(word["properties"]["stories"]["enum"], json!(["body", "all"]));
    assert_eq!(
        word["properties"]["stories"]["description"],
        json!("Body (default) or all stories.")
    );
    let deck = spec.canonical_for(Some(&["pptx"]));
    assert!(deck["properties"].get("range").is_none());
    assert_eq!(
        deck["properties"]["stories"]["enum"],
        json!(["body", "all", "layouts", "masters"])
    );
    let both = spec.canonical_for(Some(&["docx", "pptx"]));
    assert_eq!(both, spec.canonical_for(None));
    // Still valid in a call: the whole schema is validated.
    let (call, _) = accept::<ReplaceText>(
        json!({"doc": "d1", "find": "a", "replace": "b", "expect": "all", "stories": "masters"}),
        Some("docx"),
    );
    assert_eq!(call.stories, Some(Stories::Masters));
}

#[test]
fn numbers_for_strings_nulls_for_items_and_free_objects() {
    let (call, _) = accept::<Rules>(
        json!({"doc": "d1", "grade": 3.5, "values": [1, null, 2.5], "slides": [2, "s:256"], "options": {"any": 1}}),
        None,
    );
    assert_eq!(call.grade, Some(NumOrStr::Num(3.5)));
    assert_eq!(call.values, Some(vec![Some(1.0), None, Some(2.5)]));
    assert_eq!(call.slides, Some(vec![IntOrStr::Int(2), IntOrStr::Str("s:256".into())]));
    assert_eq!(call.options.unwrap()["any"], json!(1));
    let validation = Rules::spec().schema().validation();
    assert_eq!(
        validation["properties"]["values"]["items"],
        json!({"type": "number", "orNull": true})
    );
    assert_eq!(
        validation["properties"]["grade"],
        json!({"type": "string", "description": "A grade: text or a number.", "orNumber": true})
    );
    // A whole number written as 2.0 is read as 2.
    let (call, _) = accept::<WordFormat>(json!({"doc": "d1", "find": [{"text": "x", "occurrence": 2.0}]}), None);
    assert_eq!(call.find.unwrap()[0].occurrence, Some(2));
}

#[test]
fn arguments_as_json_text() {
    let (call, _) = accept::<WordFormat>(json!("{\"doc\": \"d1\", \"bold\": true}"), None);
    assert_eq!(call.bold, Some(true));
    let (call, _) = accept::<WordSetText>(
        json!({"doc": "d1", "items": "[{\"target\": \"p:A\", \"text\": \"a\\vb\"}]"}),
        None,
    );
    assert_eq!(call.items.unwrap()[0].text, "a\u{b}b");
    let refused = WordSetText::accept(json!(""), None).unwrap_err();
    assert_eq!(refused.message(), "'doc' is required");
}

#[test]
fn exactly_one_of_a_group() {
    let mut spec = WordSetText::spec();
    spec.exactly_one.push(vec!["target".into(), "items".into()]);
    let error = spec.accept(json!({"doc": "d1"})).unwrap_err();
    assert_eq!(error.message(), "give exactly one of target, items");
    assert_eq!(error.field(), Some("target"));
    let error = spec
        .accept(json!({"doc": "d1", "target": "p:A", "items": []}))
        .unwrap_err();
    assert_eq!(error.message(), "give exactly one of target, items; got target, items");
    assert_eq!(error.field(), Some("items"));
    assert!(spec.accept(json!({"doc": "d1", "target": "p:A"})).is_ok());
}

#[test]
fn a_misplaced_attribute_is_a_definition_problem() {
    #[derive(ToolArgs)]
    #[tool(name = "toy_wrong", description = "Wrong attributes.")]
    #[allow(dead_code)]
    struct Wrong {
        /// Document id.
        doc: String,
        /// A flag.
        #[arg(minimum = 1, single_as_list)]
        flag: Option<bool>,
        /// Required, with empty_is_absent.
        #[arg(empty_is_absent)]
        name: String,
    }
    assert_eq!(
        Wrong::problems(),
        [
            "flag: minimum does not apply to a boolean",
            "flag: single_as_list does not apply to a boolean",
            "name: empty_is_absent on a required field",
        ]
    );
}

#[test]
fn calls_are_taken_on_many_threads_at_once() {
    // Nothing in the contract is shared or locked: specs are values, and the interpreter
    // lock (absent on 3.15t) is never needed for safety.
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ooxml_tools::ToolSpec>();
    assert_send_sync::<ooxml_tools::ToolError>();
    assert_send_sync::<ooxml_tools::ToolResult>();
    let threads: Vec<_> = (0..8)
        .map(|i| {
            std::thread::spawn(move || {
                let (call, notes) = WordFormat::accept(json!({"doc": format!("d{i}"), "top": i}), None).unwrap();
                assert_eq!(notes, ["top read as space_before"]);
                call.doc
            })
        })
        .collect();
    let docs: Vec<String> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(docs.len(), 8);
}
