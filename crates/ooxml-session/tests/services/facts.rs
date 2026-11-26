//! Text facts, on made-up sources: ooxml-edit's `test_tools_textfacts.py` ported, each test
//! named after the one it ports. The host's part -- a dataset by its id, a table of another
//! open document by its address -- is the session's: its dataset store and a call's sources
//! (`CallSources`) over a toy grid, as the Python tests' `ds_keep` and grid format are;
//! every figure is made up.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_numfmt::{Locale, EN_US};
use ooxml_session::datasets::{CallDatasets, DatasetStore, DocMeta, Of};
use ooxml_session::facts::{bad_format, expand_arguments, known_prefixes, quote, segments, Fact, FactEnv, FORMS};
use ooxml_session::sources::{CallSources, OpenDoc, SourceHost};
use ooxml_tools::datasets::Dataset;
use ooxml_tools::sources::{Cell, Day, Prefixes};
use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::sync::Arc;

/// The made-up table: item, rating, share, when.
fn checks() -> (Vec<&'static str>, Vec<Vec<Cell>>) {
    let row = |item: &str, rating: Cell, share: f64, when: &str| {
        vec![
            Cell::Text(item.into()),
            rating,
            Cell::Float(share),
            Cell::Text(when.into()),
        ]
    };
    (
        vec!["item", "rating", "share", "when"],
        vec![
            row("N-1", Cell::Float(2.166_666_666_666_666_5), 0.253, "2026-03-01"),
            row("A/7", Cell::Int("3".into()), 0.5, "2026-04-02"),
            row("N-2", Cell::Float(1_234_567.891), 1.0, "2026-05-03"),
        ],
    )
}

/// The host's part: dataset `by_item` (and `odd`) in the session's store, the table
/// `checks` of the grid `d2` when it is open.
struct Toy {
    with_table: bool,
    odd: bool,
}

const VERSION: u64 = 4;
const NAME: &str = "checks-made-up.xlsx";

fn refuse(code: Refusal, message: String, options: Vec<String>) -> ToolError {
    ToolError::refusal(
        code,
        message,
        WayOut::names(options).unwrap_or_else(|| WayOut::one(Value::Null)),
    )
}

/// The open documents and the grid format's resolver.
struct Host {
    docs: Vec<OpenDoc>,
    prefixes: Prefixes,
}

fn heads(doc: &str) -> Option<u64> {
    (doc == "d2").then_some(VERSION)
}

fn columns() -> Vec<String> {
    checks().0.iter().map(|c| (*c).to_owned()).collect()
}

impl Toy {
    /// The session's datasets: `by_item` derived from `d2`, and `odd` (no document).
    fn store(&self) -> DatasetStore {
        let (names, rows) = checks();
        let sources = rows
            .iter()
            .map(|row| {
                let Cell::Text(item) = &row[0] else { unreachable!() };
                names.iter().map(|c| vec![format!("t:checks/{item}/{c}")]).collect()
            })
            .collect();
        let by_item = Dataset::new(columns(), rows)
            .unwrap()
            .with_types(vec![
                None,
                Some("number".into()),
                Some("percent".into()),
                Some("date".into()),
            ])
            .unwrap()
            .with_sources(sources)
            .unwrap();
        let mut store = DatasetStore::default();
        let mut call = CallDatasets::new();
        let meta = DocMeta {
            doc: "d2".into(),
            kind: ooxml_session::DocKind::Xlsx,
            name: NAME.into(),
        };
        let retry = retry();
        call.define(
            "by_item",
            Arc::new(by_item),
            Of::Doc(&meta),
            None,
            &[],
            &store,
            "now",
            "id",
            &retry,
        )
        .unwrap();
        if self.odd {
            let odd = Dataset::new(
                vec!["item".into(), "avg:0.20".into()],
                vec![vec![Cell::Text("N-1".into()), Cell::Int("7".into())]],
            )
            .unwrap();
            call.define(
                "odd",
                Arc::new(odd),
                Of::Nothing,
                None,
                &[],
                &store,
                "now",
                "id",
                &retry,
            )
            .unwrap();
        }
        call.finish(&mut store, &heads).unwrap();
        store
    }

    fn host(&self) -> Host {
        let mut docs = vec![OpenDoc {
            handle: "d1".into(),
            kind: ooxml_session::DocKind::Docx,
            name: "brief.docx".into(),
        }];
        if self.with_table {
            docs.push(OpenDoc {
                handle: "d2".into(),
                kind: ooxml_session::DocKind::Xlsx,
                name: NAME.into(),
            });
        }
        Host {
            docs,
            prefixes: Prefixes::new([("xlsx", vec!["t:"]), ("docx", vec![])]),
        }
    }
}

