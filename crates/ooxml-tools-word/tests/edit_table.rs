//! `word_edit_table`'s handlers on a made-up document, for the tool design rules they
//! touch: a column inserted, a row copied (what it leaves on the row copied said), and a
//! refusal carrying its way out (rule 2: a column past the last names the columns; a row
//! not copied gives the calls that add a row like it and fill it). Their equality with
//! docx-agent is `crates/ooxml-oracle/python/word_edits_diff.py --objects`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::{Cancel, Limits};
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, StoreConfig};
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{word_edit_table, Editing, WordEditTable};
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
    let editor = Editor::open(package(), StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let editing = Editing {
        editor: &editor,
        caller: &caller,
        opened: &opened,
        tracking: None,
        doc: "d1",
        cancel: &Never,
        date: "2026-11-17T10:00:00Z",
        layout: None,
    };
    let mut arguments = arguments;
    arguments["doc"] = json!("d1");
    arguments["table"] = json!("t:00000A00");
    let (args, _) = WordEditTable::accept(arguments, Some("docx")).unwrap();
    Value::Object(word_edit_table(&args, &editing).to_json())
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
