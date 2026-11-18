//! The object tools' definitions: the golden exports docx-agent's Python test holds to
//! today's (`test_rust_objects.py`), and their argument rules (a list of names given as
//! `columns` read as `from_columns`; a place named another placing tool's way read as the
//! tool's own). Their handlers' equality with docx-agent is
//! `crates/ooxml-oracle/python/word_edits_diff.py --objects`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_tools::ToolArgs;
use ooxml_tools_word::{WordControls, WordDrawings, WordEditTable, WordFormatTable, KIND};
use serde_json::json;

#[test]
fn the_definitions_are_the_golden_exports() {
    for (spec, golden) in [
        (WordEditTable::spec(), include_str!("golden/word_edit_table.json")),
        (WordFormatTable::spec(), include_str!("golden/word_format_table.json")),
        (WordDrawings::spec(), include_str!("golden/word_drawings.json")),
        (WordControls::spec(), include_str!("golden/word_controls.json")),
    ] {
        assert!(spec.problems().is_empty(), "{}: {:?}", spec.name, spec.problems());
        assert_eq!(format!("{}\n", spec.export(Some(&[KIND]))), golden, "{}", spec.name);
    }
}

#[test]
fn a_list_of_names_as_columns_is_read_as_from_columns() {
    let given = json!({"doc": "d1", "table": "t:A", "action": "sync", "from": "d:x", "columns": ["Region", "Total"]});
    let (call, notes) = WordEditTable::accept(given, None).unwrap();
    assert_eq!(call.from_columns, Some(vec!["Region".to_owned(), "Total".to_owned()]));
    assert_eq!(call.columns, None);
    assert_eq!(notes, ["columns read as from_columns"]);
    // numbers, or from_columns given too: columns stays split's count (and is checked as one)
    let given = json!({"doc": "d1", "table": "t:A", "action": "split", "columns": 3});
    let (call, notes) = WordEditTable::accept(given, None).unwrap();
    assert_eq!((call.columns, notes.len()), (Some(3), 0));
    let given = json!({"doc": "d1", "table": "t:A", "action": "sync", "columns": ["2"]});
    assert!(WordEditTable::accept(given, None).is_err());
}

#[test]
fn a_place_named_another_tools_way_is_read_as_its_own() {
    let (call, notes) = WordControls::accept(json!({"doc": "d1", "action": "insert", "after": "p:A@3"}), None).unwrap();
    assert_eq!(call.at.as_deref(), Some("p:A@3"));
    assert_eq!(notes, ["after read as at"]);
    let (call, notes) =
        WordDrawings::accept(json!({"doc": "d1", "action": "insert_picture", "size_pt": 40}), None).unwrap();
    assert_eq!(call.width, Some(40.0));
    assert_eq!(notes, ["size_pt read as width"]);
}
