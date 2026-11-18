//! The structure tools' definitions: the golden exports docx-agent's Python test holds to
//! today's (`test_rust_structure.py`), and their argument rules (a place named another
//! placing tool's way read as the tool's own, or refused with the call that works).
//! Their handlers' equality with docx-agent is `crates/ooxml-oracle/python/word_edits_diff.py
//! --structure`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_tools::ToolArgs;
use ooxml_tools_word::{
    WordCopyFrom, WordFields, WordHeadersFooters, WordLinks, WordMove, WordNotes, WordSections, KIND,
};
use serde_json::json;

#[test]
fn the_definitions_are_the_golden_exports() {
    for (spec, golden) in [
        (WordMove::spec(), include_str!("golden/word_move.json")),
        (WordCopyFrom::spec(), include_str!("golden/word_copy_from.json")),
        (WordSections::spec(), include_str!("golden/word_sections.json")),
        (
            WordHeadersFooters::spec(),
            include_str!("golden/word_headers_footers.json"),
        ),
        (WordFields::spec(), include_str!("golden/word_fields.json")),
        (WordNotes::spec(), include_str!("golden/word_notes.json")),
        (WordLinks::spec(), include_str!("golden/word_links.json")),
    ] {
        assert!(spec.problems().is_empty(), "{}: {:?}", spec.name, spec.problems());
        assert_eq!(format!("{}\n", spec.export(Some(&[KIND]))), golden, "{}", spec.name);
    }
}

#[test]
fn a_place_named_another_tools_way_is_read_as_its_own() {
    let (call, notes) = WordMove::accept(json!({"doc": "d1", "blocks": "p:A", "at": "before:p:B"}), None).unwrap();
    assert_eq!(call.after.as_deref(), Some("before:p:B"));
    assert_eq!(notes, ["at read as after"]);
    let (call, notes) = WordNotes::accept(json!({"doc": "d1", "action": "insert", "after": "p:A@3"}), None).unwrap();
    assert_eq!(call.at.as_deref(), Some("p:A@3"));
    assert_eq!(notes, ["after read as at"]);
}

#[test]
fn a_place_whose_side_is_in_its_name_is_refused_with_the_call() {
    let error = WordNotes::accept(json!({"doc": "d1", "action": "insert", "before": "p:A@3:9"}), None).unwrap_err();
    assert_eq!(error.field(), Some("before"));
    let call = error.way_out().and_then(|w| w.try_call()).unwrap();
    assert_eq!(call.arguments["at"], json!("p:A@3"));
    assert!(call.arguments.get("before").is_none());
}
