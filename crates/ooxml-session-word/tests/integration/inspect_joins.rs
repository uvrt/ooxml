//! Round 60 R146: a paragraph of the file's text deleted as a tracked change, its mark too,
//! is listed by `word_inspect` as `joins` on the block after it whatever that block is -- a
//! paragraph, a table (Accept All joins it into the first cell), a content control -- so a
//! host that maps the body's blocks to sections still places it. The final view stays as
//! Word 16.106's Accept All leaves it (measured: the paragraph goes, the next block keeps
//! its text). Made-up documents only.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::editing::{as_caller, call, docx, session};
use ooxml_session::{DocKind, Session};
use ooxml_session_word::prelude::*;
use ooxml_tools_word::new_document::Blank;
use serde_json::{json, Value};
use std::sync::Arc;

fn p(id: &str, text: &str) -> String {
    format!(r#"<w:p w14:paraId="{id}" w14:textId="77777777"><w:r><w:t>{text}</w:t></w:r></w:p>"#)
}

/// A heading, `Intro text.` (`p:00000002`), `next` and `After text.`.
fn body(next: &str) -> String {
    format!(
        r#"<w:p w14:paraId="00000001" w14:textId="77777777"><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Annex</w:t></w:r></w:p>{}{next}{}"#,
        p("00000002", "Intro text."),
        p("00000003", "After text.")
    )
}

fn table() -> String {
    let cell = |id: &str, text: &str| format!("<w:tc>{}</w:tc>", p(id, text));
    format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr>{}{}</w:tr><w:tr>{}{}</w:tr></w:tbl>"#,
        cell("00000011", "A1"),
        cell("00000012", "B1"),
        cell("00000013", "A2"),
        cell("00000014", "B2")
    )
}

fn control(id: u32, content: &str) -> String {
    format!(r#"<w:sdt><w:sdtPr><w:id w:val="{id}"/></w:sdtPr><w:sdtContent>{content}</w:sdtContent></w:sdt>"#)
}

/// `target` deleted with tracking on (by `Reviewer`).
fn delete_tracked(s: &Session, target: &str) {
    let w = as_caller("w1");
    let on = call(
        s,
        "word_set_tracking",
        json!({"doc": "d1", "on": true, "author": "Reviewer"}),
        &w,
    );
    assert_eq!(on["ok"], true, "{on}");
    let deleted = call(s, "word_delete", json!({"doc": "d1", "target": target}), &w);
    assert_eq!(deleted["ok"], true, "{deleted}");
}

/// `word_inspect` of the body: its top-level blocks.
fn blocks(s: &Session) -> Vec<Value> {
    let read = call(
        s,
        "word_inspect",
        json!({"doc": "d1", "target": "body"}),
        &as_caller("w1"),
    );
    assert_eq!(read["ok"], true, "{read}");
    read["data"]["blocks"].as_array().unwrap().clone()
}

/// Each block's id and its `joins`.
fn listed(blocks: &[Value]) -> Vec<(String, Vec<String>)> {
    blocks
        .iter()
        .map(|b| {
            let joins = b["joins"]
                .as_array()
                .map_or_else(Vec::new, |j| j.iter().map(|v| v.as_str().unwrap().to_owned()).collect());
            (b["id"].as_str().unwrap().to_owned(), joins)
        })
        .collect()
}

fn texts(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("paragraph") {
                out.push(map["t"].as_str().unwrap_or_default().to_owned());
            }
            map.values().for_each(|v| texts(v, out));
        }
        Value::Array(items) => items.iter().for_each(|v| texts(v, out)),
        _ => {}
    }
}

/// The final view's paragraph texts, in tables and controls too.
fn final_texts(blocks: &[Value]) -> Vec<String> {
    let mut out = Vec::new();
    texts(&Value::Array(blocks.to_vec()), &mut out);
    out
}

fn ids(pairs: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    pairs
        .iter()
        .map(|(id, joins)| ((*id).to_owned(), joins.iter().map(|j| (*j).to_owned()).collect()))
        .collect()
}

fn opened(body: &str) -> Session {
    let s = session();
    s.open(docx(body), DocKind::Docx, "a.docx").unwrap();
    s
}

fn block_of(s: &Session, id: &str) -> Option<String> {
    let d = s.documents().first().unwrap().clone();
    s.snapshot(&d).unwrap().block_of(id).unwrap().unwrap()
}

