//! The structure tools' definitions: the golden exports docx-agent's Python test holds to
//! today's (`test_rust_structure.py`), and their argument rules (a place named another
//! placing tool's way read as the tool's own, or refused with the call that works).
//! Their handlers' equality with docx-agent is `crates/ooxml-oracle/python/word_edits_diff.py
//! --structure`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::{Cancel, Limits};
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, StoreConfig};
use ooxml_tools::{ToolArgs, ToolResult};
use ooxml_tools_word::{
    word_links, word_notes, CallEditor, Editing, WordCopyFrom, WordFields, WordHeadersFooters, WordLinks, WordMove,
    WordNotes, WordSections, KIND,
};
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

/// A made-up document: two paragraphs, a bookmark over the first one's word "Alpha".
fn package() -> Package {
    let body = concat!(
        r#"<w:p w14:paraId="10000001" w14:textId="20000001"><w:bookmarkStart w:id="0" w:name="First"/>"#,
        r#"<w:r><w:t>Alpha</w:t></w:r><w:bookmarkEnd w:id="0"/><w:r><w:t xml:space="preserve"> beta gamma</w:t></w:r></w:p>"#,
        r#"<w:p w14:paraId="10000002" w14:textId="20000002"><w:r><w:t>Delta epsilon</w:t></w:r></w:p>"#,
    );
    let parts = [
        ("[Content_Types].xml".to_owned(), r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#.to_owned()),
        ("word/_rels/document.xml.rels".to_owned(), format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}styles" Target="styles.xml"/></Relationships>"#)),
        ("word/styles.xml".to_owned(), format!(r#"<w:styles {W}><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style></w:styles>"#)),
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

/// `tool` called with `arguments` on a fresh copy of the made-up document.
fn call<A: ToolArgs>(arguments: Value, tool: impl Fn(&A, &Editing<'_>) -> ToolResult<Value>) -> (Value, Editor) {
    let editor = Editor::open(package(), StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let reply = {
        let editing = Editing {
            editor: &CallEditor::new(&editor),
            caller: &caller,
            opened: &opened,
            tracking: None,
            doc: "d1",
            cancel: &Never,
        };
        let (args, _) = A::accept(arguments, Some("docx")).unwrap();
        Value::Object(tool(&args, &editing).to_json())
    };
    (reply, editor)
}

#[test]
fn the_definitions_are_the_golden_exports() {
    for (spec, golden) in [
        (WordMove::spec(), include_str!("golden/word_move.json")),
        (WordCopyFrom::spec(), include_str!("golden/word_copy_from.json")),
        (WordSections::spec(), include_str!("golden/word_sections.json")),
        (
            WordHeadersFooters::spec(),
            include_str!("golden/word_headers_footers.json"),
        ),
        (WordFields::spec(), include_str!("golden/word_fields.json")),
        (WordNotes::spec(), include_str!("golden/word_notes.json")),
        (WordLinks::spec(), include_str!("golden/word_links.json")),
    ] {
        assert!(spec.problems().is_empty(), "{}: {:?}", spec.name, spec.problems());
        assert_eq!(format!("{}\n", spec.export(Some(&[KIND]))), golden, "{}", spec.name);
    }
}

#[test]
fn a_place_named_another_tools_way_is_read_as_its_own() {
    let (call, notes) = WordMove::accept(json!({"doc": "d1", "blocks": "p:A", "at": "before:p:B"}), None).unwrap();
    assert_eq!(call.after.as_deref(), Some("before:p:B"));
    assert_eq!(notes, ["at read as after"]);
    let (call, notes) = WordNotes::accept(json!({"doc": "d1", "action": "insert", "after": "p:A@3"}), None).unwrap();
    assert_eq!(call.at.as_deref(), Some("p:A@3"));
    assert_eq!(notes, ["after read as at"]);
}

#[test]
fn a_place_whose_side_is_in_its_name_is_refused_with_the_call() {
    let error = WordNotes::accept(json!({"doc": "d1", "action": "insert", "before": "p:A@3:9"}), None).unwrap_err();
    assert_eq!(error.field(), Some("before"));
    let call = error.way_out().and_then(|w| w.try_call()).unwrap();
    assert_eq!(call.arguments["at"], json!("p:A@3"));
    assert!(call.arguments.get("before").is_none());
}

#[test]
fn a_bookmark_marking_its_text_already_is_no_change() {
    let (reply, editor) = call(
        json!({"doc": "d1", "action": "add_bookmark", "target": "p:10000001@0:5", "bookmark": "first"}),
        word_links,
    );
    assert_eq!(reply["summary"], json!("No change: bm:First marks this text already"));
    assert_eq!(editor.store().snapshot().doc().package().part_names().len(), 5);
}

#[test]
fn a_bookmark_and_a_hyperlink_are_made_and_listed() {
    let (reply, _) = call(
        json!({"doc": "d1", "action": "add_bookmark", "find": "epsilon", "bookmark": "Second"}),
        word_links,
    );
    assert_eq!(reply["created"], json!(["bm:Second"]));
    let (reply, _) = call(
        json!({"doc": "d1", "action": "add_hyperlink", "target": "p:10000002@0:5", "bookmark": "First"}),
        word_links,
    );
    assert_eq!(reply["created"], json!(["hl:p:10000002/0"]));
}

#[test]
fn refusals_carry_their_way_out() {
    let (reply, _) = call(
        json!({"doc": "d1", "action": "add_bookmark", "target": "p:10000001", "bookmark": "1st"}),
        word_links,
    );
    assert_eq!(reply["error"]["code"], json!("refused"));
    let (reply, _) = call(
        json!({"doc": "d1", "action": "add_bookmark", "bookmark": "B"}),
        word_links,
    );
    assert_eq!(reply["error"]["code"], json!("invalid_arguments"));
    assert!(reply["error"]["message"]
        .as_str()
        .unwrap()
        .starts_with("give exactly one of target, find"));
    let (reply, _) = call(
        json!({"doc": "d1", "action": "insert", "find": "a", "text": "x"}),
        word_notes,
    );
    assert_eq!(reply["error"]["code"], json!("ambiguous"));
}

#[test]
fn a_first_footnote_brings_the_notes_parts() {
    let (reply, editor) = call(
        json!({"doc": "d1", "action": "insert", "at": "p:10000001@5", "text": "A source."}),
        word_notes,
    );
    assert_eq!(reply["summary"], json!("Inserted fn:1"));
    let head = editor.store().snapshot();
    let names = head.doc().package().part_names();
    assert!(names.iter().any(|n| n == "word/footnotes.xml"));
    assert!(names.iter().any(|n| n == "word/endnotes.xml"));
    let (reply, _) = call(json!({"doc": "d1", "action": "list"}), word_notes);
    assert_eq!(reply["data"], json!([]));
}
