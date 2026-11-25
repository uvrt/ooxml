//! The rows a session's `word_edit_table` calls wrote (round 58, R126) on a made-up "Shelf"
//! table: kept in the session's record beside the versions -- two writers at once both
//! land in it, an undone row stays named but is not found in the versions after -- and
//! text written into an empty cell given its column's formatting in the same step, said in the
//! reply.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::editing::{as_caller, body_of, call, docx, session};
use ooxml_session::{CallContext, DocKind};
use ooxml_session_word::prelude::*;
use serde_json::{json, Value};
use std::sync::Arc;

const BLUE: &str =
    r#"<w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:color w:val="1F4E79"/><w:sz w:val="16"/></w:rPr>"#;

fn cell(text: Option<&str>) -> String {
    let runs = text.map_or_else(String::new, |t| format!("<w:r>{BLUE}<w:t>{t}</w:t></w:r>"));
    format!(r#"<w:tc><w:tcPr><w:tcW w:w="2400" w:type="dxa"/></w:tcPr><w:p>{runs}</w:p></w:tc>"#)
}

fn row(id: &str, values: [Option<&str>; 2]) -> String {
    format!(
        r#"<w:tr w14:paraId="{id}" w14:textId="77777777">{}{}</w:tr>"#,
        cell(values[0]),
        cell(values[1])
    )
}

/// A header row, then a written row, two empty rows and another written row.
fn shelf() -> String {
    let rows = [
        row("0A000001", [Some("Item"), Some("Count")]),
        row("0A000002", [Some("Lamp"), Some("3")]),
        row("0A000003", [None, None]),
        row("0A000004", [None, None]),
        row("0A000005", [Some("Stool"), Some("2")]),
        row("0A000006", [Some("Chair"), Some("4")]),
    ];
    format!(
        r#"<w:p w14:paraId="00000001" w14:textId="77777777"><w:r><w:t>Shelf.</w:t></w:r></w:p><w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="2400"/><w:gridCol w:w="2400"/></w:tblGrid>{}</w:tbl><w:p w14:paraId="00000002" w14:textId="77777777"/>"#,
        rows.concat()
    )
}

fn set(row: usize, text: &str) -> Value {
    json!({"doc": "d1", "table": "t:0A000001", "action": "set_cells",
        "cells": [{"row": row, "col": 0, "text": text}]})
}

#[test]
fn two_writers_at_once_both_keep_their_rows_and_empty_cells_take_their_column() {
    let s = Arc::new(session());
    let d = s.open(docx(&shelf()), DocKind::Docx, "a.docx").unwrap();
    let writers: Vec<_> = [(2, "Rug"), (3, "Vase")]
        .into_iter()
        .enumerate()
        .map(|(k, (r, text))| {
            let s = Arc::clone(&s);
            std::thread::spawn(move || call(&s, "word_edit_table", set(r, text), &as_caller(&format!("w{k}"))))
        })
        .collect();
    for writer in writers {
        let reply = writer.join().unwrap();
        assert!(reply.get("error").is_none(), "{reply}");
        let warnings = reply["warnings"].as_array().cloned().unwrap_or_default();
        assert!(
            warnings
                .iter()
                .any(|w| w.as_str().unwrap().contains("written in its column's formatting")),
            "{reply}"
        );
    }
    let written = s.rows_written(&d).unwrap();
    assert!(
        written.contains("0A000003") && written.contains("0A000004"),
        "{written:?}"
    );
    let body = body_of(&s, &d);
    assert_eq!(body.matches(r#"<w:color w:val="1F4E79"/>"#).count(), 10, "{body}");
}

#[test]
fn a_row_undone_stays_named_and_is_not_found() {
    let s = session();
    let d = s.open(docx(&shelf()), DocKind::Docx, "a.docx").unwrap();
    let reply = call(
        &s,
        "word_edit_table",
        json!({"doc": "d1", "table": "t:0A000001", "action": "insert_row", "index": 5}),
        &as_caller("w1"),
    );
    assert!(reply.get("error").is_none(), "{reply}");
    let written = s.rows_written(&d).unwrap();
    let [made] = written.iter().collect::<Vec<_>>()[..] else {
        panic!("{written:?}");
    };
    assert!(body_of(&s, &d).contains(made.as_str()));
    assert_eq!(s.undo(&d, 1, &CallContext::default()).unwrap().count, 1);
    assert!(s.rows_written(&d).unwrap().contains(made));
    assert!(!body_of(&s, &d).contains(made.as_str()));
}

#[test]
fn text_and_its_column_formatting_are_one_step() {
    let s = session();
    let d = s.open(docx(&shelf()), DocKind::Docx, "a.docx").unwrap();
    let before = body_of(&s, &d);
    let reply = call(&s, "word_edit_table", set(2, "Rug"), &as_caller("w1"));
    assert!(reply.get("error").is_none(), "{reply}");
    assert_eq!(body_of(&s, &d).matches(r#"<w:color w:val="1F4E79"/>"#).count(), 9);
    // one undo takes back the text and the formatting it was given: the call was one commit
    assert_eq!(s.undo(&d, 1, &as_caller("w1")).unwrap().count, 1);
    assert_eq!(body_of(&s, &d), before);
}