#[test]
fn the_teams_repro_a_deleted_paragraph_before_a_table_joins_the_table() {
    // new_document, "# Annex", "Intro text.", a 2x2 table, "After text.", saved and opened
    // again (the text is the file's), then deleted tracked
    let blank = Blank {
        page: "A4".into(),
        landscape: false,
        language: "en-US".into(),
        separators: (".".into(), ",".into()),
        title: None,
        author: None,
        created: "2026-11-25T09:00:00Z".into(),
    };
    let made = session();
    let d = made
        .open(Arc::from(blank.bytes().unwrap()), DocKind::Docx, "new.docx")
        .unwrap();
    let markdown = "# Annex\n\nIntro text.\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\nAfter text.";
    let written = call(
        &made,
        "word_insert_markdown",
        json!({"doc": "d1", "at": "end", "markdown": markdown}),
        &as_caller("w1"),
    );
    assert_eq!(written["ok"], true, "{written}");
    let s = session();
    s.open(Arc::from(made.save(&d).unwrap()), DocKind::Docx, "annex.docx")
        .unwrap();
    let before = blocks(&s);
    let intro = before.iter().find(|b| b["t"] == "Intro text.").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (heading, table, after) = (
        before[0]["id"].as_str().unwrap().to_owned(),
        before[2]["id"].as_str().unwrap().to_owned(),
        before[3]["id"].as_str().unwrap().to_owned(),
    );
    delete_tracked(&s, &intro);
    let now = blocks(&s);
    assert_eq!(
        listed(&now),
        ids(&[(&heading, &[]), (&table, &[&intro]), (&after, &[])]),
        "{now:?}"
    );
    // the final view as Word's Accept All shows it: the paragraph's text gone, the
    // first cell's its own (the cell's paragraph says what joined it)
    assert_eq!(final_texts(&now), ["Annex", "A", "B", "1", "2", "After text."]);
    assert_eq!(now[1]["rows"][0][0]["blocks"][0]["joins"], json!([intro]));
    // the paragraph is still found: by block_of, and by word_inspect as the table
    assert_eq!(block_of(&s, &intro).as_deref(), Some(intro.as_str()));
    let read = call(
        &s,
        "word_inspect",
        json!({"doc": "d1", "target": intro}),
        &as_caller("w1"),
    );
    assert_eq!(read["data"]["blocks"][0]["id"], json!(table), "{read}");
}

#[test]
fn before_a_table_its_mark_deleted_paragraph_is_the_tables_joins() {
    let s = opened(&body(&table()));
    delete_tracked(&s, "p:00000002");
    let now = blocks(&s);
    // the edit stamps the table's id (its rows had none)
    let table = now[1]["id"].as_str().unwrap().to_owned();
    assert!(table.starts_with("t:"), "{table}");
    assert_eq!(
        listed(&now),
        ids(&[("p:00000001", &[]), (&table, &["p:00000002"]), ("p:00000003", &[])])
    );
    assert_eq!(final_texts(&now), ["Annex", "A1", "B1", "A2", "B2", "After text."]);
    assert_eq!(block_of(&s, "p:00000002").as_deref(), Some("p:00000002"));
}

#[test]
fn before_a_content_control_its_mark_deleted_paragraph_is_the_controls_joins() {
    let s = opened(&body(&control(5, &p("00000021", "In control"))));
    delete_tracked(&s, "p:00000002");
    let now = blocks(&s);
    assert_eq!(
        listed(&now),
        ids(&[("p:00000001", &[]), ("cc:5", &["p:00000002"]), ("p:00000003", &[])])
    );
    // the control's paragraph says what joined it, as a cell's does
    assert_eq!(now[1]["blocks"][0]["joins"], json!(["p:00000002"]));
    assert_eq!(final_texts(&now), ["Annex", "In control", "After text."]);
    assert_eq!(block_of(&s, "p:00000002").as_deref(), Some("p:00000002"));
}

#[test]
fn an_empty_control_passes_the_paragraph_on_to_the_block_after_it() {
    let s = opened(&body(&format!("{}{}", control(6, ""), p("00000031", "Next para."))));
    delete_tracked(&s, "p:00000002");
    let now = blocks(&s);
    assert_eq!(
        listed(&now),
        ids(&[
            ("p:00000001", &[]),
            ("cc:6", &[]),
            ("p:00000031", &["p:00000002"]),
            ("p:00000003", &[])
        ])
    );
}

#[test]
fn before_a_paragraph_it_stays_that_paragraphs_joins() {
    let s = opened(&body(&p("00000031", "Next para.")));
    delete_tracked(&s, "p:00000002");
    let now = blocks(&s);
    assert_eq!(
        listed(&now),
        ids(&[
            ("p:00000001", &[]),
            ("p:00000031", &["p:00000002"]),
            ("p:00000003", &[])
        ])
    );
    assert_eq!(final_texts(&now), ["Annex", "Next para.", "After text."]);
    assert_eq!(block_of(&s, "p:00000002").as_deref(), Some("p:00000002"));
}
