//! A session owning Word documents (phase P2, lane P2c) on made-up packages: the editing
//! tools run on the document's editor with the tracking mode the session keeps; the
//! session's undo and redo, and a caller's own; the save gate; the bridge's update adding
//! the ids it brings to the ledger.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::context::{FaceData, FaceRequest, Timestamp};
use ooxml_base::{BaseError, CacheBudget, Cancel, Clock, Context, FontSource, Limits, Locale, PartStore};
use ooxml_session::{CallContext, Caller, DocId, DocKind, Session, SessionConfig, SessionError, Undone};
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

struct NoFonts;
impl FontSource for NoFonts {
    fn face(&self, _: &FaceRequest) -> Result<Option<FaceData>, BaseError> {
        Ok(None)
    }
}
struct NoParts;
impl PartStore for NoParts {
    fn load(&self, _: &str) -> Result<Option<Arc<[u8]>>, BaseError> {
        Ok(None)
    }
    fn store(&self, _: &str, _: Arc<[u8]>) -> Result<(), BaseError> {
        Ok(())
    }
}
struct Unlimited;
impl CacheBudget for Unlimited {
    fn reserve(&self, _: usize) -> bool {
        true
    }
    fn release(&self, _: usize) {}
    fn used(&self) -> usize {
        0
    }
}
struct Never;
impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}
struct Fixed;
impl Clock for Fixed {
    fn now(&self) -> Timestamp {
        Timestamp {
            seconds: 1_795_000_000,
            nanos: 0,
        }
    }
    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

fn session() -> Session {
    let context = Context {
        fonts: Arc::new(NoFonts),
        parts: Arc::new(NoParts),
        clock: Arc::new(Fixed),
        budget: Arc::new(Unlimited),
        cancel: Arc::new(Never),
        limits: Limits::default(),
        locale: Locale::default(),
    };
    Session::new(SessionConfig::default(), context)
}

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#;

fn paragraph(id: &str, text: &str) -> String {
    format!(r#"<w:p w14:paraId="{id}" w14:textId="77777777"><w:r><w:t>{text}</w:t></w:r></w:p>"#)
}

fn document(body: &str) -> String {
    format!("<w:document {NS}><w:body>{body}</w:body></w:document>")
}

fn docx(body: &str) -> Arc<[u8]> {
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    let mut w = Writer::new();
    for (name, data) in [
        ("[Content_Types].xml", types.to_owned()),
        ("_rels/.rels", rels.to_owned()),
        ("word/document.xml", document(body)),
    ] {
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
    Arc::from(w.finish(b"").unwrap())
}

fn as_caller(name: &str) -> CallContext {
    CallContext {
        caller: Caller::named(name),
        ..CallContext::default()
    }
}

fn call(session: &Session, name: &str, arguments: Value, context: &CallContext) -> Value {
    Value::Object(session.call(name, arguments, context).to_json())
}

fn put(session: &Session, doc: &DocId, body: &str, context: &CallContext) -> u64 {
    let parts = vec![(
        "word/document.xml".to_owned(),
        Some(Arc::from(document(body).as_bytes())),
    )];
    session.update(doc, parts, context).unwrap().version
}

fn body_of(session: &Session, doc: &DocId) -> String {
    let saved = session.save(doc).unwrap();
    let package = ooxml_opc::Package::open(Arc::from(saved), &Limits::default()).unwrap();
    String::from_utf8(package.read("word/document.xml").unwrap().unwrap().to_vec()).unwrap()
}

#[test]
fn the_tracking_mode_is_kept_and_review_tools_edit() {
    let s = session();
    let tracked = format!(
        r#"{}<w:p w14:paraId="22222222" w14:textId="77777777"><w:ins w:id="1" w:author="Ann" w:date="2026-01-01T00:00:00Z"><w:r><w:t>new</w:t></w:r></w:ins></w:p>"#,
        paragraph("11111111", "one")
    );
    let d = s.open(docx(&tracked), DocKind::Docx, "a.docx").unwrap();
    let context = as_caller("w1");
    let on = call(
        &s,
        "word_set_tracking",
        json!({"doc": "d1", "on": true, "author": "Bo"}),
        &context,
    );
    assert_eq!(on["data"]["author"], json!("Bo"), "{on}");
    let listed = call(&s, "word_changes", json!({"doc": "d1", "action": "list"}), &context);
    assert_eq!(listed["ok"], json!(true), "{listed}");
    let accepted = call(
        &s,
        "word_changes",
        json!({"doc": "d1", "action": "accept", "all": true}),
        &context,
    );
    assert_eq!(accepted["ok"], json!(true), "{accepted}");
    assert_eq!(accepted["version"], json!(1));
    assert!(!body_of(&s, &d).contains("w:ins"));
    // the session's undo takes the accept back
    assert_eq!(
        s.undo(&d, 1, &CallContext::default()).unwrap(),
        Undone { count: 1, version: 2 }
    );
    assert!(body_of(&s, &d).contains("w:ins"));
    let off = call(&s, "word_set_tracking", json!({"doc": "d1", "on": false}), &context);
    assert_eq!(off["data"]["tracking"], json!(false), "{off}");
}

#[test]
fn a_caller_undoes_only_its_own_latest_change() {
    let s = session();
    let d = s
        .open(docx(&paragraph("11111111", "one")), DocKind::Docx, "a.docx")
        .unwrap();
    let (ann, bo) = (as_caller("ann"), as_caller("bo"));
    put(&s, &d, &paragraph("11111111", "by ann"), &ann);
    put(&s, &d, &paragraph("11111111", "by bo"), &bo);
    match s.undo(&d, 1, &ann) {
        Err(SessionError::Entangled { step, later }) => {
            assert_eq!(step, 1);
            assert_eq!(later, vec![(2, Some("bo".to_owned()))]);
        }
        other => panic!("{other:?}"),
    }
    // bo's is the latest: undone; then ann's is, and redo brings back only one's own
    assert_eq!(s.undo(&d, 5, &bo).unwrap().count, 1);
    assert_eq!(s.undo(&d, 1, &ann).unwrap().count, 1);
    assert!(body_of(&s, &d).contains(">one<"));
    assert_eq!(s.redo(&d, 1, &bo).unwrap().count, 0, "ann's undo is the next to redo");
    assert_eq!(s.redo(&d, 1, &ann).unwrap().count, 1);
    assert!(body_of(&s, &d).contains("by ann"));
    assert_eq!(s.redo(&d, 1, &CallContext::default()).unwrap().count, 1);
    assert!(body_of(&s, &d).contains("by bo"));
    assert_eq!(s.undo(&d, 0, &CallContext::default()).unwrap().count, 0);
}

#[test]
fn the_save_gate_refuses_new_problems_and_the_bridge_feeds_the_ledger() {
    let s = session();
    let d = s
        .open(docx(&paragraph("11111111", "one")), DocKind::Docx, "a.docx")
        .unwrap();
    assert!(s.validation(&d).unwrap().unwrap().new.is_empty());
    s.save_checked(&d).unwrap();
    // a bridged tool repeats a paragraph id: a new problem, and saving is refused
    let twice = format!("{}{}", paragraph("11111111", "one"), paragraph("11111111", "two"));
    put(&s, &d, &twice, &CallContext::default());
    match s.save_checked(&d) {
        Err(SessionError::NewProblems { new }) => assert!(!new.is_empty()),
        other => panic!("{other:?}"),
    }
    // ids a bridged tool brings are in the ledger, and stay there after undo
    let fresh = format!("{}{}", paragraph("11111111", "one"), paragraph("3A3A3A3A", "two"));
    put(&s, &d, &fresh, &CallContext::default());
    s.save_checked(&d).unwrap();
    s.undo(&d, 2, &CallContext::default()).unwrap();
    let head = s.snapshot(&d).unwrap();
    assert_eq!(head.version(), 4);
    assert!(head.word().unwrap().ids().contains(0x3A3A_3A3A));
}

#[test]
fn bridged_calls_are_counted_by_tool() {
    let s = session();
    let d = s
        .open(docx(&paragraph("11111111", "one")), DocKind::Docx, "a.docx")
        .unwrap();
    let parts = |text: &str| {
        vec![(
            "word/document.xml".to_owned(),
            Some(Arc::from(document(&paragraph("11111111", text)).as_bytes())),
        )]
    };
    let (updated, opened) = s
        .bridge(&d, "word_format", parts("two"), &CallContext::default())
        .unwrap();
    assert_eq!((updated.version, opened.version()), (1, 1));
    s.bridge(&d, "word_format", parts("three"), &CallContext::default())
        .unwrap();
    s.bridge(&d, "word_move", parts("four"), &CallContext::default())
        .unwrap();
    let counts: Vec<(String, u64)> = s.bridge_calls().into_iter().collect();
    assert_eq!(counts, vec![("word_format".to_owned(), 2), ("word_move".to_owned(), 1)]);
}

#[test]
fn the_text_tools_edit_through_dispatch() {
    let s = session();
    let body = format!(
        "{}{}",
        paragraph("11111111", "one word here"),
        paragraph("22222222", "two word there")
    );
    let d = s.open(docx(&body), DocKind::Docx, "a.docx").unwrap();
    let context = as_caller("w1");
    let set = call(
        &s,
        "word_set_text",
        json!({"doc": "d1", "target": "p:11111111", "text": "First line\nSecond line"}),
        &context,
    );
    assert_eq!(
        set["summary"],
        json!("Set the text of 1 paragraph(s), 1 new after them"),
        "{set}"
    );
    assert_eq!(set["changed"], json!(["p:11111111"]));
    // rule 3: the same text again changes nothing, and says so without failing
    let same = call(
        &s,
        "word_set_text",
        json!({"doc": "d1", "target": "p:11111111", "text": "First line"}),
        &context,
    );
    assert_eq!(same["ok"], json!(true), "{same}");
    assert_eq!(same["changed"], json!([]));
    // several items are one step: one undo takes both back
    let items = call(
        &s,
        "word_set_text",
        json!({"doc": "d1", "items": [
            {"target": "p:11111111", "text": "A"}, {"target": "p:22222222", "text": "B"}]}),
        &context,
    );
    assert_eq!(items["summary"], json!("Set the text of 2 paragraph(s)"), "{items}");
    s.undo(&d, 1, &context).unwrap();
    assert!(body_of(&s, &d).contains("two word there"));
    // rule 2: a span is refused with the call that would work
    let span = s.call(
        "word_set_text",
        json!({"doc": "d1", "target": "p:11111111..p:22222222", "text": "x"}),
        &context,
    );
    let span = Value::Object(span.to_json());
    assert_eq!(span["error"]["code"], json!("invalid_arguments"), "{span}");
    assert!(span["error"]["try_call"].is_object(), "{span}");
    // what is not ported yet changes nothing and says so to the host
    let styled = s.call(
        "word_set_text",
        json!({"doc": "d1", "target": "p:22222222", "text": "a **strong** word"}),
        &context,
    );
    assert!(!styled.unported().is_empty());
    assert!(!body_of(&s, &d).contains("strong"));
    // replace_text: one expected, two found: refused with each listed
    let two = call(
        &s,
        "replace_text",
        json!({"doc": "d1", "find": "line", "replace": "row"}),
        &context,
    );
    assert_eq!(two["error"]["code"], json!("ambiguous"), "{two}");
    assert_eq!(two["error"]["valid_options"].as_array().map(Vec::len), Some(2), "{two}");
    let all = call(
        &s,
        "replace_text",
        json!({"doc": "d1", "find": "line", "replace": "row", "expect": "all"}),
        &context,
    );
    assert_eq!(all["data"]["count"], json!(2), "{all}");
    assert!(body_of(&s, &d).contains("Second row"));
    let none = call(
        &s,
        "replace_text",
        json!({"doc": "d1", "find": "absent", "replace": "x"}),
        &context,
    );
    assert_eq!(none["error"]["message"], json!("'absent' occurs nowhere"), "{none}");
}