impl SourceHost for Host {
    fn documents(&self) -> Vec<OpenDoc> {
        self.docs.clone()
    }

    fn prefixes(&self) -> &Prefixes {
        &self.prefixes
    }

    /// `t:checks/<key>` of `d2`, as the grid format resolves it (`_table`).
    fn resolve(&self, _doc: &str, address: &str) -> Result<Dataset, ToolError> {
        let (names, rows) = checks();
        let body = address.strip_prefix("t:").unwrap_or(address);
        let (name, key) = body.split_once('/').unwrap_or((body, ""));
        if name != "checks" {
            return Err(refuse(
                Refusal::NotFound,
                format!("no table '{name}'"),
                vec!["checks".into()],
            ));
        }
        let key = segments(key).first().map(|(k, _)| k.clone()).unwrap_or_default();
        let numbered: Vec<(usize, Vec<Cell>)> = rows
            .into_iter()
            .enumerate()
            .filter(|(i, row)| {
                key.is_empty()
                    || if key.starts_with('#') {
                        format!("#{}", i + 1) == key
                    } else {
                        row[0] == Cell::Text(key.clone())
                    }
            })
            .collect();
        if !key.is_empty() && numbered.len() != 1 {
            return Err(refuse(
                Refusal::NotFound,
                format!("no row '{key}' in {name}"),
                vec!["checks".into()],
            ));
        }
        let letter = |c: usize| char::from(b'A' + u8::try_from(c).unwrap());
        let sources = numbered
            .iter()
            .map(|(i, _)| {
                (0..names.len())
                    .map(|c| vec![format!("G!{}{}", letter(c), i + 2)])
                    .collect()
            })
            .collect();
        Ok(
            Dataset::new(columns(), numbered.into_iter().map(|(_, row)| row).collect())
                .unwrap()
                .with_sources(sources)
                .unwrap(),
        )
    }
}

fn retry() -> TryCall {
    TryCall {
        tool: "toy_say".into(),
        arguments: Map::new(),
        fill: Vec::new(),
    }
}

fn nl_be() -> Locale {
    Locale::new(',', '.', "dmy", "nl-BE", ';').unwrap()
}

fn language(tag: &str) -> Locale {
    Locale::new(',', '.', "dmy", tag, ';').unwrap()
}

type Said = Result<(Value, Vec<Fact>), ToolError>;

/// `toy_say`: the texts as written, and what was written where; its references read by
/// the host's sources (the session's datasets, the grid the call holds).
fn say_in(locale: &Locale, toy: &Toy, texts: &[&str], rows: Option<Value>) -> Said {
    let mut arguments = json!({"doc": "d1", "items": texts.iter().map(|t| json!({"text": t})).collect::<Vec<_>>()});
    if let Some(rows) = rows {
        arguments["rows"] = rows;
    }
    let known = known_prefixes(&["t:"], &[]);
    let store = toy.store();
    let host = toy.host();
    let datasets: Vec<String> = store.entries().iter().map(|e| e.id.clone()).collect();
    let retry = retry();
    let env = FactEnv {
        known: &known,
        own: &[],
        datasets: &datasets,
        locale,
        retry: &retry,
    };
    let mut call = CallDatasets::new();
    let given = arguments.as_object().unwrap().clone();
    let held = vec!["d2".to_owned()];
    let sources = CallSources {
        host: &host,
        store: &store,
        datasets: RefCell::new(&mut call),
        heads: &heads,
        tool: "toy_say",
        arguments: &given,
        primary: Some("d1"),
        held: &held,
        retry: &retry,
        own: None,
    };
    expand_arguments(&arguments, &["items[].text".into(), "rows[]".into()], &env, &sources)
}

const TOY: Toy = Toy {
    with_table: true,
    odd: false,
};

fn say(texts: &[&str]) -> Said {
    say_in(&EN_US, &TOY, texts, None)
}

fn said(texts: &[&str]) -> String {
    let (arguments, _) = say(texts).unwrap();
    arguments["items"][0]["text"].as_str().unwrap().to_owned()
}

fn failed(found: Said, code: &str) -> Value {
    let error = found.unwrap_err().to_json();
    assert_eq!(error["code"], code, "{error}");
    error
}

