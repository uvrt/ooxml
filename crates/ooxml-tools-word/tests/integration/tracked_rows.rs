//! Tracked row deletions as Word makes them (round 51, R103; Word's observation
//! `crates/ooxml-oracle/word/observations/rows-word.json`): a table's last row that is the
//! author's own pending insertion, holding another author's insertion, goes, and what the
//! other author inserted moves to the paragraph after the table, deleted; a table the author
//! inserted beside a table, deleted, goes with the empty paragraph it came with, the
//! document as it was.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, Still, Unlimited, R, W};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{word_delete, word_edit_table, CallEditor, Editing, WordDelete, WordEditTable};
use ooxml_word_edit::{Editor, Tracking};
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::{json, Value};
use std::sync::Arc;

const DATE: &str = "2026-11-11T09:00:00Z";

/// A cell of one paragraph: plain, or (with `rows`) the inserted row's -- its mark `me`'s
/// insertion, its text `other`'s.
fn cell(id: u32, text: &str, inserted: bool) -> String {
    if inserted {
        format!(
            r#"<w:tc><w:p w14:paraId="{id:08X}" w14:textId="7{id:07X}"><w:pPr><w:rPr><w:ins w:id="{}" w:author="Writer" w:date="{DATE}"/></w:rPr></w:pPr><w:ins w:id="{}" w:author="Other" w:date="{DATE}"><w:r><w:t>{text}</w:t></w:r></w:ins></w:p></w:tc>"#,
            id + 1,
            id + 2
        )
    } else {
        format!(r#"<w:tc><w:p w14:paraId="{id:08X}" w14:textId="7{id:07X}"><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>"#)
    }
}

/// A made-up document: a table of three rows, its last `Writer`'s pending insertion holding
/// `Other`'s text, and a paragraph after it.
fn package() -> Package {
    package_of(&rows_body())
}

/// [`package`]'s body.
fn rows_body() -> String {
    format!(
        concat!(
            r#"<w:tbl><w:tblPr><w:tblW w:w="4000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>"#,
            r#"<w:tr w14:paraId="00000A00" w14:textId="70000A00">{}{}</w:tr>"#,
            r#"<w:tr w14:paraId="00000B00" w14:textId="70000B00">{}{}</w:tr>"#,
            r#"<w:tr w14:paraId="00000C00" w14:textId="70000C00"><w:trPr><w:ins w:id="1" w:author="Writer" w:date="{DATE}"/></w:trPr>{}{}</w:tr>"#,
            r#"</w:tbl><w:p w14:paraId="00000D01" w14:textId="70000D01"><w:r><w:t>After.</w:t></w:r></w:p>"#
        ),
        cell(0xA01, "A", false),
        cell(0xA02, "a", false),
        cell(0xB01, "B", false),
        cell(0xB02, "b", false),
        cell(0xC01, "Theirs", true),
        cell(0xC10, "", true),
        DATE = DATE
    )
}

/// A made-up document of `body`.
fn package_of(body: &str) -> Package {
    let parts = [
        ("[Content_Types].xml".to_owned(), r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_owned()),
        ("_rels/.rels".to_owned(), format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}officeDocument" Target="word/document.xml"/></Relationships>"#)),
        ("word/document.xml".to_owned(), format!("<w:document {W}><w:body>{body}</w:body></w:document>")),
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

/// `word_edit_table` with `arguments`, tracked by `Writer`: the reply and the body after.
fn tracked(arguments: Value) -> (Value, String) {
    tracked_on(package(), "word_edit_table", arguments)
}

/// `tool` with `arguments` on `package`, tracked by `Writer`: the reply and the body after.
fn tracked_on(package: Package, tool: &str, arguments: Value) -> (Value, String) {
    let editor = Editor::open(package, StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let call_editor = CallEditor::new(&editor);
    let tracking = Tracking {
        author: "Writer".to_owned(),
        date: DATE.to_owned(),
    };
    let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, DATE).with_tracking(Some(&tracking));
    let mut arguments = arguments;
    arguments["doc"] = json!("d1");
    let reply = Value::Object(
        if tool == "word_delete" {
            word_delete(&WordDelete::accept(arguments, Some("docx")).unwrap().0, &editing)
        } else {
            arguments["table"] = json!("t:00000A00");
            word_edit_table(&WordEditTable::accept(arguments, Some("docx")).unwrap().0, &editing)
        }
        .to_json(),
    );
    let snapshot = editor.store().snapshot();
    let tree = snapshot.doc().tree("word/document.xml").unwrap().unwrap();
    (reply, String::from_utf8(tree.write()).unwrap())
}

#[test]
fn ones_own_last_row_goes_and_anothers_text_in_it_moves_after_the_table() {
    let (reply, body) = tracked(json!({"action": "delete_row", "index": 2}));
    assert_eq!(reply["ok"], json!(true), "{reply}");
    assert!(!body.contains("00000C00"), "the row stays: {body}");
    assert_eq!(body.matches("<w:tr ").count(), 2, "{body}");
    let after = &body[body.find("</w:tbl>").unwrap()..];
    let theirs = after
        .find(r#"w:author="Other""#)
        .expect("their insertion moved after the table");
    let deleted = after.find("<w:delText>Theirs</w:delText>").expect("deleted");
    assert!(theirs < deleted && deleted < after.find("After.").unwrap(), "{after}");
    assert!(
        reply["removed"].as_array().unwrap().iter().any(|r| r == "p:00000C01"),
        "{reply}"
    );
}

/// The body element of `xml` (a document part's), its children as written.
fn body_of(xml: &str) -> &str {
    let start = xml.find("<w:body>").unwrap() + "<w:body>".len();
    &xml[start..xml.find("</w:body>").unwrap()]
}

#[test]
fn a_table_one_inserted_beside_a_table_goes_with_its_empty_paragraph() {
    // Start, table One, the empty paragraph Writer inserted with the new table (between it
    // and One), the new table (all Writer's), a paragraph, table Two
    let plain = |id: u32, text: &str| {
        format!(r#"<w:p w14:paraId="{id:08X}" w14:textId="7{id:07X}"><w:r><w:t>{text}</w:t></w:r></w:p>"#)
    };
    let empty = |id: u32| format!(r#"<w:p w14:paraId="{id:08X}" w14:textId="7{id:07X}"/>"#);
    let table = |row: u32, text: &str| {
        format!(
            r#"<w:tbl><w:tblPr><w:tblW w:w="2000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr w14:paraId="{row:08X}" w14:textId="7{row:07X}">{}</w:tr></w:tbl>"#,
            cell(row + 1, text, false)
        )
    };
    let ins = |id: u32| format!(r#"<w:ins w:id="{id}" w:author="Writer" w:date="{DATE}"/>"#);
    let new = format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="2000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr w14:paraId="00000E00" w14:textId="70000E00"><w:trPr>{}</w:trPr><w:tc><w:p w14:paraId="00000E01" w14:textId="70000E01"><w:pPr><w:rPr>{}</w:rPr></w:pPr><w:ins w:id="12" w:author="Writer" w:date="{DATE}"><w:r><w:t>new</w:t></w:r></w:ins></w:p></w:tc></w:tr></w:tbl>"#,
        ins(10),
        ins(11)
    );
    let before = [
        plain(0x100, "Start."),
        table(0x200, "One"),
        empty(0x300),
        table(0x400, "Two"),
        empty(0x500),
    ]
    .concat();
    let separator = format!(
        r#"<w:p w14:paraId="00000D00" w14:textId="70000D00"><w:pPr><w:rPr>{}</w:rPr></w:pPr></w:p>"#,
        ins(9)
    );
    let made = [
        plain(0x100, "Start."),
        table(0x200, "One"),
        separator,
        new,
        empty(0x300),
        table(0x400, "Two"),
        empty(0x500),
    ]
    .concat();
    let (reply, body) = tracked_on(package_of(&made), "word_delete", json!({"target": "t:00000E00"}));
    assert_eq!(reply["ok"], json!(true), "{reply}");
    let expected = package_of(&before).read("word/document.xml").unwrap().unwrap();
    assert_eq!(
        body_of(&body),
        body_of(std::str::from_utf8(&expected).unwrap()),
        "{reply}"
    );
    assert!(
        reply["removed"].as_array().unwrap().iter().any(|r| r == "p:00000D00"),
        "{reply}"
    );
}
