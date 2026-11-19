//! `word_edit_table`'s handlers on a made-up document, for the tool design rules they
//! touch: a column inserted, a row copied (what it leaves on the row copied said), cells
//! merged and split, a merge already made changing nothing (rule 3), and a refusal
//! carrying its way out (rule 2: a column past the last names the columns; a row not
//! copied gives the calls that add a row like it and fill it; a split past a cell's span
//! gives the splits that keep the grid); cells set by labels an earlier cell wrote, a
//! column's rows filled and said by range and counts (rule 4), a label or place not there
//! refused with the rows or the table's shape, one merged cell given two values refused
//! with the split (rule 2); a row holding a drawing and a content control copied with new
//! numbers. Their equality with
//! docx-agent is `crates/ooxml-oracle/python/word_edits_diff.py --objects`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::{Cancel, Limits};
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, StoreConfig};
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{word_edit_table, CallEditor, Editing, WordEditTable};
use ooxml_word_edit::Editor;
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

struct Never;
impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

struct Still;
impl Clock for Still {
    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

struct Unlimited;
impl CacheBudget for Unlimited {
    fn reserve(&self, _bytes: usize) -> bool {
        true
    }
    fn release(&self, _bytes: usize) {}
    fn used(&self) -> usize {
        0
    }
}

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#;
const DRAWN: &str = r#" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:wp14="http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing""#;
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";

fn cell(text: &str, id: u32) -> String {
    format!(r#"<w:tc><w:p w14:paraId="{id:08X}" w14:textId="7{id:07X}"><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>"#)
}

/// A made-up document: a table of three rows and two columns; the second row's first
/// cell holds a bookmark, the third's a footnote reference.
fn package() -> Package {
    let marked = r#"<w:tc><w:p w14:paraId="00000B01" w14:textId="70000B01"><w:bookmarkStart w:id="0" w:name="Kept"/><w:r><w:t>B</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p></w:tc>"#;
    let noted = r#"<w:tc><w:p w14:paraId="00000C01" w14:textId="70000C01"><w:r><w:t>C</w:t></w:r><w:r><w:footnoteReference w:id="1"/></w:r></w:p></w:tc>"#;
    let body = format!(
        concat!(
            r#"<w:tbl><w:tblPr><w:tblW w:w="4000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>"#,
            r#"<w:tr w14:paraId="00000A00" w14:textId="70000A00">{}{}</w:tr>"#,
            r#"<w:tr w14:paraId="00000B00" w14:textId="70000B00">{}{}</w:tr>"#,
            r#"<w:tr w14:paraId="00000C00" w14:textId="70000C00">{}{}</w:tr>"#,
            r#"</w:tbl><w:p w14:paraId="00000D01" w14:textId="70000D01"/>"#
        ),
        cell("A", 0xA01),
        cell("a", 0xA02),
        marked,
        cell("b", 0xB02),
        noted,
        cell("c", 0xC02)
    );
    let parts = [
        ("[Content_Types].xml".to_owned(), r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_owned()),
        ("_rels/.rels".to_owned(), format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}officeDocument" Target="word/document.xml"/></Relationships>"#)),
        ("word/document.xml".to_owned(), format!("<w:document {W}><w:body>{body}</w:body></w:document>")),
    ];
    package_of(parts)
}

/// A package of `parts` as named.
fn package_of(parts: [(String, String); 3]) -> Package {
    let mut w = Writer::new();
    for (name, data) in parts {
        let entry = NewEntry {
            name,
            method: Method::Deflated,
            dos_time: 0,
            dos_date: 33,
            external_attr: 0,
            create_system: 3,
        };
        w.add(&entry, data.as_bytes()).unwrap();
    }
    Package::open(Arc::from(w.finish(b"").unwrap()), &Limits::default()).unwrap()
}

/// `word_edit_table` called with `arguments` (and `doc`, `table`) on a fresh document.
fn edit(arguments: Value) -> Value {
    edits(&[arguments])
}

/// `word_edit_table` called with each of `calls` in turn on a fresh document; the last
/// reply.
fn edits(calls: &[Value]) -> Value {
    edits_on(package(), calls).0
}

/// `word_edit_table` called with each of `calls` in turn on `package`; the last reply and
/// the body part as it is after them.
fn edits_on(package: Package, calls: &[Value]) -> (Value, String) {
    let editor = Editor::open(package, StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let editing = Editing {
        editor: &CallEditor::new(&editor),
        caller: &caller,
        opened: &opened,
        tracking: None,
        doc: "d1",
        cancel: &Never,
        date: "2026-11-17T10:00:00Z",
        layout: None,
    };
    let mut reply = Value::Null;
    for arguments in calls {
        let mut arguments = arguments.clone();
        arguments["doc"] = json!("d1");
        arguments["table"] = json!("t:00000A00");
        let (args, _) = WordEditTable::accept(arguments, Some("docx")).unwrap();
        reply = Value::Object(word_edit_table(&args, &editing).to_json());
    }
    let snapshot = editor.store().snapshot();
    let tree = snapshot.doc().tree("word/document.xml").unwrap().unwrap();
    (reply, String::from_utf8(tree.write()).unwrap())
}

#[test]
fn a_column_inserted_is_the_tables_widest_and_its_paragraphs_new() {
    let reply = edit(json!({"action": "insert_column", "index": 0}));
    assert_eq!(reply["data"]["columns"], 3, "{reply}");
    assert_eq!(reply["created"].as_array().map(Vec::len), Some(3), "{reply}");
}

#[test]
fn a_column_past_the_last_is_refused_with_the_columns() {
    let reply = edit(json!({"action": "insert_column", "index": 5}));
    assert_eq!(reply["error"]["code"], "not_found", "{reply}");
    assert_eq!(reply["error"]["valid_options"], json!(["0", "1"]), "{reply}");
}

#[test]
fn a_row_copied_leaves_its_bookmark_on_the_row_copied() {
    let reply = edit(json!({"action": "copy_row", "index": 1}));
    assert_eq!(reply["data"]["rows"], 4, "{reply}");
    let warnings = reply["warnings"].to_string();
    assert!(warnings.contains("left on row 1, not copied: 1 bookmark(s)"), "{reply}");
}

#[test]
fn a_row_not_copied_gives_the_calls_that_add_one_like_it() {
    let reply = edit(json!({"action": "copy_row", "index": 2}));
    assert_eq!(reply["error"]["code"], "refused", "{reply}");
    let calls = &reply["error"]["details"]["calls"];
    assert_eq!(calls[0]["arguments"]["action"], "insert_row", "{reply}");
    assert_eq!(calls[1]["arguments"]["cells"][0]["row"], 3, "{reply}");
}

#[test]
fn cells_merged_across_are_one_cell() {
    let reply = edit(json!({"action": "merge", "first": {"row": 0, "col": 0}, "last": {"row": 0, "col": 1}}));
    assert_eq!(reply["changed"], json!(["t:00000A00"]), "{reply}");
    assert!(reply["data"]["markdown"].to_string().contains("colspan"), "{reply}");
}

#[test]
fn a_merge_already_made_changes_nothing() {
    let reply = edit(json!({"action": "merge", "first": {"row": 1, "col": 1}, "last": {"row": 1, "col": 1}}));
    assert!(reply["warnings"][0].to_string().contains("nothing changed"), "{reply}");
    assert!(reply.get("changed").is_none_or(|c| c == &json!([])), "{reply}");
}

#[test]
fn a_cell_split_down_adds_a_row_and_across_a_column() {
    // two columns by default, as Word's dialog offers
    let reply = edit(json!({"action": "split", "first": {"row": 0, "col": 0}, "rows": 2}));
    assert_eq!(
        (reply["data"]["rows"].clone(), reply["data"]["columns"].clone()),
        (json!(4), json!(3)),
        "{reply}"
    );
    assert_eq!(reply["created"].as_array().map(Vec::len), Some(4), "{reply}");
}

#[test]
fn a_split_past_a_cells_span_gives_the_splits_that_keep_the_grid() {
    let merge = json!({"action": "merge", "first": {"row": 0, "col": 0}, "last": {"row": 0, "col": 1}});
    let reply = edits(&[
        merge,
        json!({"action": "split", "first": {"row": 0, "col": 0}, "columns": 3}),
    ]);
    assert_eq!(reply["error"]["code"], "refused", "{reply}");
    assert_eq!(reply["error"]["valid_options"], json!([2]), "{reply}");
    assert_eq!(reply["error"]["details"]["call"]["arguments"]["columns"], 2, "{reply}");
}

#[test]
fn cells_set_by_labels_read_what_earlier_cells_wrote() {
    let reply = edit(json!({"action": "set_cells", "cells": [
        {"row": 1, "col": 0, "text": "Renamed"},
        {"row_label": "Renamed", "col_label": "a", "text": "Found"},
    ]}));
    let markdown = reply["data"]["markdown"].to_string();
    assert!(markdown.contains("Renamed") && markdown.contains("Found"), "{reply}");
}

#[test]
fn a_label_matching_no_row_is_refused_with_the_rows() {
    let reply = edit(json!({"action": "set_cells", "cells": [{"row_label": "Nowhere", "col": 0, "text": "x"}]}));
    assert_eq!(reply["error"]["code"], "not_found", "{reply}");
    // the third row's first cell holds a footnote reference, shown as an object mark
    assert_eq!(
        reply["error"]["valid_options"],
        json!(["A", "B", "C\u{fffc}"]),
        "{reply}"
    );
}

#[test]
fn a_cell_past_the_table_is_refused_with_its_shape() {
    let reply = edit(json!({"action": "set_cells", "cells": [{"row": 9, "col": 0, "text": "x"}]}));
    assert_eq!(reply["error"]["code"], "not_found", "{reply}");
    assert!(
        reply["error"]["message"].to_string().contains("3 row(s) (0 to 2)"),
        "{reply}"
    );
}

#[test]
fn a_column_filled_is_said_by_its_range_and_counts() {
    // the default look has a header row: '*' leaves row 0 out
    let reply = edit(json!({"action": "set_cells", "cells": [{"col": 1, "rows": "*", "text": "b"}]}));
    assert_eq!(
        reply["data"]["filled"],
        json!([{"entry": 0, "col": 1, "rows": "1..2", "cells": 2, "changed": 1, "header_rows": 1}]),
        "{reply}"
    );
    assert_eq!(reply["changed"], json!(["t:00000A00"]), "{reply}");
    assert!(reply["data"].get("markdown").is_none(), "{reply}");
}

#[test]
fn a_fill_past_the_last_row_gives_the_rows_there_are() {
    let reply = edit(json!({"action": "set_cells", "cells": [{"col": 1, "rows": "1..7", "text": "x"}]}));
    assert_eq!(reply["error"]["code"], "not_found", "{reply}");
    assert_eq!(reply["error"]["valid_options"], json!(["*", "1..2"]), "{reply}");
}

#[test]
fn two_values_for_one_merged_cell_are_refused_with_the_split() {
    let merge = json!({"action": "merge", "first": {"row": 0, "col": 0}, "last": {"row": 0, "col": 1}});
    let reply = edits(&[
        merge,
        json!({"action": "set_cells", "cells": [{"row": 0, "col": 0, "text": "x"}, {"row": 0, "col": 1, "text": "y"}]}),
    ]);
    assert_eq!(reply["error"]["code"], "refused", "{reply}");
    let split = &reply["error"]["details"]["merged"][0]["split"]["arguments"];
    assert_eq!(
        (split["rows"].clone(), split["columns"].clone()),
        (json!(1), json!(2)),
        "{reply}"
    );
}

#[test]
fn a_value_by_reference_is_left_to_the_host() {
    let reply = edit(json!({"action": "set_cells", "cells": [{"row": 0, "col": 0, "from": "d:x/#1/a"}]}));
    // `unported` is the host's to read: the reply changes nothing and says nothing
    assert_eq!(
        (reply["ok"].clone(), reply["summary"].clone()),
        (json!(true), json!("")),
        "{reply}"
    );
    assert_eq!(reply["changed"], json!([]), "{reply}");
}

/// A made-up document whose first row holds a drawing (number 5) and a content control
/// (id -7), and whose paragraph after the table holds a drawing numbered 9.
fn drawn_package() -> Package {
    let inline = |number: u32, anchor: &str| {
        format!(
            r#"<w:r><w:drawing><wp:inline wp14:anchorId="{anchor}" wp14:editId="{anchor}"><wp:extent cx="10" cy="10"/><wp:docPr id="{number}" name="Picture {number}"/></wp:inline></w:drawing></w:r>"#
        )
    };
    let body = format!(
        concat!(
            r#"<w:tbl><w:tblPr><w:tblW w:w="4000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>"#,
            r#"<w:tr w14:paraId="00000A00" w14:textId="70000A00"><w:tc><w:p w14:paraId="00000A01" w14:textId="70000A01">{}</w:p></w:tc>"#,
            r#"<w:tc><w:sdt><w:sdtPr><w:id w:val="-7"/></w:sdtPr><w:sdtContent><w:p w14:paraId="00000A02" w14:textId="70000A02"><w:r><w:t>a</w:t></w:r></w:p></w:sdtContent></w:sdt></w:tc></w:tr>"#,
            r#"</w:tbl><w:p w14:paraId="00000D01" w14:textId="70000D01">{}</w:p>"#
        ),
        inline(5, "1A2B3C4D"),
        inline(9, "2A2B3C4E"),
    );
    package_of([
        ("[Content_Types].xml".to_owned(), r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_owned()),
        ("_rels/.rels".to_owned(), format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}officeDocument" Target="word/document.xml"/></Relationships>"#)),
        ("word/document.xml".to_owned(), format!("<w:document {W}{DRAWN}><w:body>{body}</w:body></w:document>")),
    ])
}

#[test]
fn a_row_with_a_drawing_and_a_control_copied_takes_new_numbers() {
    let (reply, body) = edits_on(drawn_package(), &[json!({"action": "copy_row", "index": 0})]);
    assert_eq!(reply["ok"], json!(true), "{reply}");
    // past every drawing number the document has, the control's id fresh, the drawing's ids too
    assert!(body.contains(r#"<wp:docPr id="10""#), "{body}");
    assert_eq!(body.matches(r#"<wp:docPr id="5""#).count(), 1, "{body}");
    assert_eq!(body.matches(r#"<w:id w:val="-7"/>"#).count(), 1, "{body}");
    assert_eq!(body.matches("<w:id w:val=").count(), 2, "{body}");
    assert_eq!(body.matches(r#"wp14:anchorId="1A2B3C4D""#).count(), 1, "{body}");
    assert_eq!(body.matches(r#"wp14:editId="1A2B3C4D""#).count(), 1, "{body}");
}
