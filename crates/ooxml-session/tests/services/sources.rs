//! Sources: a tool takes rows from a dataset (`d:<id>`, a run of its rows) or a table of
//! another open document (`t:<name>`), and a source that names no rows is refused with what
//! there is. ooxml-edit's `test_tools_sources.py` (the host's part: what the chart toy is
//! handed) and `test_source_refusals_r56.py` ported onto the toy session, each test named
//! after the one it ports. Every name and figure is made up.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::toy_session::{grid_sources, sales, Ctx, DeckHooks, Sess, Tool, NOW};
use ooxml_session::datasets::{DatasetLimits, Of};
use ooxml_session::facts::FactSources;
use ooxml_session::sources::{refusal, Refusing};
use ooxml_tools::datasets::Dataset;
use ooxml_tools::sources::Cell;
use ooxml_tools::{Failure, ToolError, TryCall};
use serde_json::{json, Map, Value};
use std::sync::Arc;

/// The toy placing tool (the host's part of `edit_chart`'s `from`): the rows a source
/// names, where they came from, and the first column's values as placed.
fn place(ctx: &mut Ctx<'_>) -> Result<Value, ToolError> {
    let from = ctx.arguments["from"].as_str().unwrap().to_owned();
    let columns: Vec<String> = ctx.arguments["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_owned())
        .collect();
    let retry = ctx.retry.clone();
    ctx.sources(|sources| {
        let rows = sources.source(&from, "from", false)?;
        let at: Vec<usize> = columns
            .iter()
            .map(|c| rows.column(c, "columns", &retry))
            .collect::<Result<_, _>>()?;
        let all: Vec<usize> = (0..rows.rows.len()).collect();
        let provenance = |k: usize| {
            let mut found = rows.provenance(k, &all);
            if let Some(Value::Array(given)) = found.get("sources") {
                let given = given.iter().map(|s| s.as_str().unwrap().to_owned()).collect();
                found.insert("sources".into(), Value::from(sources.merge(&rows, given)));
            }
            Value::Object(found)
        };
        let categories: Vec<Value> = rows.rows.iter().map(|row| row[at[0]].to_json()).collect();
        Ok(
            json!({"from": rows.describe(), "categories": provenance(at[0]), "series": [provenance(at[1])],
                  "placed": categories}),
        )
    })
}

const PLACE: Tool = Tool {
    name: "edit_chart",
    datasets: &[],
    sources: &["from"],
    run: place,
};

/// Keep a made-up dataset of the sales table, derived from the grid `d2` (`plain`: without
/// sources, and no document).
fn ds_put(ctx: &mut Ctx<'_>) -> Result<Value, ToolError> {
    let id = ctx.arguments["id"].as_str().unwrap().to_owned();
    let plain = ctx.arguments.contains_key("plain");
    let (header, rows) = sales();
    let mut dataset = Dataset::new(header, rows).unwrap().with_keys(1).unwrap();
    if !plain {
        dataset = dataset.with_sources(grid_sources(4, 3)).unwrap();
    }
    let meta = (!plain).then(|| ctx.sess.meta("d2"));
    let of = meta.as_ref().map_or(Of::Nothing, Of::Doc);
    let store = &ctx.sess.store;
    ctx.datasets
        .define(&id, Arc::new(dataset), of, None, &[], store, NOW, "id", &ctx.retry)?;
    Ok(json!({}))
}

const PUT: Tool = Tool {
    name: "ds_put",
    datasets: &[],
    sources: &[],
    run: ds_put,
};

/// The deck's hint (`_deck_hint`): a drawing's id given for a dataset.
fn deck_hint(value: &str) -> Option<String> {
    let rest = value.strip_prefix("d:")?;
    if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
        return Some(format!("{value} is one of the deck's drawings, not a dataset"));
    }
    (value == "t:blank").then(String::new)
}

/// The deck's own table (`_own`): `t:own`, and anything under it.
fn deck_own(address: &str) -> Option<String> {
    (address.split('/').next() == Some("t:own")).then(|| "a table".to_owned())
}