fn message(error: &Value) -> &str {
    error["message"].as_str().unwrap()
}

/// `test_a_dataset_value_is_written_with_its_sources`.
#[test]
fn a_dataset_value_is_written_with_its_sources() {
    let (arguments, facts) = say(&["Rated {{d:by_item/N-2/rating:#,##0.0}} at N-2"]).unwrap();
    assert_eq!(arguments["items"], json!([{"text": "Rated 1,234,567.9 at N-2"}]));
    let fact = &facts[0];
    assert_eq!(
        (
            fact.field.as_str(),
            fact.start,
            fact.end,
            fact.text.as_str(),
            fact.address.as_str()
        ),
        ("items[0].text", 6, 17, "1,234,567.9", "d:by_item/N-2/rating")
    );
    assert_eq!(
        Value::Object(fact.brief()),
        json!({"from": "d:by_item/N-2/rating", "text": "1,234,567.9", "sources": ["t:checks/N-2/rating"]})
    );
    assert_eq!(
        Value::Object(fact.record.clone()),
        json!({"source": "d:by_item/N-2/rating", "document": NAME, "version": VERSION, "format": "#,##0.0",
               "text": "1,234,567.9", "sources": ["t:checks/N-2/rating"]})
    );
    let keys: Vec<&str> = fact.record.keys().map(String::as_str).collect();
    assert_eq!(keys, ["source", "document", "version", "format", "text", "sources"]);
}

/// `test_nl_be_writes_a_decimal_comma_and_its_own_grouping`.
#[test]
fn nl_be_writes_a_decimal_comma_and_its_own_grouping() {
    let text = "Gemiddeld {{d:by_item/N-1/rating:0.0}}; {{d:by_item/N-2/rating:#,##0.00}}; {{d:by_item/N-1/share}}; \
                {{d:by_item/N-1/share:0.0%}}; {{d:by_item/N-1/when}}; {{d:by_item/#2/rating}}";
    let (arguments, _) = say_in(&nl_be(), &TOY, &[text], None).unwrap();
    assert_eq!(
        arguments["items"][0]["text"],
        "Gemiddeld 2,2; 1.234.567,89; 25,3%; 25,3%; 1/3/2026; 3"
    );
}

/// `test_without_a_format_a_number_is_written_as_a_table_cell_gets_it`.
#[test]
fn without_a_format_a_number_is_written_as_a_table_cell_gets_it() {
    assert_eq!(
        said(&["{{d:by_item/N-1/rating}} {{d:by_item/N-1/rating:0}}"]),
        "2.16666666666667 2"
    );
}

/// `test_a_table_value_of_another_document_is_read_by_its_row`.
#[test]
fn a_table_value_of_another_document_is_read_by_its_row() {
    let (arguments, facts) = say(&["Door: {{t:checks/'A/7'/rating}} ({{t:checks/#1/item}})"]).unwrap();
    assert_eq!(arguments["items"][0]["text"], "Door: 3 (N-1)");
    assert_eq!(
        Value::Object(facts[0].brief()),
        json!({"from": "t:checks/'A/7'/rating", "text": "3", "sources": ["G!B3"]})
    );
    assert_eq!(facts[0].record["source"], "t:checks/'A/7'/rating");
    assert_eq!(facts[0].record["document"], NAME);
}

/// `test_rows_of_cells_and_several_texts_each_say_where`.
#[test]
fn rows_of_cells_and_several_texts_each_say_where() {
    let rows = json!([["{{d:by_item/#3/item}}", "plain"]]);
    let (arguments, facts) = say_in(&EN_US, &TOY, &["a", "x {{d:by_item/N-1/item}}"], Some(rows)).unwrap();
    assert_eq!(arguments["rows"], json!([["N-2", "plain"]]));
    let fields: Vec<&str> = facts.iter().map(|f| f.field.as_str()).collect();
    assert_eq!(fields, ["items[1].text", "rows[0][0]"]);
}

/// `test_other_braces_stay_and_an_escape_writes_the_reference_literally`.
#[test]
fn other_braces_stay_and_an_escape_writes_the_reference_literally() {
    let (arguments, facts) = say(&["Dear {{name}}, {x:1} {{ q:1 }} \\{{d:by_item/N-1/item}}"]).unwrap();
    assert_eq!(
        arguments["items"][0]["text"],
        "Dear {{name}}, {x:1} {{ q:1 }} {{d:by_item/N-1/item}}"
    );
    assert!(facts.is_empty());
}

