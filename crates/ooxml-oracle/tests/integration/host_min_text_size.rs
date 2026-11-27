//! The application's minimum text size on the Rust host (round 60, R140; docx-agent 0.1's
//! `docx_format(min_text_size=...)`, round 17 P6b): Word's format holds it, one number for
//! every kind of text or one per kind, and `check` reports the text drawn below it where the
//! call gives no `min_size`. A kind the mapping leaves out is not checked; a caption (a
//! paragraph in the Caption style) takes the caption's minimum wherever it is, else its
//! place's. On a made-up document with every story (markdown/constructs.docx).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_host::{CallAs, HostSession};
use ooxml_oracle::host_replay::Replayer;
use ooxml_session::DocKind;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// A host session whose Word format sets `min_text_size` (`None`: no setting), the made-up
/// document with every story open as `d1`.
fn opened(min_text_size: Option<Value>) -> HostSession {
    let replayer = Replayer::new(root()).unwrap();
    let toolbox = min_text_size.map_or_else(|| json!({}), |m| json!({"min_text_size": m}));
    let s = replayer.session(&json!({"toolbox": toolbox, "session": {}})).unwrap();
    let bytes = std::fs::read(root().join("crates/ooxml-oracle/fixtures/generated/markdown/constructs.docx")).unwrap();
    s.open(Arc::from(bytes), DocKind::Docx, "constructs.docx").unwrap();
    s
}

fn call(s: &HostSession, tool: &str, arguments: Value) -> Value {
    Value::Object(s.call(tool, arguments, &CallAs::default()).unwrap().to_json())
}

fn layout(s: &HostSession, arguments: Value) -> Value {
    let checked = call(s, "check", arguments);
    assert!(checked.get("error").is_none(), "{checked}");
    checked["data"]["layout"].clone()
}