/// `d1` a deck, then `grids` grids of the sales table (`d2`, `d3`).
fn session_with(grids: usize, deck: DeckHooks, limits: DatasetLimits) -> Sess {
    let mut sess = Sess::new(limits);
    sess.deck = deck;
    sess.listing = true;
    sess.open("pptx", "deck.pptx");
    for n in 0..grids {
        sess.adopt(&[("sales", sales())], &format!("book{n}.xlsx"));
    }
    sess
}

/// The tests' session: a deck and a grid of `sales` and `words`.
fn session() -> Sess {
    let mut sess = Sess::new(DatasetLimits::default());
    sess.open("pptx", "deck.pptx");
    let words = (
        vec!["k".to_owned(), "v".to_owned()],
        vec![vec![Cell::Text("a".into()), Cell::Text("x".into())]],
    );
    sess.adopt(&[("sales", sales()), ("words", words)], "book.xlsx");
    sess
}

fn add(sess: &mut Sess, from: &str) -> crate::toy_session::Reply {
    let arguments = json!({"doc": "d1", "target": "1", "action": "add", "chart_type": "column",
                           "from": from, "columns": ["region", "units"]});
    sess.call(PLACE, arguments)
}

fn put(sess: &mut Sess, id: &str) {
    assert!(sess.call(PUT, json!({"id": id})).ok());
}

fn placed(sess: &mut Sess, from: &str) -> Value {
    let reply = add(sess, from);
    assert!(reply.ok(), "{from}: {:?}", reply.error);
    reply.data
}

/// `test_add_from_a_dataset_passes_literal_values_and_reports_their_sources`.
#[test]
fn add_from_a_dataset_passes_literal_values_and_reports_their_sources() {
    let mut sess = session();
    put(&mut sess, "sales");
    let data = placed(&mut sess, "d:sales");
    assert_eq!(data["placed"], json!(["North", "South", "East"]));
    assert_eq!(data["from"]["source"], "d:sales");
    assert_eq!(data["from"]["doc"], "d2");
    assert_eq!(
        data["categories"],
        json!({"column": "region", "rows": "1-3", "sources": ["G!A2:A4"]})
    );
    assert_eq!(
        data["series"],
        json!([{"column": "units", "rows": "1-3", "sources": ["G!B2:B4"]}])
    );
}

/// `test_add_from_a_table_of_the_one_open_grid`.
#[test]
fn add_from_a_table_of_the_one_open_grid() {
    let mut sess = session();
    let reply = sess.call(
        PLACE,
        json!({"doc": "d1", "from": "t:sales", "columns": ["Region", "units"]}),
    );
    assert!(reply.ok(), "{:?}", reply.error);
    let data = reply.data;
    assert_eq!(
        (&data["from"]["source"], &data["from"]["doc"]),
        (&json!("d2/t:sales"), &json!("d2"))
    );
    assert_eq!(data["from"]["version"], sess.version("d2"));
    assert_eq!(data["series"][0]["sources"], json!(["G!B2:B4"]));
    assert_eq!(data["placed"], json!(["North", "South", "East"]));
}

/// `test_a_table_read_from_invalidated_formula_caches_is_placed_stale`.
#[test]
fn a_table_read_from_invalidated_formula_caches_is_placed_stale() {
    let mut sess = session();
    let caches = |sess: &mut Sess, counts: Vec<(&'static str, i64)>| {
        sess.grids["d2"].lock().unwrap().caches.insert("sales".into(), counts);
    };
    caches(&mut sess, vec![("cached_unverified", 1), ("external", 1)]);
    let from = &placed(&mut sess, "t:sales")["from"];
    assert_eq!(from["cache"], json!({"cached_unverified": 1, "external": 1}));
    assert!(from.get("stale").is_none());
    caches(&mut sess, vec![("invalidated", 3)]);
    let from = &placed(&mut sess, "t:sales")["from"];
    assert_eq!(
        (&from["cache"], &from["stale"]),
        (&json!({"invalidated": 3}), &json!(true))
    );
}

