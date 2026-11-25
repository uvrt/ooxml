//! Pending tracked row deletions read in the final view (round 60, R137): a row deleted with
//! tracking on, holding text the file had when opened, leaves its cells' marks deleted; the
//! final view is Accept All's, which removes the row whole (Word 16.106,
//! `row-deletions-word.json`), so word_read's final view and word_inspect of any target
//! answer, after a save and reopen too, and the table takes further edits.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::editing::{as_caller, call, docx, session};
use ooxml_session::{DocId, DocKind, Session};
use serde_json::{json, Value};
use std::sync::Arc;

/// What Word 16.106 for Mac wrote: per case the tracked body (Word's own deletion, or one
/// written as Word writes it) and the body after Word's Accept All.
const OBSERVED: &str = include_str!("row-deletions-word.json");

/// `text` without the id comments and `data-id` attributes `word_read` writes.
fn without_ids(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("<!--") {
        out.push_str(rest[..at].strip_suffix(' ').unwrap_or(&rest[..at]));
        let end = rest[at..].find("-->").map_or(rest.len(), |e| at + e + 3);
        rest = rest[end..].strip_prefix('\n').unwrap_or(&rest[end..]);
    }
    out.push_str(rest);
    let mut clean = String::new();
    let mut rest = out.as_str();
    while let Some(at) = rest.find(" data-id=\"") {
        clean.push_str(&rest[..at]);
        let after = &rest[at + " data-id=\"".len()..];
        rest = after.find('"').map_or("", |e| &after[e + 1..]);
    }
    clean.push_str(rest);
    clean
}

/// The table properties of `body`, those of `saved`'s table where both have one: Word writes
/// its own (`w:tblLook` among them, which decides a header row) when it saves a file, so
/// the rows are compared under the same ones.
fn with_table_properties(body: &str, saved: &str) -> String {
    let properties = |xml: &str| {
        let start = xml.find("<w:tblPr>")?;
        let end = xml[start..].find("</w:tblPr>")? + start + "</w:tblPr>".len();
        Some((start, end))
    };
    match (properties(body), properties(saved)) {
        (Some((start, end)), Some((from, to))) => format!("{}{}{}", &body[..start], &saved[from..to], &body[end..]),
        _ => body.to_owned(),
    }
}

/// `word_read` of `doc` in `view`: the reply.
fn read(s: &Session, doc: &str, view: &str) -> Value {
    call(s, "word_read", json!({"doc": doc, "view": view}), &as_caller("r"))
}

/// The Markdown of a reply, ids left out.
fn markdown(reply: &Value) -> String {
    assert_eq!(reply["ok"], json!(true), "{reply}");
    without_ids(reply["data"].as_str().unwrap())
}

/// `word_inspect` of `target` answers.
fn inspects(s: &Session, doc: &str, target: &str) {
    let reply = call(
        s,
        "word_inspect",
        json!({"doc": doc, "target": target}),
        &as_caller("r"),
    );
    assert_eq!(reply["ok"], json!(true), "{target}: {reply}");
}

#[test]
fn the_final_view_of_each_pending_row_deletion_is_word_s_accept_all() {
    let observed: Value = serde_json::from_str(OBSERVED).unwrap();
    let cases = observed.as_object().unwrap();
    assert!(cases.len() >= 10, "{}", cases.len());
    for (name, case) in cases.iter().filter(|(name, _)| !name.starts_with('_')) {
        let s = session();
        let accepted = case["accepted_body"].as_str().unwrap();
        let tracked = with_table_properties(case["tracked_body"].as_str().unwrap(), accepted);
        s.open(docx(&tracked), DocKind::Docx, "tracked.docx").unwrap();
        s.open(docx(accepted), DocKind::Docx, "accepted.docx").unwrap();
        let ours = markdown(&read(&s, "d1", "final"));
        let words = markdown(&read(&s, "d2", "final"));
        assert_eq!(ours, words, "{name}: our final view, then Word's Accept All");
        inspects(&s, "d1", "body");
        // the markup view keeps the rows, struck through
        assert_ne!(markdown(&read(&s, "d1", "markup")), words, "{name}");
    }
}

