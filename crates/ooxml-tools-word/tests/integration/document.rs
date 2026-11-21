//! `set_properties` on made-up documents: a property set in place, a new one where Word writes
//! it, the core part made when the package has none, a value already there (no step), and the
//! refusal of none. Equality with docx-agent on fixtures and the corpus is
//! `crates/ooxml-oracle/python/word_edits_diff.py`'s (`--structure`, `property_cases`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, Still, Unlimited, R};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ToolResult;
use ooxml_tools_word::{set_properties, CallEditor, Editing, SetProperties};
use ooxml_word_edit::{Editor, WordDoc};
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::Value;
use std::sync::Arc;

const CORE: &str = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>Old</dc:title><cp:lastModifiedBy>A</cp:lastModifiedBy></cp:coreProperties>"#;

/// A one-paragraph document, with `core` as its core part when given.
fn package(core: Option<&str>) -> Package {
    let mut overrides = String::new();
    let mut rels = format!(r#"<Relationship Id="rId1" Type="{R}officeDocument" Target="word/document.xml"/>"#);
    let mut parts = Vec::new();
    if let Some(core) = core {
        overrides.push_str(r#"<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>"#);
        rels.push_str(r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>"#);
        parts.push(("docProps/core.xml".to_owned(), core.to_owned()));
    }
    parts.push((
        "[Content_Types].xml".to_owned(),
        format!(
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>{overrides}</Types>"#
        ),
    ));
    parts.push((
        "_rels/.rels".to_owned(),
        format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#),
    ));
    parts.push((
        "word/document.xml".to_owned(),
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Hi</w:t></w:r></w:p></w:body></w:document>"#.to_owned(),
    ));
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

fn editor(package: Package) -> (Editor, WordDoc) {
    let editor = Editor::open(package, StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let opened = editor.store().snapshot().doc().clone();
    (editor, opened)
}

fn args() -> SetProperties {
    SetProperties {
        doc: "d1".into(),
        title: None,
        author: None,
        language: None,
        subject: None,
    }
}

fn run(editor: &Editor, opened: &WordDoc, args: &SetProperties) -> Value {
    let caller = Caller::anonymous();
    let call = CallEditor::new(editor);
    let editing = Editing::new(&call, &caller, opened, "d1", &Never, "2026-11-17T10:00:00Z");
    let result: ToolResult<Value> = set_properties(args, &editing);
    Value::Object(result.to_json())
}

fn core(editor: &Editor, part: &str) -> String {
    let bytes = editor.store().snapshot().doc().package().read(part).unwrap().unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[test]
fn properties_are_set_in_place_and_where_word_writes_them() {
    let (editor, opened) = editor(package(Some(CORE)));
    let mut call = args();
    call.title = Some("Plan".into());
    call.author = Some("Writer A".into());
    call.language = Some("nl-NL".into());
    let reply = run(&editor, &opened, &call);
    assert_eq!(reply["summary"], "Set title, author, language");
    assert_eq!(reply["changed"], serde_json::json!(["properties"]));
    assert_eq!(
        reply["data"],
        serde_json::json!({"title": "Plan", "author": "Writer A", "language": "nl-NL", "subject": null})
    );
    let xml = core(&editor, "docProps/core.xml");
    assert!(
        xml.contains("<dc:title>Plan</dc:title><dc:creator>Writer A</dc:creator><cp:lastModifiedBy>A</cp:lastModifiedBy><dc:language>nl-NL</dc:language>"),
        "{xml}"
    );
}

#[test]
fn a_value_already_there_is_no_step() {
    let (editor, opened) = editor(package(Some(CORE)));
    let seq = editor.store().snapshot().seq();
    let mut call = args();
    call.title = Some("Old".into());
    let reply = run(&editor, &opened, &call);
    assert_eq!(reply["summary"], "Set title");
    assert!(
        reply
            .get("changed")
            .is_none_or(|c| c.as_array().is_some_and(Vec::is_empty)),
        "{reply}"
    );
    assert_eq!(editor.store().snapshot().seq(), seq);
}

#[test]
fn the_core_part_is_made_when_there_is_none() {
    let (editor, opened) = editor(package(None));
    let mut call = args();
    call.subject = Some("Budget".into());
    let reply = run(&editor, &opened, &call);
    assert_eq!(reply["data"]["subject"], "Budget");
    let xml = core(&editor, "docProps/core.xml");
    assert!(
        xml.ends_with("<dc:subject>Budget</dc:subject></cp:coreProperties>"),
        "{xml}"
    );
    assert!(core(&editor, "_rels/.rels").contains("Target=\"docProps/core.xml\""));
    assert!(core(&editor, "[Content_Types].xml").contains("/docProps/core.xml"));
}

#[test]
fn a_call_without_a_property_is_refused_with_the_names() {
    let (editor, opened) = editor(package(Some(CORE)));
    let reply = run(&editor, &opened, &args());
    assert_eq!(reply["error"]["code"], "invalid_arguments", "{reply}");
    assert_eq!(reply["error"]["message"], "give a property to set");
    assert_eq!(
        reply["error"]["valid_options"],
        serde_json::json!(["title", "author", "language", "subject"])
    );
}