/// `test_a_qualified_table_names_its_document`.
#[test]
fn a_qualified_table_names_its_document() {
    let mut sess = session();
    sess.adopt(&[("sales", sales())], "other.xlsx"); // d3
    let reply = add(&mut sess, "t:sales");
    let error = reply.failed("ambiguous");
    assert_eq!(error["valid_options"], json!(["d2/t:sales", "d3/t:sales"]));
    assert_eq!(error["field"], "from");
    // Round 56 (R120): the refusal read both to offer only tables that are there.
    for doc in ["d2", "d3"] {
        sess.grids[doc].lock().unwrap().reads = 0;
    }
    placed(&mut sess, "d3/t:sales");
    assert_eq!(
        (
            sess.grids["d3"].lock().unwrap().reads,
            sess.grids["d2"].lock().unwrap().reads
        ),
        (1, 0)
    );
}

/// `test_a_source_that_names_nothing_is_refused`.
#[test]
fn a_source_that_names_nothing_is_refused() {
    for (source, code) in [
        ("t:nothing", "not_found"),           // the format's own refusal
        ("Sales!A1:B4", "invalid_arguments"), // no format takes it
        ("d9/t:sales", "not_found"),          // no such document
        ("d1/t:sales", "invalid_arguments"),  // a document whose format takes no t:
        ("d:none", "not_found"),              // no such dataset
    ] {
        let mut sess = session();
        add(&mut sess, source).failed(code);
    }
    let error = add(&mut session(), "Sales!A1:B4").failed("invalid_arguments").clone();
    assert_eq!(
        error["message"],
        "'Sales!A1:B4' is not a source: give a dataset (d:<id>) or an address of an open document (t:...)"
    );
    // the shapes it offers name no source: none is left, and the call as given is the way out
    assert_eq!(error["valid_options"], json!([]));
    assert_eq!(error["try_call"]["tool"], "edit_chart");
}

/// `test_no_open_grid_means_no_table`.
#[test]
fn no_open_grid_means_no_table() {
    let mut sess = Sess::new(DatasetLimits::default());
    sess.open("pptx", "deck.pptx");
    let reply = add(&mut sess, "t:sales");
    let error = reply.failed("not_found");
    assert_eq!(error["message"], "no open document takes t: addresses: open one first");
}

/// `test_part_of_a_dataset_is_not_a_source`.
#[test]
fn part_of_a_dataset_is_not_a_source() {
    let mut sess = session();
    put(&mut sess, "sales");
    let reply = add(&mut sess, "d:sales[units]");
    let error = reply.failed("invalid_arguments");
    assert_eq!(error["valid_options"], json!(["d:sales", "d:sales/#1:"]));
}

/// `test_a_run_of_a_datasets_rows_is_a_source`.
#[test]
fn a_run_of_a_datasets_rows_is_a_source() {
    for (source, address, categories, rows, cells) in [
        (
            "d:sales/#2:#3",
            "d:sales/#2:#3",
            json!(["South", "East"]),
            "1-2",
            json!(["G!A3:A4"]),
        ),
        (
            "d:sales/#3:#2",
            "d:sales/#2:#3",
            json!(["South", "East"]),
            "1-2",
            json!(["G!A3:A4"]),
        ),
        (
            "d:sales/#2:",
            "d:sales/#2:",
            json!(["South", "East"]),
            "1-2",
            json!(["G!A3:A4"]),
        ),
        ("d:sales/#1", "d:sales/#1", json!(["North"]), "1", json!(["G!A2"])),
        ("d:sales/#2:#2", "d:sales/#2", json!(["South"]), "1", json!(["G!A3"])),
    ] {
        let mut sess = session();
        put(&mut sess, "sales");
        let data = placed(&mut sess, source);
        assert_eq!(data["placed"], categories);
        assert_eq!(
            (&data["from"]["source"], &data["from"]["doc"]),
            (&json!(address), &json!("d2"))
        );
        assert_eq!(
            data["categories"],
            json!({"column": "region", "rows": rows, "sources": cells})
        );
    }
}