fn kinds(found: &Value) -> BTreeSet<String> {
    found
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_minimum_per_kind_checks_the_kinds_it_names_each_against_its_own() {
    let s = opened(Some(json!({"body": 100, "footnote": 90, "header": null})));
    let found = layout(&s, json!({"doc": "d1", "include": ["layout"]}));
    let small = &found["small_text"];
    assert_eq!(
        kinds(small),
        BTreeSet::from(["body".to_owned(), "footnote".to_owned()]),
        "{found}"
    );
    for entry in small.as_array().unwrap() {
        let wanted = if entry["kind"] == "body" { 100.0 } else { 90.0 };
        assert_eq!(entry["min"].as_f64(), Some(wanted), "{entry}");
        assert!(entry["size"].as_f64().unwrap() < wanted, "{entry}");
    }
    // the kinds drawn but not named (table, header, footer, endnote) are not checked
    let drawn: BTreeSet<String> = found["smallest_text"].as_object().unwrap().keys().cloned().collect();
    assert!(
        drawn.is_superset(&BTreeSet::from(["body".into(), "table".into(), "header".into()])),
        "{found}"
    );
}

/// The kinds the made-up document draws at 10 pt (the rest at 11 pt).
fn at_ten() -> BTreeSet<String> {
    ["body", "endnote", "footnote"].map(str::to_owned).into()
}

#[test]
fn one_number_checks_every_kind_and_names_no_minimum_per_entry() {
    let s = opened(Some(json!(10.5)));
    let found = layout(&s, json!({"doc": "d1", "include": ["layout"]}));
    assert_eq!(kinds(&found["small_text"]), at_ten(), "{found}");
    assert!(found["small_text"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e.get("min").is_none()));
}

#[test]
fn the_calls_min_size_comes_before_the_applications_minimum() {
    let s = opened(Some(json!({"table": 100})));
    let found = layout(&s, json!({"doc": "d1", "include": ["layout"], "min_size": 10.5}));
    assert_eq!(kinds(&found["small_text"]), at_ten(), "{found}");
    assert!(found["small_text"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e.get("min").is_none()));
}

#[test]
fn a_caption_takes_the_captions_minimum_else_its_places() {
    let s = opened(Some(json!({"caption": 100})));
    let read = call(&s, "word_read", json!({"doc": "d1"}));
    let first = read["data"]
        .as_str()
        .unwrap()
        .lines()
        .find_map(|l| {
            l.strip_prefix("<!-- ")?
                .split_whitespace()
                .next()
                .filter(|id| id.starts_with("p:"))
        })
        .unwrap()
        .to_owned();
    let inserted = call(
        &s,
        "word_fields",
        json!({"doc": "d1", "action": "insert_caption", "after": first, "label": "Table", "text": "Made-up items"}),
    );
    assert!(inserted.get("error").is_none(), "{inserted}");
    let found = layout(&s, json!({"doc": "d1", "include": ["layout"]}));
    let small = found["small_text"].as_array().unwrap();
    assert_eq!(
        kinds(&found["small_text"]),
        BTreeSet::from(["caption".to_owned()]),
        "{found}"
    );
    assert_eq!(small.len(), 1, "{found}");
    let caption = small[0]["para"].clone();
    // without a caption's minimum the caption is text of its place: the body's
    let s = opened(Some(json!({"body": 100})));
    call(
        &s,
        "word_fields",
        json!({"doc": "d1", "action": "insert_caption", "after": first, "label": "Table", "text": "Made-up items"}),
    );
    let found = layout(&s, json!({"doc": "d1", "include": ["layout"]}));
    let entry = found["small_text"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["para"] == caption)
        .cloned();
    assert_eq!(entry.map(|e| e["kind"].clone()), Some(json!("body")), "{found}");
}

#[test]
fn the_applications_minimum_asks_for_the_layout_and_none_reports_nothing() {
    let s = opened(Some(json!({"body": 100})));
    let checked = call(&s, "check", json!({"doc": "d1", "include": ["validate"]}));
    assert!(checked["data"]["layout"]["small_text"].is_array(), "{checked}");
    let s = opened(None);
    let found = layout(&s, json!({"doc": "d1", "include": ["layout"]}));
    assert!(found.get("small_text").is_none(), "{found}");
}

#[test]
fn a_setting_that_is_not_a_minimum_is_not_made() {
    let replayer = Replayer::new(root()).unwrap();
    for bad in [
        json!({"chart": 9}),
        json!({}),
        json!(0),
        json!("9"),
        json!(true),
        json!({"slides": 10}),
    ] {
        let made = replayer.session(&json!({"toolbox": {"min_text_size": bad}, "session": {}}));
        assert!(made.is_err(), "{bad}");
    }
}

/// Round 60, R142: a check of more ranges than it takes says its limit in `details`.
#[test]
fn too_many_ranges_say_the_limit_in_details() {
    let s = opened(None);
    let ranges: Vec<String> = (0..21).map(|k| format!("p@body/{k}")).collect();
    let checked = call(&s, "check", json!({"doc": "d1", "range": ranges}));
    let error = &checked["error"];
    assert_eq!(error["code"], json!("limit"), "{checked}");
    assert_eq!(error["field"], json!("range"));
    assert_eq!(error["details"], json!({"limit": 20, "given": 21}));
    assert_eq!(
        error["message"],
        json!("21 ranges: at most 20 per check; a span p:A..t:B covers many blocks")
    );
}

// -- held to Word ---------------------------------------------------------------------------

/// The made-up document of Word's probe (`word/probes/min_text_size_probe.py`, made by
/// `word/cases/min_text_size_cases.py`): a paragraph of each kind, named by its first word.
const KINDS_DOCX: &str = "crates/ooxml-oracle/tests/data/word-probes/min_text_size_probe/kinds.docx";
/// What Word 16.106 made of it (`word/observations/min-text-size-word.json`).
const OBSERVED: &str = "crates/ooxml-oracle/word/observations/min-text-size-word.json";

/// Each paragraph's key (its text's first word) by its paraId, and the keys in a table's
/// cells, read from the document's stored parts.
fn keys_of(bytes: &Arc<[u8]>) -> (std::collections::BTreeMap<String, String>, BTreeSet<String>) {
    let archive = ooxml_zip::Archive::read(Arc::clone(bytes), &ooxml_zip::Limits::default()).unwrap();
    let mut keys = std::collections::BTreeMap::new();
    let mut in_cells = BTreeSet::new();
    for entry in archive.entries() {
        let xml = String::from_utf8_lossy(archive.stored_data(entry)).into_owned();
        for (at, _) in xml.match_indices("w14:paraId=\"") {
            let id = &xml[at + 12..at + 20];
            let rest = &xml[at..];
            let paragraph = &rest[..rest.find("</w:p>").unwrap_or(rest.len())];
            let Some(text) = paragraph.find("xml:space=\"preserve\">").map(|t| &paragraph[t + 21..]) else {
                continue;
            };
            let key: String = text.chars().take_while(char::is_ascii_alphabetic).collect();
            if key.is_empty() {
                continue;
            }
            let before = &xml[..at];
            if before.rfind("<w:tc>") > before.rfind("</w:tc>") {
                in_cells.insert(key.clone());
            }
            keys.insert(format!("p:{id}"), key);
        }
    }
    (keys, in_cells)
}

/// Each key's kind as Word's story and style make it (a paragraph in Word's own caption style
/// a caption, a body paragraph in a cell table text), and its size as Word gives it.
fn word_kinds(in_cells: &BTreeSet<String>) -> std::collections::BTreeMap<String, (String, f64)> {
    let observed: Value = serde_json::from_slice(&std::fs::read(root().join(OBSERVED)).unwrap()).unwrap();
    // Word's own insert caption gives its caption style; a Dutch Word names it locally as its id
    let caption = observed["word-caption"]["style_id"].as_str().unwrap();
    assert_eq!(observed["word-caption"]["style_name"], json!("caption"));
    let mut out = std::collections::BTreeMap::new();
    for (key, fact) in observed["word-kinds"].as_object().unwrap() {
        let place = match fact["story"].as_str().unwrap() {
            "main text story" if in_cells.contains(key) => "table",
            "main text story" => "body",
            "footnotes story" => "footnote",
            "endnotes story" => "endnote",
            "primary header story" => "header",
            "primary footer story" => "footer",
            other => panic!("{key}: story {other}"),
        };
        let kind = if fact["style"] == caption { "caption" } else { place };
        out.insert(key.clone(), (kind.to_owned(), fact["size"].as_f64().unwrap()));
    }
    out
}

/// The check's `small_text` of the Word probe's document with the application's minimum
/// `minimum`, by key: kind and size.
fn small_by_key(minimum: Value) -> std::collections::BTreeMap<String, (String, f64)> {
    let replayer = Replayer::new(root()).unwrap();
    let s = replayer
        .session(&json!({"toolbox": {"min_text_size": minimum}, "session": {}}))
        .unwrap();
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(root().join(KINDS_DOCX)).unwrap());
    let (keys, _) = keys_of(&bytes);
    s.open(Arc::clone(&bytes), DocKind::Docx, "kinds.docx").unwrap();
    let found = layout(&s, json!({"doc": "d1", "include": ["layout"]}));
    let mut out = std::collections::BTreeMap::new();
    for entry in found["small_text"].as_array().unwrap() {
        let para = entry["para"].as_str().unwrap();
        let id = para.rsplit('/').next().unwrap();
        let key = keys.get(id).unwrap_or_else(|| panic!("{para}: no key"));
        let kind = entry["kind"].as_str().unwrap().to_owned();
        assert!(
            out.insert(key.clone(), (kind, entry["size"].as_f64().unwrap()))
                .is_none(),
            "{found}"
        );
    }
    out
}