/// A cell of one paragraph, `text` in it.
fn cell(id: u32, text: &str) -> String {
    format!(r#"<w:tc><w:p w14:paraId="{id:08X}" w14:textId="77777777"><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>"#)
}

/// The team's table: three rows (GV/GV.OC/2, GV/GV.RM/3, ID/ID.AM/1) and a paragraph after.
fn team() -> String {
    let rows = [["GV", "GV.OC", "2"], ["GV", "GV.RM", "3"], ["ID", "ID.AM", "1"]];
    let mut body = String::from(
        r#"<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="2400"/><w:gridCol w:w="2400"/><w:gridCol w:w="2400"/></w:tblGrid>"#,
    );
    for (r, row) in (0u32..).zip(rows) {
        let base = 0xA00 + r * 0x10;
        body.push_str(&format!(r#"<w:tr w14:paraId="{base:08X}" w14:textId="77777777">"#));
        for (c, text) in (1u32..).zip(row) {
            body.push_str(&cell(base + c, text));
        }
        body.push_str("</w:tr>");
    }
    body.push_str(r#"</w:tbl><w:p w14:paraId="00000B00" w14:textId="77777777"><w:r><w:t>After.</w:t></w:r></w:p>"#);
    body
}

/// Every target the document has answers word_inspect, and the final view reads.
fn reads_everywhere(s: &Session, doc: &str) -> String {
    for target in ["body", "t:00000A00", "p:00000A01", "p:00000B00"] {
        inspects(s, doc, target);
    }
    markdown(&read(s, doc, "final"))
}

#[test]
fn a_tracked_row_deletion_of_original_text_reads_before_and_after_a_reopen() {
    let s = session();
    let d = s.open(docx(&team()), DocKind::Docx, "team.docx").unwrap();
    let context = as_caller("w1");
    let on = call(
        &s,
        "word_set_tracking",
        json!({"doc": "d1", "on": true, "author": "Reviewer"}),
        &context,
    );
    assert_eq!(on["ok"], json!(true), "{on}");
    let deleted = call(
        &s,
        "word_edit_table",
        json!({"doc": "d1", "table": "t:00000A00", "action": "delete_row", "index": 2}),
        &context,
    );
    assert_eq!(deleted["ok"], json!(true), "{deleted}");
    let before = reads_everywhere(&s, "d1");
    assert!(before.contains("GV.OC") && before.contains("GV.RM"), "{before}");
    assert!(!before.contains("ID.AM"), "the deleted row is gone: {before}");
    assert!(markdown(&read(&s, "d1", "markup")).contains("ID.AM"));

    // saved and opened again, as a client file arrives with a reviewer's pending deletion
    let saved: Arc<[u8]> = Arc::from(s.save(&d).unwrap());
    let again = session();
    let reopened: DocId = again.open(saved, DocKind::Docx, "team.docx").unwrap();
    assert_eq!(reads_everywhere(&again, "d1"), before);

    // the table takes the next tracked deletion, counted as accepted (the deleted row left out)
    let on = call(
        &again,
        "word_set_tracking",
        json!({"doc": "d1", "on": true, "author": "Reviewer"}),
        &context,
    );
    assert_eq!(on["ok"], json!(true), "{on}");
    let next = call(
        &again,
        "word_edit_table",
        json!({"doc": "d1", "table": "t:00000A00", "action": "delete_row", "index": 1}),
        &context,
    );
    assert_eq!(next["ok"], json!(true), "{next}");
    let after = reads_everywhere(&again, "d1");
    assert!(
        after.contains("GV.OC") && !after.contains("GV.RM") && !after.contains("ID.AM"),
        "{after}"
    );
    assert!(again.save(&reopened).is_ok());
}