/// `test_a_reference_to_many_values_is_refused_with_an_example`.
#[test]
fn a_reference_to_many_values_is_refused_with_an_example() {
    let error = failed(say(&["{{d:by_item/rating}}"]), "invalid_arguments");
    assert!(message(&error).contains("not one value") && message(&error).contains("d:by_item/"));
    assert_eq!(error["field"], "items[0].text");
}

/// `test_an_unknown_dataset_row_or_column_is_not_found`.
#[test]
fn an_unknown_dataset_row_or_column_is_not_found() {
    let options = |error: &Value| error["valid_options"].as_array().unwrap().clone();
    let error = failed(say(&["{{d:nope/N-1/rating}}"]), "not_found");
    assert!(options(&error).contains(&json!("d:by_item")));
    let error = failed(say(&["{{d:by_item/N-9/rating}}"]), "not_found");
    assert!(options(&error).contains(&json!("d:by_item/N-1")));
    let error = failed(say(&["{{d:by_item/N-1/ratings}}"]), "not_found");
    assert!(options(&error).contains(&json!("rating")));
    failed(say(&["{{d:by_item/#9/rating}}"]), "not_found");
}

/// `test_a_format_on_text_is_refused`.
#[test]
fn a_format_on_text_is_refused() {
    let error = failed(say(&["{{d:by_item/N-1/item:0.0}}"]), "invalid_arguments");
    assert!(message(&error).contains("is for numbers"));
}

/// `test_t_without_an_open_table_document_is_not_found`.
#[test]
fn t_without_an_open_table_document_is_not_found() {
    let toy = Toy {
        with_table: false,
        odd: false,
    };
    failed(say_in(&EN_US, &toy, &["{{t:checks/N-1/rating}}"], None), "not_found");
}

/// `test_a_suffix_that_is_no_number_format_says_so_with_the_forms`.
#[test]
fn a_suffix_that_is_no_number_format_says_so_with_the_forms() {
    for (suffix, near) in [
        ("0.20", "0.00"),
        ("0,0", "0.0"),
        (".2f", "0.00"),
        ("#,###.0", "#,##0.0"),
        ("0.0 %", "0.0%"),
        ("00", "0"),
    ] {
        let error = failed(
            say(&[&format!("{{{{d:by_item/#1/rating:{suffix}}}}}")]),
            "invalid_arguments",
        );
        let message = message(&error);
        assert!(
            message.contains(&format!("'{}' is not a number format", suffix.trim())),
            "{message}"
        );
        assert!(!message.contains("no column"));
        assert!(
            message.contains(&format!("{{{{d:by_item/#1/rating:{near}}}}}")),
            "{message}"
        );
        assert_eq!(error["valid_options"], json!(FORMS));
        assert_eq!(error["field"], "items[0].text");
    }
}

/// `test_a_bad_suffix_on_an_unknown_column_or_a_source_says_the_same`.
#[test]
fn a_bad_suffix_on_an_unknown_column_or_a_source_says_the_same() {
    for text in ["{{d:by_item/#1/avg:0.20}}", "{{d2/t:checks/N-1/rating:0.20}}"] {
        let error = failed(say(&[text]), "invalid_arguments");
        assert!(message(&error).contains("'0.20' is not a number format"));
    }
}

/// `test_a_column_named_with_a_colon_still_resolves`.
#[test]
fn a_column_named_with_a_colon_still_resolves() {
    let toy = Toy {
        with_table: true,
        odd: true,
    };
    let (arguments, _) = say_in(
        &EN_US,
        &toy,
        &["{{d:odd/N-1/avg:0.20}}", "{{d:odd/N-1/'avg:0.20':0.0}}"],
        None,
    )
    .unwrap();
    assert_eq!(arguments["items"], json!([{"text": "7"}, {"text": "7.0"}]));
}

/// `test_a_quoted_column_or_a_plain_unknown_one_is_not_read_as_a_format`.
#[test]
fn a_quoted_column_or_a_plain_unknown_one_is_not_read_as_a_format() {
    let error = failed(say(&["{{d:by_item/#1/'avg:0.20'}}"]), "not_found");
    assert!(message(&error).contains("no column 'avg:0.20'"));
    let error = failed(say(&["{{d:by_item/#1/avg:total}}"]), "not_found");
    assert!(message(&error).contains("no column"));
    let error = failed(say(&["{{d:2024}}"]), "not_found");
    assert!(!message(&error).contains("not a number format"));
    assert_eq!(bad_format("d:2024"), None);
    assert_eq!(bad_format("d:x/#1/avg:0.0"), None);
}

