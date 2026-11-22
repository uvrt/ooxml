//! `check`'s `layout` and `range` (lane P5k): the layout's facts read from the pages and
//! from the document alone, in docx-agent's order, kept to a range's blocks and pages with
//! each range's report; past the call's limit on laying out, the facts read from the
//! document alone. Their equality with docx-agent's is
//! `crates/ooxml-oracle/python/word_reads_diff.py check`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::check::{checked, limited, versions, Past};
use crate::common::engine;
use ooxml_tools_word::{EnginePages, LastCheck, LayoutLimit, CHECK_NO_LAYOUT};
use serde_json::json;
use std::time::Duration;

#[test]
fn a_check_of_the_layout_reports_its_facts_in_docx_agents_order() {
    let (first, _) = versions();
    let layout = EnginePages::of(engine());
    let last = LastCheck::default();
    let reply = checked(
        &first,
        1,
        json!({"doc": "d1", "include": ["layout"]}),
        &last,
        Some(&layout),
    );
    assert_eq!(reply["summary"], "Checked layout", "{reply}");
    let facts = reply["data"]["layout"].as_object().unwrap();
    let keys: Vec<&str> = facts
        .keys()
        .map(String::as_str)
        .filter(|k| !k.ends_with("_left_out"))
        .collect();
    let order = [
        "smallest_text",
        "direct_formatting",
        "spacers",
        "empty_paragraphs",
        "keep_with_next_at_foot",
        "table_parted",
        "numbering_skips_level",
        "table_grid_mismatch",
        "accept_loses_text",
        "punctuation_spacing",
        "text_language",
        "settings",
        "split_rows",
        "header_not_repeated",
        "column_fit",
        "past_margins",
        "uncertain",
        "accept_loses_original",
        "row_text_format",
    ];
    let expected: Vec<&str> = order.iter().copied().filter(|k| facts.contains_key(*k)).collect();
    assert_eq!(keys, expected);
    assert!(facts["settings"].get("keep_together").is_some(), "{reply}");
    // no reflow asked: none said, and the next check's since is not measured from this one
    assert!(reply["data"].get("reflow").is_none());
    assert!(last.get().is_none());
    // but its pages are kept as the version's, for a changing call to measure from
    let laid = last.laid().expect("the version laid out is kept");
    assert_eq!(laid.version, Some(1));
}

#[test]
fn min_size_and_styles_ask_for_the_layout() {
    let (first, _) = versions();
    let layout = EnginePages::of(engine());
    let last = LastCheck::default();
    let arguments = json!({"doc": "d1", "include": ["validate"], "min_size": 40, "styles": ["No Such Style"]});
    let reply = checked(&first, 1, arguments, &last, Some(&layout));
    assert_eq!(reply["summary"], "Checked validate, layout", "{reply}");
    let facts = &reply["data"]["layout"];
    assert!(!facts["small_text"].as_array().unwrap().is_empty(), "{facts}");
    assert_eq!(facts["styles_unknown"], json!(["No Such Style"]));
    assert!(!facts["styles_not_allowed"].as_array().unwrap().is_empty(), "{facts}");
    // without the host's layout: not supported yet
    let reply = checked(&first, 1, json!({"doc": "d1", "include": ["layout"]}), &last, None);
    assert_eq!(reply["not_supported"], CHECK_NO_LAYOUT);
}

#[test]
fn a_check_of_a_range_reports_the_range_and_its_facts() {
    let (first, _) = versions();
    let layout = EnginePages::of(engine());
    let last = LastCheck::default();
    let reply = checked(
        &first,
        1,
        json!({"doc": "d1", "range": ["s:1", " s:1 "]}),
        &last,
        Some(&layout),
    );
    assert_eq!(
        reply["summary"], "Checked validate, reflow, fields, layout in 1 range(s)",
        "{reply}"
    );
    let data = reply["data"].as_object().unwrap();
    let keys: Vec<&str> = data.keys().map(String::as_str).collect();
    assert_eq!(keys, ["validate", "range", "reflow", "layout", "fields", "timing"]);
    let range = &data["range"][0];
    assert_eq!(range["range"], "s:1");
    assert!(range["blocks"].as_u64().unwrap() >= 1, "{range}");
    assert!(!range["pages"].as_array().unwrap().is_empty(), "{range}");
    assert!(data["validate"].get("elsewhere").is_some(), "{reply}");
    let parts: Vec<&String> = data["timing"].as_object().unwrap().keys().collect();
    assert_eq!(parts, ["resolve", "validate", "layout", "facts", "fields"]);
}

#[test]
fn a_layout_past_the_limit_gives_the_facts_read_from_the_document_alone() {
    let (_, edited) = versions();
    let layout = EnginePages::of(engine());
    let last = LastCheck::default();
    let limit = LayoutLimit {
        cancel: &Past,
        timeout: Duration::from_millis(1),
    };
    let arguments = json!({"doc": "d1", "include": ["layout"], "range": ["s:1"]});
    let reply = limited(&edited, 2, arguments, &last, Some(&layout), Some(limit));
    let data = &reply["data"];
    let error = "timeout: laying the document out did not finish within 0.001 s";
    assert_eq!(data["layout"]["error"], error, "{reply}");
    assert!(data["layout"]["empty_paragraphs"].is_array(), "{reply}");
    assert!(data["layout"].get("smallest_text").is_none(), "{reply}");
    assert_eq!(data["range"][0]["pages"], json!(null));
}

#[test]
fn a_notes_paragraph_is_on_the_pages_that_draw_it() {
    let (first, _) = versions();
    let layout = EnginePages::of(engine());
    let last = LastCheck::default();
    let arguments = json!({"doc": "d1", "include": ["layout"], "styles": ["Normal"]});
    let reply = checked(&first, 1, arguments, &last, Some(&layout));
    let entries = reply["data"]["layout"]["styles_not_allowed"].as_array().unwrap();
    let noted = entries
        .iter()
        .find(|e| e["blocks"][0].as_str().is_some_and(|b| b.starts_with("footnotes/")))
        .expect("a footnote's style");
    assert_eq!(noted["pages"], json!([1]), "{noted}");
}
