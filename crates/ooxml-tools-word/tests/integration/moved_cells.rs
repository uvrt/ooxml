//! Positions in a table whose pending cell deletions do not line up across rows (lane
//! fix-moved-left): Word's Accept All removes each deleted cell that keeps nothing and moves
//! the cells after it left before it rebuilds the grid (Word 16.106, `Stagger-gone`), so a
//! column as accepted is a different grid column in different rows. A call naming a cell by
//! row and column writes that row's own cell there; one naming a whole column is refused as
//! not supported, the rows it differs in named.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, Still, Unlimited, R, W};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{word_edit_table, word_format_table, CallEditor, Editing, WordEditTable, WordFormatTable};
use ooxml_word_edit::Editor;
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::{json, Value};
use std::sync::Arc;

const DATE: &str = "2026-11-11T09:00:00Z";

fn cell(id: u32, width: u32, text: &str) -> String {
    format!(
        r#"<w:tc><w:tcPr><w:tcW w:w="{width}" w:type="dxa"/></w:tcPr><w:p w14:paraId="{id:08X}" w14:textId="7{id:07X}"><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>"#
    )
}

/// A deleted cell whose text and paragraph mark are deleted too: Accept All leaves nothing
/// of it.
fn gone(id: u32, width: u32) -> String {
    format!(
        r#"<w:tc><w:tcPr><w:tcW w:w="{width}" w:type="dxa"/><w:cellDel w:id="{}" w:author="Other" w:date="{DATE}"/></w:tcPr><w:p w14:paraId="{id:08X}" w14:textId="7{id:07X}"><w:pPr><w:rPr><w:del w:id="{}" w:author="Other" w:date="{DATE}"/></w:rPr></w:pPr><w:del w:id="{}" w:author="Other" w:date="{DATE}"><w:r><w:delText>x</w:delText></w:r></w:del></w:p></w:tc>"#,
        id + 1,
        id + 2,
        id + 3
    )
}

/// `Stagger-gone`: [del][B1][C1], [A2][del][C2], [A3][B3][C3] on a grid of 1000/2000/3000.
fn body() -> String {
    let row = |id: u32, cells: [String; 3]| {
        format!(
            r#"<w:tr w14:paraId="{id:08X}" w14:textId="7{id:07X}">{}</w:tr>"#,
            cells.concat()
        )
    };
    format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="1000"/><w:gridCol w:w="2000"/><w:gridCol w:w="3000"/></w:tblGrid>{}{}{}</w:tbl><w:p w14:paraId="00000D01" w14:textId="70000D01"/>"#,
        row(
            0xA00,
            [gone(0xA10, 1000), cell(0xA20, 2000, "B1"), cell(0xA30, 3000, "C1")]
        ),
        row(
            0xB00,
            [cell(0xB10, 1000, "A2"), gone(0xB20, 2000), cell(0xB30, 3000, "C2")]
        ),
        row(
            0xC00,
            [
                cell(0xC10, 1000, "A3"),
                cell(0xC20, 2000, "B3"),
                cell(0xC30, 3000, "C3")
            ]
        ),
    )
}

fn package() -> Package {
    let parts = [
        ("[Content_Types].xml".to_owned(), r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_owned()),
        ("_rels/.rels".to_owned(), format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}officeDocument" Target="word/document.xml"/></Relationships>"#)),
        ("word/document.xml".to_owned(), format!("<w:document {W}><w:body>{}</w:body></w:document>", body())),
    ];
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

/// `tool` with `arguments` on the table, untracked: the reply and the body after.
fn call(tool: &str, arguments: Value) -> (Value, String) {
    let editor = Editor::open(package(), StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let call_editor = CallEditor::new(&editor);
    let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, DATE);
    let mut arguments = arguments;
    arguments["doc"] = json!("d1");
    arguments["table"] = json!("t:00000A00");
    let reply = Value::Object(
        if tool == "word_format_table" {
            word_format_table(&WordFormatTable::accept(arguments, Some("docx")).unwrap().0, &editing)
        } else {
            word_edit_table(&WordEditTable::accept(arguments, Some("docx")).unwrap().0, &editing)
        }
        .to_json(),
    );
    let snapshot = editor.store().snapshot();
    let tree = snapshot.doc().tree("word/document.xml").unwrap().unwrap();
    (reply, String::from_utf8(tree.write()).unwrap())
}

fn refused_as_moved(reply: &Value) {
    assert_eq!(reply["ok"], json!(false), "{reply}");
    let error = &reply["error"];
    assert_eq!(
        error["details"]["not_supported"],
        json!(ooxml_word_model::docx::tables::MOVED_COLUMN),
        "{reply}"
    );
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("not one grid column in every row"), "{message}");
    assert!(
        message.contains("Accept or reject the pending cell changes first, or edit the cells by row"),
        "{message}"
    );
}

#[test]
fn a_whole_column_whose_cells_move_is_refused_naming_the_rows() {
    let (reply, _) = call("word_edit_table", json!({"action": "delete_column", "index": 1}));
    refused_as_moved(&reply);
    assert!(
        reply["error"]["message"].as_str().unwrap().contains("in row(s) 0, 1"),
        "{reply}"
    );
    let (reply, _) = call("word_edit_table", json!({"action": "insert_column", "index": 1}));
    refused_as_moved(&reply);
    let (reply, _) = call(
        "word_format_table",
        json!({"scope": "column", "column": 1, "width": 72}),
    );
    refused_as_moved(&reply);
    let (reply, _) = call(
        "word_edit_table",
        json!({"action": "set_cells", "cells": [{"col": 1, "rows": "0..2", "text": "x"}]}),
    );
    refused_as_moved(&reply);
    let (reply, _) = call(
        "word_edit_table",
        json!({"action": "merge", "first": {"row": 0, "col": 0}, "last": {"row": 1, "col": 1}}),
    );
    refused_as_moved(&reply);
}

#[test]
fn a_cell_by_row_and_column_is_that_rows_own_cell_there() {
    // as accepted the grid is 1000 x 6: row 0 is [B1 over cols 0-1][C1 over 2-4], row 1
    // [A2 over 0][C2 over 1-3]
    let (reply, body) = call(
        "word_edit_table",
        json!({"action": "set_cells", "cells": [{"row": 0, "col": 2, "text": "new C1"}, {"row": 1, "col": 1, "text": "new C2"}]}),
    );
    assert_eq!(reply["ok"], json!(true), "{reply}");
    assert!(body.contains("new C1") && body.contains("new C2"), "{body}");
    assert!(!body.contains(">C1<") && !body.contains(">C2<"), "{body}");
    let notes = reply.to_string();
    assert!(notes.contains("col 1 is grid column 2 in that row"), "{notes}");
}

#[test]
fn rows_stay_writable() {
    let (reply, _) = call("word_edit_table", json!({"action": "delete_row", "index": 2}));
    assert_eq!(reply["ok"], json!(true), "{reply}");
}
