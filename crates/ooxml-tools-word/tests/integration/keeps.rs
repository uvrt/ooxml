//! `keep_together` in `word_format` and `word_format_table` (lane E2): a span kept together
//! that the version before the change -- laid out by the document's last check -- draws
//! taller than a page is said so (docx-agent's `_laid_before`, `_taller_than_page`), and
//! nothing is said without that check; a table kept together is laid out after the change
//! and said taller than a page where it is drawn on more than one, and where no layout is
//! handed the keep is made and the warning says its pages are not known. Equality with
//! docx-agent is `crates/ooxml-oracle/python/word_style_cases.py` and
//! `word_object_cases.py`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::check::checked;
use crate::common::{engine, Never, Still, Unlimited, R, W};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{
    word_format, word_format_table, CallEditor, Editing, EnginePages, LastCheck, WordFormat, WordFormatTable,
};
use ooxml_word_edit::Editor;
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::{json, Value};
use std::sync::Arc;

/// Paragraphs in the made-up documents: many more lines than a page holds.
const LINES: usize = 90;

fn package(body: &str) -> Package {
    let rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}officeDocument" Target="word/document.xml"/></Relationships>"#
    );
    let document = format!("<w:document {W}><w:body>{body}</w:body></w:document>");
    let mut w = Writer::new();
    for (name, data) in [("_rels/.rels", rels), ("word/document.xml", document)] {
        let entry = NewEntry {
            name: name.into(),
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

fn p(k: usize) -> String {
    format!(
        r#"<w:p w14:paraId="{:08X}"><w:r><w:t>Line {k}</w:t></w:r></w:p>"#,
        k + 1
    )
}

/// A table of `rows` one-cell rows after a paragraph.
fn table(rows: usize) -> String {
    let rows: String = (0..rows)
        .map(|k| {
            format!(
                r#"<w:tr w14:paraId="{:08X}"><w:tc><w:p w14:paraId="{:08X}"><w:r><w:t>Row {k}</w:t></w:r></w:p></w:tc></w:tr>"#,
                0x1000 + k,
                0x2000 + k
            )
        })
        .collect();
    format!(
        r#"{}<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>{rows}</w:tbl>{}"#,
        p(0),
        p(1)
    )
}

/// `word_format` with `arguments` on `package`, measured against what `last` saw.
fn formatted(package: &Package, arguments: Value, last: &LastCheck) -> Value {
    let editor = Editor::open(
        package.clone(),
        StoreConfig::default(),
        Arc::new(Still),
        Arc::new(Unlimited),
    )
    .unwrap();
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let call_editor = CallEditor::new(&editor);
    let editing =
        Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-17T10:00:00Z").with_last_check(Some(last));
    let (args, _) = WordFormat::accept(arguments, Some("docx")).unwrap();
    Value::Object(word_format(&args, &editing).to_json())
}

/// `word_format_table` with `arguments` on `package`, laid out by `layout` where given.
fn table_formatted(package: &Package, arguments: Value, layout: Option<&EnginePages>) -> Value {
    let editor = Editor::open(
        package.clone(),
        StoreConfig::default(),
        Arc::new(Still),
        Arc::new(Unlimited),
    )
    .unwrap();
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let call_editor = CallEditor::new(&editor);
    let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-17T10:00:00Z")
        .with_layout(layout.map(|l| l as &dyn ooxml_word_edit::FieldLayout));
    let (args, _) = WordFormatTable::accept(arguments, Some("docx")).unwrap();
    Value::Object(word_format_table(&args, &editing).to_json())
}

fn warnings(reply: &Value) -> Vec<String> {
    reply["warnings"]
        .as_array()
        .map(|w| w.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The version number a freshly opened editor's head has.
fn head_number(package: &Package) -> u64 {
    let editor = Editor::open(
        package.clone(),
        StoreConfig::default(),
        Arc::new(Still),
        Arc::new(Unlimited),
    )
    .unwrap();
    editor.store().snapshot().seq().0
}

#[test]
fn a_span_taller_than_a_page_the_last_check_laid_out_cannot_be_kept() {
    let package = package(&(0..LINES).map(p).collect::<String>());
    let span = format!("p:{:08X}..p:{:08X}", 1, LINES - 1);
    let arguments = json!({"doc": "d1", "targets": [span], "keep_together": true});
    // no check laid this version out: nothing is measured, the keep is made
    let unmeasured = formatted(&package, arguments.clone(), &LastCheck::default());
    assert!(unmeasured.get("error").is_none(), "{unmeasured}");
    assert!(warnings(&unmeasured).is_empty(), "{unmeasured}");
    // laid out by a check of this very version: said taller than a page
    let layout = EnginePages::of(engine());
    let last = LastCheck::default();
    checked(
        &package,
        head_number(&package),
        json!({"doc": "d1", "include": ["layout"]}),
        &last,
        Some(&layout),
    );
    let measured = formatted(&package, arguments.clone(), &last);
    let said = warnings(&measured);
    assert_eq!(said.len(), 1, "{measured}");
    assert!(said[0].starts_with(&format!("{span} is ")), "{measured}");
    assert!(said[0].contains("pt tall and a page holds"), "{measured}");
    assert!(said[0].ends_with("cannot be kept on one page (Word starts it on a new page and breaks it where the page is full); keep shorter spans together"), "{measured}");
    // a check of another version is not this one's layout
    let other = LastCheck::default();
    checked(
        &package,
        head_number(&package) + 1,
        json!({"doc": "d1", "include": ["layout"]}),
        &other,
        Some(&layout),
    );
    assert!(warnings(&formatted(&package, arguments, &other)).is_empty());
    // a short span fits: nothing said
    let short = json!({"doc": "d1", "targets": [format!("p:{:08X}..p:{:08X}", 1, 3)], "keep_together": true});
    assert!(warnings(&formatted(&package, short, &last)).is_empty());
}

#[test]
fn a_table_kept_together_is_laid_out_after_the_change() {
    let tall = package(&table(LINES));
    let arguments = json!({"doc": "d1", "table": "t:00001000", "scope": "table", "keep_together": true});
    let layout = EnginePages::of(engine());
    let reply = table_formatted(&tall, arguments.clone(), Some(&layout));
    assert_eq!(
        warnings(&reply),
        ["t:00001000 is taller than a page: Word starts it on a new page and breaks it where the page is full"],
        "{reply}"
    );
    let short = package(&table(3));
    let fits = table_formatted(&short, arguments.clone(), Some(&layout));
    assert!(warnings(&fits).is_empty(), "{fits}");
    // without a layout the keep is made, and its pages are said not known
    let unlaid = table_formatted(&short, arguments, None);
    assert!(unlaid.get("error").is_none(), "{unlaid}");
    assert_eq!(unlaid["changed"], json!(["t:00001000"]), "{unlaid}");
    let said = warnings(&unlaid);
    assert_eq!(said.len(), 1, "{unlaid}");
    assert!(
        said[0].starts_with("t:00001000 is kept together, but the document could not be laid out to it"),
        "{unlaid}"
    );
}