/// `test_a_row_past_a_datasets_last_is_refused_not_cut`.
#[test]
fn a_row_past_a_datasets_last_is_refused_not_cut() {
    for (source, options) in [
        ("d:sales/#2:#4", json!(["d:sales/#2:#3"])),
        ("d:sales/#4:", json!(["d:sales/#3"])),
        ("d:sales/#9", json!(["d:sales/#3"])),
    ] {
        let mut sess = session();
        put(&mut sess, "sales");
        let reply = add(&mut sess, source);
        let error = reply.failed("invalid_arguments");
        let message = error["message"].as_str().unwrap();
        assert!(
            message.contains("has 3 rows") && message.contains("nothing is cut"),
            "{message}"
        );
        assert_eq!((&error["valid_options"], &error["field"]), (&options, &json!("from")));
    }
}

/// `test_a_dataset_without_sources_reports_its_rows`.
#[test]
fn a_dataset_without_sources_reports_its_rows() {
    let mut sess = session();
    assert!(sess.call(PUT, json!({"id": "plain", "plain": "yes"})).ok());
    let data = placed(&mut sess, "d:plain");
    assert_eq!(data["series"], json!([{"column": "units", "rows": "1-3"}]));
}

/// `test_a_stale_dataset_warns`.
#[test]
fn a_stale_dataset_warns() {
    let mut sess = session();
    put(&mut sess, "sales");
    sess.store.find_mut("sales").unwrap().mark_stale();
    let reply = add(&mut sess, "d:sales");
    assert!(reply.ok());
    assert_eq!(reply.data["from"]["stale"], true);
    assert!(reply.warnings.iter().any(|w| w.contains("d:sales is stale")));
}

/// `test_a_resolved_table_is_held_to_the_dataset_limit`.
#[test]
fn a_resolved_table_is_held_to_the_dataset_limit() {
    let limits = DatasetLimits {
        max_cells: 5,
        ..DatasetLimits::default()
    };
    let mut sess = session_with(1, DeckHooks::default(), limits);
    let reply = add(&mut sess, "t:sales");
    let error = reply.failed("limit");
    assert_eq!(
        error["message"],
        "d2/t:sales has 12 values; at most 5: derive a smaller dataset"
    );
}

fn set_from(sess: &mut Sess, value: &str) -> crate::toy_session::Reply {
    let arguments = json!({"doc": "d1", "target": "c1", "action": "set_values", "series": "units",
                           "from": value, "columns": ["region", "units"]});
    sess.call(PLACE, arguments)
}

const HINTING: DeckHooks = DeckHooks {
    hint: Some(deck_hint),
    own: None,
};

/// `test_without_a_hint_the_message_is_unchanged`.
#[test]
fn without_a_hint_the_message_is_unchanged() {
    let mut sess = session();
    let reply = set_from(&mut sess, "d:3");
    let error = reply.failed("not_found");
    assert_eq!(
        (&error["message"], &error["field"]),
        (&json!("no dataset d:3"), &json!("from"))
    );
    let mut alone = Sess::new(DatasetLimits::default());
    alone.open("pptx", "deck.pptx");
    let reply = set_from(&mut alone, "t:sales");
    let error = reply.failed("not_found");
    assert_eq!(error["message"], "no open document takes t: addresses: open one first");
}

/// `test_the_call_documents_format_adds_its_hint_to_a_missing_dataset`.
#[test]
fn the_call_documents_format_adds_its_hint_to_a_missing_dataset() {
    let mut sess = session_with(1, HINTING, DatasetLimits::default());
    put(&mut sess, "sales");
    let reply = set_from(&mut sess, "d:3");
    let error = reply.failed("not_found");
    assert_eq!(
        error["message"],
        "no dataset d:3; d:3 is one of the deck's drawings, not a dataset"
    );
    assert_eq!(
        (&error["field"], &error["valid_options"]),
        (&json!("from"), &json!(["d:sales"]))
    );
    assert_eq!(*sess.asked.lock().unwrap(), ["d:3"]);
}