/// Every kind named at 12 pt: each paragraph is reported once, of the kind Word's story and
/// style make it, at the size Word gives it.
#[test]
fn each_paragraphs_kind_and_size_are_words() {
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(root().join(KINDS_DOCX)).unwrap());
    let (_, in_cells) = keys_of(&bytes);
    let word = word_kinds(&in_cells);
    assert_eq!(word.len(), 10, "{word:?}");
    let all: serde_json::Map<String, Value> = ["body", "table", "footnote", "endnote", "header", "footer", "caption"]
        .iter()
        .map(|k| ((*k).to_owned(), json!(12)))
        .collect();
    assert_eq!(small_by_key(Value::Object(all)), word);
}

/// The team's minimums (round 60: body 10, table 8.5, caption 7, footnote 8): each kind is held
/// to its own -- body text at 9 pt, a cell's at 8 and a footnote's at 7.5 are reported beside
/// a caption at 6.5, though all are above the smallest minimum (docx-agent 0.1 reported only
/// the text below that, 7 pt). Header, footer and endnote text is not checked.
#[test]
fn each_kind_is_held_to_its_own_minimum_not_the_smallest() {
    let team = json!({"body": 10, "table": 8.5, "caption": 7, "footnote": 8});
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(root().join(KINDS_DOCX)).unwrap());
    let (_, in_cells) = keys_of(&bytes);
    let expected: std::collections::BTreeMap<String, (String, f64)> = word_kinds(&in_cells)
        .into_iter()
        .filter(|(_, (kind, size))| team[kind.as_str()].as_f64().is_some_and(|m| *size < m))
        .collect();
    let keys: Vec<&str> = expected.keys().map(String::as_str).collect();
    assert_eq!(keys, ["Alpha", "Delta", "Echo", "India"]);
    assert_eq!(small_by_key(team), expected);
}