/// `test_a_date_pattern_writes_the_locales_measured_names`: the names measured in Excel for
/// Mac (2026-10-16; a region's, 2026-10-17).
#[test]
fn a_date_pattern_writes_the_locales_measured_names() {
    let text = "{{d:by_item/N-1/when:dddd d mmmm yyyy}} ({{d:by_item/N-1/when:ddd}}, {{d:by_item/N-1/when:mmm}}, \
                {{d:by_item/N-1/when:mmmmm}})";
    for (tag, written) in [
        ("en-US", "Sunday 1 March 2026 (Sun, Mar, M)"),
        ("en-GB", "Sunday 1 March 2026 (Sun, Mar, M)"),
        ("nl-NL", "zondag 1 maart 2026 (zo, mrt, m)"),
        ("nl-BE", "zondag 1 maart 2026 (zo, mrt, m)"),
        ("de-DE", "Sonntag 1 März 2026 (So, Mär, M)"),
        ("fr-FR", "dimanche 1 mars 2026 (dim., mars, m)"),
        ("es-ES", "domingo 1 marzo 2026 (do., mar., m)"),
        ("it-IT", "domenica 1 marzo 2026 (dom, mar, m)"),
        ("en-BE", "Sunday 1 March 2026 (Sun, Mar, M)"),
        ("fr-BE", "dimanche 1 mars 2026 (dim., mars, m)"),
        ("de-CH", "Sonntag 1 März 2026 (So, Mär, M)"),
    ] {
        let (arguments, _) = say_in(&language(tag), &TOY, &[text], None).unwrap();
        assert_eq!(arguments["items"][0]["text"], written, "{tag}");
    }
}

/// `test_a_date_patterns_separators_are_written_as_given_in_every_locale`.
#[test]
fn a_date_patterns_separators_are_written_as_given_in_every_locale() {
    let text = "{{d:by_item/N-1/when:dd/mm/yyyy}} {{d:by_item/#3/when:d-m-yy}} {{d2/t:checks/'A/7'/when:yyyy.mm.dd}} \
                {{d:by_item/N-1/when:DD/MM/YYYY}}";
    for tag in ["en-US", "nl-BE", "de-DE", "ja-JP"] {
        let (arguments, _) = say_in(&language(tag), &TOY, &[text], None).unwrap();
        assert_eq!(
            arguments["items"][0]["text"], "01/03/2026 3-5-26 2026.04.02 01/03/2026",
            "{tag}"
        );
    }
}

/// `test_a_date_patterns_fact_keeps_the_pattern_as_its_format`.
#[test]
fn a_date_patterns_fact_keeps_the_pattern_as_its_format() {
    let (_, facts) = say(&["On {{d:by_item/N-1/when:d mmmm yyyy}}"]).unwrap();
    assert_eq!(
        Value::Object(facts[0].brief()),
        json!({"from": "d:by_item/N-1/when", "text": "1 March 2026", "sources": ["t:checks/N-1/when"]})
    );
    assert_eq!((facts[0].start, facts[0].end), (3, 15));
    assert_eq!(facts[0].record["format"], "d mmmm yyyy");
    assert_eq!(facts[0].record["source"], "d:by_item/N-1/when");
}

/// `test_a_name_token_without_measured_names_is_refused_numbers_still_work`.
#[test]
fn a_name_token_without_measured_names_is_refused_numbers_still_work() {
    let japanese = language("ja-JP");
    let error = say_in(&japanese, &TOY, &["{{d:by_item/N-1/when:d mmmm yyyy}}"], None)
        .unwrap_err()
        .to_json();
    assert!(message(&error).contains("no month or day names are measured for ja-JP"));
    assert!(message(&error).contains("dd/mm/yyyy"));
    assert_eq!(error["field"], "items[0].text");
    let (arguments, _) = say_in(&japanese, &TOY, &["{{d:by_item/N-1/when:yyyy-mm-dd}}"], None).unwrap();
    assert_eq!(arguments["items"][0]["text"], "2026-03-01");
}