/// `test_the_hint_reaches_a_t_address_no_document_takes`.
#[test]
fn the_hint_reaches_a_t_address_no_document_takes() {
    let mut sess = session_with(0, HINTING, DatasetLimits::default());
    let reply = set_from(&mut sess, "t:sales");
    let error = reply.failed("not_found");
    assert!(error["message"]
        .as_str()
        .unwrap()
        .starts_with("no open document takes t: addresses: open one first"));
    assert_eq!(*sess.asked.lock().unwrap(), ["t:sales"]);
    let reply = set_from(&mut sess, "t:blank"); // nothing to say: as it was
    let error = reply.failed("not_found");
    assert_eq!(error["message"], "no open document takes t: addresses: open one first");
}

/// `test_a_hint_never_changes_what_resolves`.
#[test]
fn a_hint_never_changes_what_resolves() {
    let mut sess = session_with(1, HINTING, DatasetLimits::default());
    put(&mut sess, "sales");
    for value in ["d:sales", "t:sales"] {
        assert!(set_from(&mut sess, value).ok());
    }
    set_from(&mut sess, "d2/t:none").failed("not_found");
    set_from(&mut sess, "x:1").failed("invalid_arguments");
    assert!(sess.asked.lock().unwrap().is_empty()); // asked only when the dispatcher finds nothing to resolve
}

const OWN: DeckHooks = DeckHooks {
    hint: None,
    own: Some(deck_own),
};

fn r56(grids: usize) -> Sess {
    session_with(grids, OWN, DatasetLimits::default())
}

