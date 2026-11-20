//! A session owning Word documents (phase P2, lane P2c) on made-up packages: the editing
//! tools run on the document's editor with the tracking mode the session keeps; the
//! session's undo and redo, and a caller's own; the save gate; the bridge's update adding
//! the ids it brings to the ledger.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, NoFonts, NoParts, Unlimited};
use ooxml_base::context::Timestamp;
use ooxml_base::{Clock, Context, Limits, Locale};
use ooxml_session::{
    CallContext, Caller, DocId, DocKind, HeldId, RetiredBy, Session, SessionConfig, SessionError, Undone,
};
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

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
fn a_check_of_the_document_alone_is_the_sessions() {
    let s = session();
    let d = s
        .open(docx(&paragraph("11111111", "one")), DocKind::Docx, "a.docx")
        .unwrap();
    let asked = json!({"doc": "d1", "include": ["validate", "fields", "app"]});
    let reply = call(&s, "check", asked.clone(), &CallContext::default());
    assert_eq!(reply["summary"], "Checked validate, fields, app");
    let data = &reply["data"];
    assert_eq!(
        data["validate"],
        json!({"new": [], "fixed": [], "baseline": 0, "problems": []})
    );
    assert_eq!(data["fields"]["counts"], json!({}));
    assert_eq!(data["app_findings"], Value::Null);
    let parts: Vec<&String> = data["timing"].as_object().unwrap().keys().collect();
    assert_eq!(parts, ["validate", "fields"]);
    // a repeated paragraph id: a new problem, against the problems the document opened with
    let twice = format!("{}{}", paragraph("11111111", "one"), paragraph("11111111", "two"));
    put(&s, &d, &twice, &CallContext::default());
    let validate = &call(&s, "check", asked, &CallContext::default())["data"]["validate"];
    assert_eq!(validate["new"].as_array().unwrap().len(), 1);
    assert!(validate["new"][0].as_str().unwrap().starts_with("[paraid-repeated]"));
    assert_eq!(validate["problems"], validate["new"]);
    // what needs the layout's facts is the host's; a reflow, where it hands no layout
    for asked in [
        json!({"doc": "d1", "include": ["validate", "layout"]}),
        json!({"doc": "d1", "include": ["validate"], "range": "p:11111111"}),
    ] {
        let result = s.call("check", asked, &CallContext::default());
        assert_eq!(result.unported(), [ooxml_tools_word::CHECK_LAYOUT.to_owned()]);
    }
    for asked in [
        json!({"doc": "d1"}),
        json!({"doc": "d1", "include": ["validate", "reflow"]}),
    ] {
        let result = s.call("check", asked, &CallContext::default());
        assert_eq!(result.unported(), [ooxml_tools_word::CHECK_NO_LAYOUT.to_owned()]);
    }
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
fn styled_text_is_written_and_values_by_reference_left_to_the_host() {
    let s = session();
    let d = s
        .open(docx(&paragraph("22222222", "two word there")), DocKind::Docx, "a.docx")
        .unwrap();
    let context = as_caller("w1");
    // what is not ported yet changes nothing and says so to the host
    let by_reference = s.call(
        "word_set_text",
        json!({"doc": "d1", "target": "p:22222222", "text": "a {{d:1/#1/name}} word"}),
        &context,
    );
    assert!(!by_reference.unported().is_empty());
    assert!(!body_of(&s, &d).contains("name"));
    // styled text is written, its runs listed
    let styled = call(
        &s,
        "word_set_text",
        json!({"doc": "d1", "target": "p:22222222", "text": "a **strong** word"}),
        &context,
    );
    assert!(
        styled["summary"]
            .as_str()
            .is_some_and(|t| t.ends_with("1 run(s) styled")),
        "{styled}"
    );
    assert!(body_of(&s, &d).contains("strong"));
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

#[test]
fn a_version_names_the_parts_it_changed_since_another() {
    let s = session();
    let d = s
        .open(docx(&paragraph("11111111", "one word")), DocKind::Docx, "a.docx")
        .unwrap();
    let context = as_caller("w1");
    let before = s.snapshot(&d).unwrap();
    assert!(before.changed_since(&before).unwrap().is_empty());
    let set = call(
        &s,
        "word_set_text",
        json!({"doc": "d1", "target": "p:11111111", "text": "two words"}),
        &context,
    );
    assert_eq!(set["ok"], json!(true), "{set}");
    let after = s.snapshot(&d).unwrap();
    let changed = after.changed_since(&before).unwrap();
    let names: Vec<&str> = changed.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["word/document.xml"]);
    let body = changed[0].1.as_deref().map(String::from_utf8_lossy).unwrap_or_default();
    assert!(body.contains("two words"), "{body}");
    // the other way round: the earlier version's bytes
    let back = before.changed_since(&after).unwrap();
    assert!(back[0]
        .1
        .as_deref()
        .map(String::from_utf8_lossy)
        .unwrap_or_default()
        .contains("one word"));
}

#[test]
fn retired_ids_say_who_removed_them_and_an_undone_deletion_is_back() {
    let s = session();
    let body = format!(
        "{}{}{}",
        paragraph("11111111", "one"),
        paragraph("22222222", "two"),
        paragraph("33333333", "three")
    );
    let d = s.open(docx(&body), DocKind::Docx, "a.docx").unwrap();
    let w1 = as_caller("w1");
    let deleted = call(&s, "word_delete", json!({"doc": "d1", "target": "p:22222222"}), &w1);
    assert!(deleted.get("error").is_none(), "{deleted}");
    let retired = s.retired(&d).unwrap();
    let [(HeldId::Block(0x2222_2222), RetiredBy::Step { seq, caller, tool })] = retired.as_slice() else {
        panic!("{retired:?}");
    };
    assert_eq!((caller.as_deref(), tool.as_deref()), (Some("w1"), Some("word_delete")));
    assert_eq!(Some(*seq), deleted["version"].as_u64());
    // the bridge's update removes another, as its tool
    s.bridge(
        &d,
        "word_move",
        vec![(
            "word/document.xml".to_owned(),
            Some(Arc::from(document(&paragraph("11111111", "one")).as_bytes())),
        )],
        &as_caller("w2"),
    )
    .unwrap();
    let retired = s.retired(&d).unwrap();
    assert_eq!(retired.len(), 2, "{retired:?}");
    assert!(
        matches!(&retired[1], (HeldId::Block(0x3333_3333), RetiredBy::Step { tool: Some(t), .. }) if &**t == "word_move")
    );
    // undone, both are back; redone, the first is removed again by the call that removed it
    assert_eq!(s.undo(&d, 2, &CallContext::default()).unwrap().count, 2);
    assert!(s.retired(&d).unwrap().is_empty());
    assert_eq!(s.redo(&d, 1, &CallContext::default()).unwrap().count, 1);
    let retired = s.retired(&d).unwrap();
    assert!(
        matches!(retired.as_slice(), [(HeldId::Block(0x2222_2222), RetiredBy::Step { tool: Some(t), .. })] if &**t == "word_delete"),
        "{retired:?}"
    );
    // a state's earlier deletions are given by the host and kept as given
    s.seed_retired(&d, vec![(HeldId::Drawing(4), 3, Some("word_delete (agent=a)".into()))])
        .unwrap();
    assert_eq!(s.retired(&d).unwrap().len(), 2);
}
