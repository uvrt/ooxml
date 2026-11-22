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
    // without the host's layout, the host answers
    let reply = checked(&first, 1, json!({"doc": "d1", "include": ["layout"]}), &last, None);
    assert_eq!(reply["unported"], CHECK_NO_LAYOUT);
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

/// The fixture with a first paragraph holding a VML shape that states no size, which the
/// layout cannot lay out (docx-agent's `_vml_first`).
fn with_a_shape() -> ooxml_opc::Package {
    let (first, _) = versions();
    let xml = String::from_utf8(first.read("word/document.xml").unwrap().unwrap().to_vec()).unwrap();
    let body = xml.find("<w:body>").unwrap() + "<w:body>".len();
    let shape = r##"<w:p><w:r><w:pict><v:rect xmlns:v="urn:schemas-microsoft-com:vml" style="position:absolute" fillcolor="#4472c4"/></w:pict></w:r><w:r><w:t>A shape.</w:t></w:r></w:p>"##;
    let edited = format!("{}{shape}{}", &xml[..body], &xml[body..]);
    first.with_bytes("word/document.xml", std::sync::Arc::from(edited.into_bytes()))
}

#[test]
fn a_placeholder_makes_the_coverage_partial_and_its_page_uncertain() {
    let shaped = with_a_shape();
    let layout = EnginePages::of(engine());
    let last = LastCheck::default();
    let arguments = json!({"doc": "d1", "include": ["reflow", "layout"]});
    let reply = checked(&shaped, 1, arguments, &last, Some(&layout));
    let reflow = &reply["data"]["reflow"];
    let coverage = &reflow["coverage"];
    assert_eq!(coverage["complete"], false, "{reply}");
    assert_eq!(coverage["status"], "partial");
    assert!(coverage.get("stop").is_none());
    let blocks = coverage["blocks_laid_out"].as_array().unwrap();
    assert_eq!(
        blocks[0].as_u64().unwrap() + 1,
        blocks[1].as_u64().unwrap(),
        "{coverage}"
    );
    let placeholder = &coverage["placeholders"][0];
    assert_eq!(placeholder["page"], 1);
    assert_eq!(placeholder["reason"], "drawing");
    assert!(placeholder["at"].as_str().unwrap().starts_with('p'), "{placeholder}");
    assert!(
        placeholder["message"]
            .as_str()
            .unwrap()
            .starts_with("w:body/w:p[1] is drawn as a placeholder of its estimated height (a drawing this model"),
        "{placeholder}"
    );
    let warned = reflow["layout_warnings"].as_array().unwrap();
    assert!(
        warned.iter().any(|w| w
            .as_str()
            .unwrap()
            .starts_with("[layout-placeholder:drawing] (page 1) w:body/w:p[1]")),
        "{reflow}"
    );
    let uncertain = &reply["data"]["layout"]["uncertain"];
    assert_eq!(uncertain[0]["page"], 1, "{uncertain}");
    assert_eq!(uncertain[0]["placeholders"][0]["reason"], "drawing");
    assert_eq!(uncertain[0]["placeholders"][0]["at"], placeholder["at"]);
    // what follows the shape is laid out as without it: the pages go on
    let (plain, _) = versions();
    let alone = checked(
        &plain,
        1,
        json!({"doc": "d1", "include": ["reflow"]}),
        &LastCheck::default(),
        Some(&EnginePages::of(engine())),
    );
    assert!(
        reflow["pages"].as_u64() >= alone["data"]["reflow"]["pages"].as_u64(),
        "{reply}"
    );
}

#[test]
fn pages_outside_the_ranges_read_no_facts_and_say_so() {
    let (first, _) = versions();
    let layout = EnginePages::of(engine());
    let last = LastCheck::default();
    let on = checked(
        &first,
        1,
        json!({"doc": "d1", "include": ["layout"], "range": ["s:1"]}),
        &last,
        Some(&layout),
    );
    let pages = on["data"]["range"][0]["pages"].clone();
    let arguments = json!({"doc": "d1", "include": ["layout"], "range": ["s:1"], "pages": [999]});
    let reply = checked(&first, 1, arguments, &last, Some(&layout));
    assert!(reply.get("unported").is_none(), "{reply}");
    let listed = pages
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    assert_eq!(
        reply["warnings"],
        json!([format!(
            "pages [999] hold none of the range's blocks (on pages [{listed}]): no layout facts"
        )]),
        "{reply}"
    );
    let facts = &reply["data"]["layout"];
    assert_eq!(facts["direct_formatting"], json!([]), "{facts}");
    assert_eq!(reply["data"]["range"][0]["pages"], pages);
}
