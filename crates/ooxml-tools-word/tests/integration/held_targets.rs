//! Targets held across edits: a positional table id inside a cell address once the first
//! edit stamps the ids (`t@body/0/c1,0` naming the table it named), and `word_lists`
//! continue given several lists' first items, each continued in turn.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, Still, Unlimited, R, W};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{
    word_edit_table, word_format, word_lists, CallEditor, Editing, WordEditTable, WordFormat, WordLists,
};
use ooxml_word_edit::Editor;
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::{json, Value};
use std::sync::Arc;

/// A made-up document of `body`: the style Numbered numbering with list 1; lists 2 and 3
/// over the same definition.
fn package(body: &str) -> Package {
    let parts = [
        ("[Content_Types].xml".to_owned(), r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/></Types>"#.to_owned()),
        ("word/_rels/document.xml.rels".to_owned(), format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}styles" Target="styles.xml"/><Relationship Id="rId2" Type="{R}numbering" Target="numbering.xml"/></Relationships>"#)),
        ("word/styles.xml".to_owned(), format!(r#"<w:styles {W}><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Numbered"><w:name w:val="Numbered"/><w:basedOn w:val="Normal"/><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr></w:style></w:styles>"#)),
        ("word/numbering.xml".to_owned(), format!(r#"<w:numbering {W}><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:pStyle w:val="Numbered"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="0"/></w:num><w:num w:numId="3"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#)),
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

/// Each of `calls` (`(tool, arguments)`) in turn on one editor of `body`: the replies.
fn calls(body: &str, calls: &[(&str, Value)]) -> Vec<Value> {
    let editor = Editor::open(
        package(body),
        StoreConfig::default(),
        Arc::new(Still),
        Arc::new(Unlimited),
    )
    .unwrap();
    let caller = Caller::anonymous();
    let call_editor = CallEditor::new(&editor);
    calls
        .iter()
        .map(|(tool, arguments)| {
            let opened = editor.store().snapshot().doc().clone();
            let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-23T10:00:00Z");
            let mut arguments = arguments.clone();
            arguments["doc"] = json!("d1");
            let result = match *tool {
                "word_edit_table" => {
                    word_edit_table(&WordEditTable::accept(arguments, Some("docx")).unwrap().0, &editing)
                }
                "word_format" => word_format(&WordFormat::accept(arguments, Some("docx")).unwrap().0, &editing),
                _ => word_lists(&WordLists::accept(arguments, Some("docx")).unwrap().0, &editing),
            };
            Value::Object(result.to_json())
        })
        .collect()
}

#[test]
fn a_cell_address_in_a_table_without_ids_names_its_table_across_edits() {
    // no paraIds in the bytes: the table has its id from the first read, and keeps it
    let cell = |r: u32, c: u32| format!("<w:tc><w:p><w:r><w:t>r{r}c{c}</w:t></w:r></w:p></w:tc>");
    let row = |r: u32| format!("<w:tr>{}{}</w:tr>", cell(r, 0), cell(r, 1));
    let body = format!(
        r#"<w:p><w:r><w:t>Before.</w:t></w:r></w:p><w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>{}{}</w:tbl><w:p/>"#,
        row(0),
        row(1)
    );
    let now = |value: Value| crate::common::now(&package(&body), value);
    let replies = calls(
        &body,
        &[
            (
                "word_edit_table",
                now(json!({"table": "t@body/0", "action": "insert_row", "index": 1})),
            ),
            ("word_format", now(json!({"targets": ["t@body/0/c1,0"], "bold": true}))),
            ("word_format", now(json!({"targets": ["t@body/0/c0,1"], "italic": true}))),
        ],
    );
    for reply in &replies {
        assert_eq!(reply["ok"], json!(true), "{reply}");
    }
}

#[test]
fn continue_given_several_lists_first_items_continues_each() {
    // a1 a2 in the style's list 1; b1 b2 in list 2, c1 c2 in list 3 (each its own numbering
    // over the style's definition): continuing b1 and c1 numbers them all on, 1. to 6.
    let item = |id: u32, text: &str, num: Option<u32>| {
        let own = num.map_or(String::new(), |n| {
            format!(r#"<w:numPr><w:ilvl w:val="0"/><w:numId w:val="{n}"/></w:numPr>"#)
        });
        format!(
            r#"<w:p w14:paraId="{id:08X}" w14:textId="20000001"><w:pPr><w:pStyle w:val="Numbered"/>{own}</w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
        )
    };
    let text = |id: u32, text: &str| {
        format!(r#"<w:p w14:paraId="{id:08X}" w14:textId="20000001"><w:r><w:t>{text}</w:t></w:r></w:p>"#)
    };
    let body = [
        item(0x1000_0001, "a1", None),
        item(0x1000_0002, "a2", None),
        text(0x1000_0003, "Text."),
        item(0x1000_0004, "b1", Some(2)),
        item(0x1000_0005, "b2", Some(2)),
        text(0x1000_0006, "More."),
        item(0x1000_0007, "c1", Some(3)),
        item(0x1000_0008, "c2", Some(3)),
    ]
    .concat();
    let replies = calls(
        &body,
        &[(
            "word_lists",
            json!({"targets": ["p:10000004", "p:10000007"], "action": "continue"}),
        )],
    );
    let reply = &replies[0];
    assert_eq!(reply["ok"], json!(true), "{reply}");
    assert_eq!(reply["changed"], json!(["p:10000004", "p:10000007"]), "{reply}");
    assert_eq!(reply["data"][0]["list"]["num_id"], json!(1), "{reply}");
    assert_eq!(reply["data"][1]["list"]["num_id"], json!(1), "{reply}");
}