/// `test_the_calls_own_table_is_said_to_be_one`.
#[test]
fn the_calls_own_table_is_said_to_be_one() {
    for value in ["t:own", "d1/t:own", "t:own/#2"] {
        let mut sess = r56(1);
        let reply = add(&mut sess, value);
        let error = reply.failed("invalid_arguments").clone();
        let address = value.strip_prefix("d1/").unwrap_or(value);
        let message = error["message"].as_str().unwrap();
        assert!(
            message.starts_with(&format!(
                "{address} is a table of d1, the document this call edits, not rows to take: from takes a dataset \
                 (d:<id>) or rows of an open xlsx document (t:<name>)"
            )),
            "{message}"
        );
        assert!(message.contains("there are tables t:sales") && message.contains(r#"e.g. from: "t:sales""#));
        assert_eq!(
            (&error["field"], &error["valid_options"]),
            (&json!("from"), &json!(["t:sales"]))
        );
        assert_eq!(
            error["details"]["names"],
            json!({"doc": "d1", "address": address, "is": "a table"})
        );
        // One source alone: the call that works.
        let meant = &error["details"]["call"];
        assert_eq!(
            (&meant["tool"], &meant["arguments"]["from"]),
            (&json!("edit_chart"), &json!("t:sales"))
        );
        assert!(sess.call(PLACE, meant["arguments"].clone()).ok());
    }
}

/// `test_datasets_and_tables_are_both_offered`.
#[test]
fn datasets_and_tables_are_both_offered() {
    let mut sess = r56(2);
    put(&mut sess, "sales");
    let reply = add(&mut sess, "t:own");
    let error = reply.failed("invalid_arguments").clone();
    assert_eq!(error["valid_options"], json!(["d:sales", "d2/t:sales", "d3/t:sales"]));
    assert!(error["message"]
        .as_str()
        .unwrap()
        .contains("there are datasets d:sales and tables d2/t:sales, d3/t:sales"));
    assert!(error["details"].get("call").is_none()); // three sources: none is the obvious one
    for option in error["valid_options"].as_array().unwrap() {
        assert!(add(&mut sess, option.as_str().unwrap()).ok(), "{option}");
    }
}

/// `test_none_there_says_how_to_get_one`.
#[test]
fn none_there_says_how_to_get_one() {
    let mut sess = r56(0);
    let reply = add(&mut sess, "t:own");
    let error = reply.failed("invalid_arguments");
    assert!(error["message"]
        .as_str()
        .unwrap()
        .ends_with("there is none: define a dataset or open a document with tables first"));
    assert_eq!(error["valid_options"], json!([]));
}

/// `test_every_option_offered_resolves`.
#[test]
fn every_option_offered_resolves() {
    for grids in [1, 2] {
        for value in [
            "d1/t:sales",
            "d1/t:nothing",
            "d9/t:sales",
            "d9/t:nothing",
            "t:nothing",
            "Sales!A1:B4",
            "d2/t:nothing",
            "t:raise",
        ] {
            let mut sess = r56(grids);
            put(&mut sess, "sales");
            let reply = add(&mut sess, value);
            assert!(!reply.ok());
            let options = reply.error.as_ref().unwrap()["valid_options"]
                .as_array()
                .unwrap()
                .clone();
            assert!(!options.is_empty(), "{value}: {:?}", reply.error);
            for option in options {
                let option = option.as_str().unwrap();
                assert!(
                    add(&mut sess, option).ok(),
                    "{grids} {value} {option}: {:?}",
                    reply.error
                );
            }
        }
    }
}

/// `test_a_document_taking_no_t_never_offers_a_table_that_is_not_there`.
#[test]
fn a_document_taking_no_t_never_offers_a_table_that_is_not_there() {
    let mut sess = r56(1);
    let reply = add(&mut sess, "d1/t:missing");
    let error = reply.failed("invalid_arguments");
    assert!(error["message"]
        .as_str()
        .unwrap()
        .starts_with("d1 (pptx) takes no t: addresses"));
    assert_eq!(error["valid_options"], json!(["t:sales"]));
    // What is there, a bare address while one grid takes t:.
    let reply = add(&mut sess, "d1/t:sales");
    assert_eq!(reply.failed("invalid_arguments")["valid_options"], json!(["t:sales"]));
    let mut two = r56(2);
    let reply = add(&mut two, "d1/t:sales");
    assert_eq!(
        reply.failed("invalid_arguments")["valid_options"],
        json!(["d2/t:sales", "d3/t:sales"])
    );
}

/// `test_a_missing_table_offers_the_datasets_too`.
#[test]
fn a_missing_table_offers_the_datasets_too() {
    let mut sess = r56(1);
    put(&mut sess, "sales");
    let reply = add(&mut sess, "t:nothing");
    let error = reply.failed("not_found");
    assert_eq!(error["message"], "no table 'nothing'; or a dataset: d:sales");
    assert_eq!(error["valid_options"], json!(["t:sales", "d:sales"]));
}

/// `test_a_format_without_the_hook_keeps_its_words`.
#[test]
fn a_format_without_the_hook_keeps_its_words() {
    let mut sess = session_with(1, DeckHooks::default(), DatasetLimits::default());
    let reply = add(&mut sess, "t:own");
    let error = reply.failed("not_found");
    assert_eq!(
        (&error["message"], &error["valid_options"]),
        (&json!("no table 'own'"), &json!(["t:sales"]))
    );
}

/// `test_a_failing_hook_is_ignored` (a Rust hook does not fail: one that says nothing).
#[test]
fn a_hook_that_says_nothing_is_ignored() {
    let mut sess = r56(1);
    let reply = add(&mut sess, "t:raise");
    assert_eq!(reply.failed("not_found")["message"], "no table 'raise'");
    assert!(sess.asked.lock().unwrap().iter().any(|a| a == "t:raise"));
}

/// `test_limit_and_other_codes_stand`.
#[test]
fn limit_and_other_codes_stand() {
    let sess = r56(1);
    let error = ToolError::failure(Failure::Limit, "too many");
    let retry = TryCall {
        tool: "x".into(),
        arguments: Map::new(),
        fill: Vec::new(),
    };
    let found = refusal(error.clone(), "t:x", &sess, &[], &Refusing::default(), &retry);
    assert_eq!(found, error);
}

/// A document whose format takes no rows (`FormatDoc::source`'s default, as docx's) is
/// refused as `sources.resolve` words it, through the formats' `FormatSources`.
#[test]
fn a_format_taking_no_rows_is_refused_by_its_kind() {
    use ooxml_session::sources::SourceHost;
    let sess = session();
    let error = sess.resolve("d1", "t:sales").unwrap_err().to_json();
    assert_eq!(error["code"], "invalid_arguments");
    assert_eq!(error["message"], "pptx documents take no such address: 't:sales'");
    assert_eq!(sess.documents()[1].kind, ooxml_session::DocKind::Xlsx);
    assert_eq!(sess.prefixes().of("xlsx"), ["t:".to_owned()]);
}