/// `test_a_date_pattern_on_a_number_and_a_number_format_on_a_date_name_each_other`.
#[test]
fn a_date_pattern_on_a_number_and_a_number_format_on_a_date_name_each_other() {
    let error = failed(say(&["{{d:by_item/N-1/rating:d mmmm yyyy}}"]), "invalid_arguments");
    assert!(message(&error).contains("a date pattern (:d mmmm yyyy) is for dates") && message(&error).contains(":0.0"));
    let error = failed(say(&["{{d:by_item/N-1/when:0.0}}"]), "invalid_arguments");
    assert!(message(&error).contains("a number format (:0.0) is for numbers"));
    assert!(message(&error).contains("date pattern") && message(&error).contains("mmmm"));
    let error = failed(say(&["{{d:by_item/N-1/item:dd/mm/yyyy}}"]), "invalid_arguments");
    assert!(message(&error).contains("is for dates"));
}

/// `test_a_suffix_that_is_no_date_pattern_says_so_with_the_tokens`.
#[test]
fn a_suffix_that_is_no_date_pattern_says_so_with_the_tokens() {
    for (suffix, why) in [
        ("DD/MM/YYYY HH:mm", "'H' is no date token"),
        ("%d-%m-%Y", "no date token"),
        ("ddddd mmmm yyyy", "'ddddd' is no date token"),
        ("d mmmm yyy", "'yyy' is no date token"),
    ] {
        let error = failed(
            say(&[&format!("{{{{d:by_item/#1/when:{suffix}}}}}")]),
            "invalid_arguments",
        );
        let message = message(&error);
        assert!(
            message.contains(&format!("'{suffix}' is not a date pattern")),
            "{message}"
        );
        assert!(message.contains(why), "{message}");
        assert!(message.contains("{{d:by_item/#1/when:d mmmm yyyy}}"), "{message}");
        assert_eq!(error["valid_options"], json!(ooxml_tools::sources::TOKENS));
    }
}

/// `test_the_number_format_refusal_lists_the_date_tokens_too`.
#[test]
fn the_number_format_refusal_lists_the_date_tokens_too() {
    let error = failed(say(&["{{d:by_item/#1/rating:0.20}}"]), "invalid_arguments");
    assert!(message(&error).contains("d dd ddd dddd m mm mmm mmmm mmmmm yy yyyy"));
    assert!(message(&error).contains(":d mmmm yyyy"));
}

/// `test_without_a_pattern_a_date_is_written_as_before`.
#[test]
fn without_a_pattern_a_date_is_written_as_before() {
    for (locale, written) in [(EN_US, "3/1/2026"), (nl_be(), "1/3/2026")] {
        let (arguments, _) = say_in(&locale, &TOY, &["{{d:by_item/N-1/when}}"], None).unwrap();
        assert_eq!(arguments["items"][0]["text"], written);
    }
}

/// Round 50 (`test_tools_textfacts_shape.py`'s cases): braces shaped like a reference that
/// names no source are refused with the nearest one; other braces stay text.
#[test]
fn braces_shaped_like_a_reference_are_refused_with_the_nearest() {
    let error = failed(say(&["{{by_item/#1/rating}}"]), "invalid_arguments");
    assert!(
        message(&error).contains("did you mean {{d:by_item/#1/rating}}?"),
        "{}",
        message(&error)
    );
    assert!(message(&error).contains("\\{{by_item/#1/rating}}"));
    let error = failed(say(&["{{D:by_item/#1/rating}}"]), "invalid_arguments");
    assert!(error["valid_options"]
        .as_array()
        .unwrap()
        .contains(&json!("d:by_item/#1/rating")));
    let error = failed(say(&["{{ds:by_item}}"]), "invalid_arguments");
    assert!(message(&error).contains("{{d:by_item}}"));
    for text in ["{{name}}", "{{ client.name }}", "{{TODO: see a/b}}", "{{Note: x/y}}"] {
        assert_eq!(said(&[text]), text);
    }
    assert_eq!(said(&["\\{{by_item/#1/rating}}"]), "{{by_item/#1/rating}}");
}

/// More than 50 references in one text is a limit.
#[test]
fn more_than_fifty_references_in_one_text_is_refused() {
    let text = "{{d:by_item/#1/item}} ".repeat(51);
    let error = failed(say(&[&text]), "limit");
    assert_eq!(error["message"], "more than 50 references in one text");
}

/// A key written as a reference writes it (`quote`), and a day is a `Day`.
#[test]
fn keys_are_quoted_as_references_write_them() {
    assert_eq!(quote("A/7"), "'A/7'");
    assert_eq!(Day::new(2026, 3, 1).unwrap().weekday(), 6);
}
