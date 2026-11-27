//! A session owning Word documents (phase P2, lane P2c) on made-up packages: the editing
//! tools run on the document's editor with the tracking mode the session keeps; the
//! session's undo and redo, and a caller's own; the save gate; an update of parts given as bytes adding
//! the ids it brings to the ledger.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, NoFonts, NoParts, Unlimited};
use ooxml_base::context::Timestamp;
use ooxml_base::{Clock, Context, Limits, Locale};
use ooxml_session::{CallContext, Caller, DocId, DocKind, Session, SessionConfig, SessionError, Undone};
use ooxml_session_word::prelude::*;
use ooxml_session_word::{HeldId, RetiredBy};
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

/// The ports of the made-up documents' sessions: no fonts, no stored parts, a fixed clock.
pub(crate) fn context() -> Context {
    Context {
        fonts: Arc::new(NoFonts),
        parts: Arc::new(NoParts),
        clock: Arc::new(Fixed),
        budget: Arc::new(Unlimited),
        cancel: Arc::new(Never),
        limits: Limits::default(),
        locale: Locale::default(),
    }
}

pub(crate) fn session() -> Session {
    Session::new(SessionConfig::default(), context(), ooxml_session_word::formats(None))
}

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#;

fn paragraph(id: &str, text: &str) -> String {
    format!(r#"<w:p w14:paraId="{id}" w14:textId="77777777"><w:r><w:t>{text}</w:t></w:r></w:p>"#)
}

/// A body whose bookmark has no end (a problem of its own; a paragraph id given twice is
/// stamped as the part is read).
fn unpaired() -> String {
    format!(
        r#"{}<w:p w14:paraId="22222222" w14:textId="77777777"><w:bookmarkStart w:id="5" w:name="open"/><w:r><w:t>two</w:t></w:r></w:p>"#,
        paragraph("11111111", "one")
    )
}

fn document(body: &str) -> String {
    format!("<w:document {NS}><w:body>{body}</w:body></w:document>")
}

pub(crate) fn docx(body: &str) -> Arc<[u8]> {
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

pub(crate) fn as_caller(name: &str) -> CallContext {
    CallContext {
        caller: Caller::named(name),
        ..CallContext::default()
    }
}

pub(crate) fn call(session: &Session, name: &str, arguments: Value, context: &CallContext) -> Value {
    Value::Object(session.call(name, arguments, context).to_json())
}

fn put(session: &Session, doc: &DocId, body: &str, context: &CallContext) -> u64 {
    let parts = vec![(
        "word/document.xml".to_owned(),
        Some(Arc::from(document(body).as_bytes())),
    )];
    session.update(doc, parts, context).unwrap().version
}

pub(crate) fn body_of(session: &Session, doc: &DocId) -> String {
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
    // the mode is the session's: describe says it
    let described = call(&s, "describe", json!({"doc": "d1"}), &context);
    assert_eq!(described["data"]["tracking"], json!({"author": "Bo"}), "{described}");
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
    // the session's undo takes the accept back: the state opened, numbered 0 again
    assert_eq!(
        s.undo(&d, 1, &CallContext::default()).unwrap(),
        Undone { count: 1, version: 0 }
    );
    assert!(body_of(&s, &d).contains("w:ins"));
    let off = call(&s, "word_set_tracking", json!({"doc": "d1", "on": false}), &context);
    assert_eq!(off["data"]["tracking"], json!(false), "{off}");
    let described = call(&s, "describe", json!({"doc": "d1"}), &context);
    assert_eq!(described["data"]["tracking"], Value::Null, "{described}");
}

#[test]
fn edits_beside_tracking_switches_are_written_under_the_mode_they_commit_under() {
    // R2 #6: the mode is read for an edit's whole call, so a switch never falls between an
    // edit's prepare and its commit: every edit is either a tracked change by the author on
    // or a plain edit, and each switch's reply sees the edits before it committed
    let s = session();
    s.open(docx(&paragraph("11111111", "one")), DocKind::Docx, "a.docx")
        .unwrap();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for k in 0..20 {
                let on = k % 2 == 0;
                let given = if on {
                    json!({"doc": "d1", "on": true, "author": "Bo"})
                } else {
                    json!({"doc": "d1", "on": false})
                };
                let set = call(&s, "word_set_tracking", given, &as_caller("switch"));
                assert_eq!(set["ok"], json!(true), "{set}");
            }
        });
        for w in 0..2 {
            let s = &s;
            scope.spawn(move || {
                for k in 0..10 {
                    let text = format!("writer {w} edit {k}");
                    let edited = call(
                        s,
                        "word_set_text",
                        json!({"doc": "d1", "target": "p:11111111", "text": text}),
                        &as_caller(&format!("w{w}")),
                    );
                    assert_eq!(edited["ok"], json!(true), "{edited}");
                }
            });
        }
    });
    // the last switch turned tracking off
    let described = call(&s, "describe", json!({"doc": "d1"}), &as_caller("r"));
    assert_eq!(described["data"]["tracking"], Value::Null, "{described}");
}

#[test]
fn change_listings_read_the_pinned_version_and_count_this_sessions_records() {
    let s = session();
    let tracked = format!(
        r#"{}<w:p w14:paraId="22222222" w14:textId="77777777"><w:ins w:id="1" w:author="Ann" w:date="2026-01-01T00:00:00Z"><w:r><w:t>new</w:t></w:r></w:ins></w:p>"#,
        paragraph("11111111", "one")
    );
    let d = s.open(docx(&tracked), DocKind::Docx, "a.docx").unwrap();
    let bo = as_caller("w1");
    call(
        &s,
        "word_set_tracking",
        json!({"doc": "d1", "on": true, "author": "Bo"}),
        &bo,
    );
    let edited = call(
        &s,
        "word_set_text",
        json!({"doc": "d1", "target": "p:11111111", "text": "one more"}),
        &bo,
    );
    assert_eq!(edited["ok"], json!(true), "{edited}");
    let pinned = s.snapshot(&d).unwrap();
    // a later accept of everything: the head has no change left, the pinned version has
    call(
        &s,
        "word_changes",
        json!({"doc": "d1", "action": "accept", "all": true}),
        &bo,
    );
    let at = CallContext {
        at: vec![pinned],
        ..CallContext::default()
    };
    let listed = call(&s, "word_changes", json!({"doc": "d1", "action": "list"}), &at);
    let authors: Vec<&Value> = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| &c["author"])
        .collect();
    assert!(
        authors.contains(&&json!("Ann")) && authors.contains(&&json!("Bo")),
        "{listed}"
    );
    // what accepting all would settle: Ann's record was there when the document was opened,
    // Bo's were written by this session
    let preview = call(
        &s,
        "word_changes",
        json!({"doc": "d1", "action": "preview", "all": true}),
        &at,
    );
    let accept = &preview["data"]["accept"];
    let records = accept["records"].as_u64().unwrap();
    assert!(records >= 2, "{preview}");
    assert_eq!(accept["this_session"].as_u64(), Some(records - 1), "{preview}");
    assert_eq!(accept["changes_left"], json!(0));
    let head = call(&s, "word_changes", json!({"doc": "d1", "action": "list"}), &bo);
    assert_eq!(head["data"], json!([]), "{head}");
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
            assert_eq!(later, vec![2]);
        }
        other => panic!("{other:?}"),
    }
    // bo's is the latest: undone; then ann's is, and redo brings back only one's own
    assert_eq!(s.undo(&d, 5, &bo).unwrap().count, 1);
    assert_eq!(s.undo(&d, 1, &ann).unwrap().count, 1);
    assert!(body_of(&s, &d).contains(">one<"));
    // ann's undo is the next to redo: bo's own redo is entangled, until ann's is redone
    assert!(matches!(s.redo(&d, 1, &bo), Err(SessionError::RedoNotNext { step: 2 })));
    assert_eq!(s.redo(&d, 1, &ann).unwrap().count, 1);
    assert!(body_of(&s, &d).contains("by ann"));
    assert_eq!(s.redo(&d, 1, &CallContext::default()).unwrap().count, 1);
    assert!(body_of(&s, &d).contains("by bo"));
    assert_eq!(s.undo(&d, 0, &CallContext::default()).unwrap().count, 0);
}

/// A caller whose context the host gave in its own order (`{"turn": .., "agent": ..}`).
fn as_writer(agent: &str) -> CallContext {
    let given = json!({"turn": 1, "agent": agent});
    let key = format!(r#"{{"agent":"{agent}","turn":1}}"#);
    CallContext {
        caller: Caller::named(&key),
        caller_context: given.as_object().cloned().map(Arc::new),
        ..CallContext::default()
    }
}

/// The paragraph `p:11111111`'s text set to `text` by `context`'s caller (one step).
fn written(s: &Session, text: &str, context: &CallContext) {
    let arguments = json!({"doc": "d1", "target": "p:11111111", "text": text});
    let done = call(s, "word_set_text", arguments, context);
    assert_eq!(done["ok"], json!(true), "{done}");
}

#[test]
fn the_undo_tool_answers_in_todays_words() {
    let s = session();
    let d = s
        .open(docx(&paragraph("11111111", "one")), DocKind::Docx, "a.docx")
        .unwrap();
    let (a, b) = (as_writer("a"), as_writer("b"));
    let undo = |arguments: Value, context: &CallContext| call(&s, "undo", arguments, context);
    let nothing = undo(json!({"doc": "d1"}), &a);
    assert_eq!(
        nothing["error"],
        json!({"code": "refused", "valid_options": [{"version": 0, "oldest": true}],
               "message": "nothing of yours to undo in d1: undo here takes back your own changes only"})
    );
    assert_eq!(
        (nothing["doc"].clone(), nothing["version"].clone()),
        (json!("d1"), json!(0))
    );
    written(&s, "by a", &a);
    written(&s, "by b", &b);
    let refused = undo(json!({"doc": "d1"}), &a);
    assert_eq!(refused["version"], json!(2));
    assert_eq!(refused["error"]["code"], json!("entangled"));
    assert_eq!(
        refused["error"]["message"],
        json!(
            "your latest change to d1 (word_set_text, version 1) is not the latest: word_set_text by turn=1, \
             agent=b changed d1 after it (version 2), and undoing yours would take back what came after. \
             Change it back with an edit, or ask whoever made the later change to undo it first"
        )
    );
    assert_eq!(
        refused["error"]["details"],
        json!({"version": 1, "later": [{"version": 2, "tool": "word_set_text", "context": {"turn": 1, "agent": "b"}}]})
    );
    let undone = undo(json!({"doc": "d1", "steps": 3}), &b);
    assert_eq!(undone["summary"], json!("Undid 1 of your own step(s) of d1"));
    assert_eq!(undone["data"], json!({"steps": 1, "version": 1, "own": true}));
    assert_eq!(undone["version"], json!(1));
    assert_eq!(undone["changed"], json!(["d1"]));
    assert_eq!(
        undone["warnings"],
        json!(["1 of 3 step(s): no more of yours, or the next depends on a later change by another caller"])
    );
    assert_eq!(undo(json!({"doc": "d1"}), &a)["data"]["version"], json!(0));
    let redo = undo(json!({"doc": "d1", "redo": true}), &b);
    assert_eq!(
        redo["error"]["message"],
        json!(
            "your undone change to d1 (version 2) is not the next to redo: another caller undid a change since. \
             Make your change again instead"
        )
    );
    let own = CallContext::default();
    let redone = undo(json!({"doc": "d1", "redo": true, "steps": 5}), &own);
    assert_eq!(redone["summary"], json!("Redid 2 step(s) of d1"));
    assert_eq!(redone["warnings"], json!([]));
    assert!(body_of(&s, &d).contains("by b"));
    // a new step drops every own undo with plain redo
    assert_eq!(undo(json!({"doc": "d1", "steps": 2}), &own)["data"]["steps"], json!(2));
    written(&s, "again", &a);
    let none = undo(json!({"doc": "d1", "redo": true}), &b);
    assert_eq!(none["error"]["code"], json!("refused"));
    assert_eq!(none["error"]["valid_options"][0]["context"]["agent"], json!("a"));
    assert!(none["error"].get("try_call").is_none(), "{none}");
    assert_eq!(
        undo(json!({"doc": "d1", "steps": 0}), &own)["error"]["message"],
        json!("steps is 0; at least 1")
    );
    let within = call(
        &s,
        "read_batch",
        json!({"ops": [{"tool": "undo", "arguments": {"doc": "d1"}}]}),
        &own,
    );
    assert!(
        within.to_string().contains("undo cannot run inside a read_batch"),
        "{within}"
    );
}

#[test]
fn the_save_gate_refuses_new_problems_and_parts_given_feed_the_ledger() {
    let s = session();
    let d = s
        .open(docx(&paragraph("11111111", "one")), DocKind::Docx, "a.docx")
        .unwrap();
    assert!(s.validation(&d).unwrap().unwrap().new.is_empty());
    s.save_checked(&d).unwrap();
    // parts given with a bookmark without its end: a new problem, and saving is refused
    put(&s, &d, &unpaired(), &CallContext::default());
    match s.save_checked(&d) {
        Err(SessionError::NewProblems { new }) => assert!(!new.is_empty()),
        other => panic!("{other:?}"),
    }
    // ids parts given bring are in the ledger, and stay there after undo
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
    // a bookmark without its end: a new problem, against the problems the document opened
    // with (a repeated paragraph id given is stamped as it is read: no problem)
    put(&s, &d, &unpaired(), &CallContext::default());
    let validate = &call(&s, "check", asked, &CallContext::default())["data"]["validate"];
    assert_eq!(validate["new"].as_array().unwrap().len(), 1);
    assert!(validate["new"][0].as_str().unwrap().starts_with("[bookmark-unpaired]"));
    assert_eq!(validate["problems"], validate["new"]);
    // what needs the layout's facts is the host's where it hands no layout; a reflow too
    for asked in [
        json!({"doc": "d1", "include": ["validate", "layout"]}),
        json!({"doc": "d1", "include": ["validate"], "range": "p:11111111"}),
    ] {
        let result = s.call("check", asked, &CallContext::default());
        assert_eq!(result.not_supported_form(), Some(ooxml_tools_word::CHECK_NO_LAYOUT));
    }
    for asked in [
        json!({"doc": "d1"}),
        json!({"doc": "d1", "include": ["validate", "reflow"]}),
    ] {
        let result = s.call("check", asked, &CallContext::default());
        assert_eq!(result.not_supported_form(), Some(ooxml_tools_word::CHECK_NO_LAYOUT));
    }
}

#[test]
fn styled_text_is_written_and_values_by_reference_left_to_the_host() {
    let s = session();
    let d = s
        .open(docx(&paragraph("22222222", "two word there")), DocKind::Docx, "a.docx")
        .unwrap();
    let context = as_caller("w1");
    // a target by reference ("$name") is the host's to resolve: nothing changes, and the
    // host is told so
    let by_reference = s.call(
        "word_set_text",
        json!({"doc": "d1", "target": "$picked", "text": "a picked word"}),
        &context,
    );
    assert!(by_reference.not_supported_form().is_some());
    assert!(!body_of(&s, &d).contains("picked"));
    // braces reaching the tool are literal text (the host resolves real references first)
    let braces = call(
        &s,
        "word_set_text",
        json!({"doc": "d1", "target": "p:22222222", "text": "a {{d:1/#1/name}} word"}),
        &context,
    );
    assert_eq!(braces["ok"], json!(true), "{braces}");
    assert!(body_of(&s, &d).contains("a {{d:1/#1/name}} word"));
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
    assert!(before.parts_changed_since(&before).unwrap().is_empty());
    let set = call(
        &s,
        "word_set_text",
        json!({"doc": "d1", "target": "p:11111111", "text": "two words"}),
        &context,
    );
    assert_eq!(set["ok"], json!(true), "{set}");
    let after = s.snapshot(&d).unwrap();
    assert_eq!(after.parts_changed_since(&before).unwrap(), ["word/document.xml"]);
    // the other way round: the same part
    assert_eq!(before.parts_changed_since(&after).unwrap(), ["word/document.xml"]);
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
    // another caller's deletion removes another, as its call
    let again = call(
        &s,
        "word_delete",
        json!({"doc": "d1", "target": "p:33333333"}),
        &as_caller("w2"),
    );
    assert!(again.get("error").is_none(), "{again}");
    let retired = s.retired(&d).unwrap();
    assert_eq!(retired.len(), 2, "{retired:?}");
    assert!(
        matches!(&retired[1], (HeldId::Block(0x3333_3333), RetiredBy::Step { caller: Some(c), .. }) if &**c == "w2")
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
}

fn part_of(session: &Session, doc: &DocId, name: &str) -> Option<String> {
    let saved = session.save(doc).unwrap();
    let package = ooxml_opc::Package::open(Arc::from(saved), &Limits::default()).unwrap();
    package
        .read(name)
        .unwrap()
        .map(|data| String::from_utf8(data.to_vec()).unwrap())
}

#[test]
fn a_value_written_by_reference_is_kept_on_its_paragraph() {
    let s = session();
    let d = s
        .open(docx(&paragraph("22222222", "two word there")), DocKind::Docx, "a.docx")
        .unwrap();
    // the host wrote "3.5" for {{d:scores/Alpha/rating}} at characters 6..9 of the text
    let record = json!({"source": "d:scores/Alpha/rating", "text": "3.5"});
    let mut context = as_caller("w1");
    let written = ooxml_tools_word::Written::list_from_json(&json!([{
        "field": "text", "start": 6, "end": 9, "text": "3.5", "record": record,
        "brief": {"from": "d:scores/Alpha/rating", "text": "3.5"},
    }]))
    .unwrap();
    context.values.written = written;
    let reply = call(
        &s,
        "word_set_text",
        json!({"doc": "d1", "target": "p:22222222", "text": "Rated 3.5 today"}),
        &context,
    );
    assert_eq!(
        reply["data"]["facts"],
        json!([{"from": "d:scores/Alpha/rating", "text": "3.5"}]),
        "{reply}"
    );
    let item = part_of(&s, &d, "customXml/item1.xml").expect("the item is made");
    assert!(
        item.ends_with(
            r#"<prov:source kind="text" key="p:22222222">{"v":1,"paragraph":"Rated 3.5 today","facts":[{"source":"d:scores/Alpha/rating","text":"3.5","p":0,"at":6}]}</prov:source></prov:sources>"#
        ),
        "{item}"
    );
    let props = part_of(&s, &d, "customXml/itemProps1.xml").unwrap();
    assert!(props.contains("ds:itemID=\"{"), "{props}");
    assert!(part_of(&s, &d, "word/_rels/document.xml.rels").is_some_and(|r| r.contains("customXml/item1.xml")));
    // the value edited by hand: its fact goes, and with the last entry the item
    call(
        &s,
        "word_set_text",
        json!({"doc": "d1", "target": "p:22222222", "text": "Rated 4 today"}),
        &as_caller("w1"),
    );
    assert!(part_of(&s, &d, "customXml/item1.xml").is_none());
    assert!(part_of(&s, &d, "customXml/itemProps1.xml").is_none());
}

/// A read after an edit that stamped the document: an id by place (`p@body/1`, the table
/// `t@body/0` and its cells) names the block it named before, as docx-agent's view follows
/// its renames -- after a table's edit, then a paragraph's.
#[test]
fn a_read_by_the_ids_an_open_gives_reads_the_same_blocks_after_edits() {
    let s = session();
    let cell = |text: &str| format!("<w:tc><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>");
    let body = format!(
        "<w:p><w:r><w:t>one</w:t></w:r></w:p><w:p><w:r><w:t>two</w:t></w:r></w:p><w:tbl><w:tblGrid><w:gridCol/><w:gridCol/></w:tblGrid><w:tr>{}{}</w:tr><w:tr>{}{}</w:tr></w:tbl><w:p/>",
        cell("a"),
        cell("b"),
        cell("c"),
        cell("d")
    );
    let d = s.open(docx(&body), DocKind::Docx, "a.docx").unwrap();
    let context = as_caller("w1");
    // the blocks unstamped in the bytes have their ids from the first read, and keep them
    let ids = paragraph_ids(&s, &d);
    let table = {
        let head = s.snapshot(&d).unwrap();
        let docx = head.word().unwrap().docx().unwrap();
        docx.body().unwrap().ids.tables().iter().next().unwrap().id.clone()
    };
    let (first, second) = (ids[0].clone(), ids[1].clone());
    let cell = format!("{table}/c0,1");
    let edits = [
        json!({"tool": "word_edit_table", "doc": "d1", "table": table, "action": "insert_row", "index": 1}),
        json!({"tool": "word_set_text", "doc": "d1", "target": first, "text": "first"}),
    ];
    for edit in edits {
        let mut arguments = edit.clone();
        let tool = arguments.as_object_mut().unwrap().remove("tool").unwrap();
        let done = call(&s, tool.as_str().unwrap(), arguments, &context);
        assert_eq!(done["ok"], json!(true), "{done}");
        for (target, text) in [(second.as_str(), "two"), (cell.as_str(), "b"), (table.as_str(), "a")] {
            let reads = [
                ("word_inspect", json!({"doc": "d1", "target": target})),
                ("word_read", json!({"doc": "d1", "range": target})),
                ("find_text", json!({"doc": "d1", "text": text, "range": target})),
            ];
            for (read_tool, arguments) in reads {
                let read = call(&s, read_tool, arguments, &context);
                assert_eq!(read["ok"], json!(true), "after {tool}, {read_tool} {target}: {read}");
                assert!(read.to_string().contains(text), "{read_tool} {target}: {read}");
            }
        }
        let span = call(
            &s,
            "word_read",
            json!({"doc": "d1", "range": format!("{second}..{table}")}),
            &context,
        );
        assert!(span.to_string().contains("two"), "after {tool}: {span}");
    }
}

#[test]
fn a_step_group_is_one_undo_step_and_a_revert_takes_its_edits_back() {
    let s = session();
    let d = s
        .open(docx(&paragraph("11111111", "one")), DocKind::Docx, "a.docx")
        .unwrap();
    let context = as_caller("w1");
    put(&s, &d, &paragraph("11111111", "two"), &context);
    // a batch that fails: its edits and its tracking mode taken back, redo as it was
    let start = s.open_group(&d).unwrap();
    let on = call(
        &s,
        "word_set_tracking",
        json!({"doc": "d1", "on": true, "author": "Bo"}),
        &context,
    );
    assert_eq!(on["ok"], json!(true), "{on}");
    put(&s, &d, &paragraph("11111111", "three"), &context);
    let reverted = s.revert_group(&d, &start, &context).unwrap();
    assert_eq!(reverted, Some(s.snapshot(&d).unwrap().version()));
    assert_eq!(s.close_group(&d, &start, &context, Some("batch")).unwrap(), None);
    assert!(body_of(&s, &d).contains(">two<"));
    assert!(!body_of(&s, &d).contains("w:ins"));
    // a batch that applies: one step, which one undo takes back
    let group = s.open_group(&d).unwrap();
    put(&s, &d, &paragraph("11111111", "four"), &context);
    put(&s, &d, &paragraph("11111111", "five"), &context);
    let made = s.close_group(&d, &group, &context, Some("batch")).unwrap();
    assert_eq!(made, Some(s.snapshot(&d).unwrap().version()));
    assert!(body_of(&s, &d).contains(">five<"));
    assert_eq!(s.undo(&d, 1, &context).unwrap().count, 1);
    assert!(body_of(&s, &d).contains(">two<"));
}

#[test]
fn a_batch_is_read_by_its_own_thread_alone_and_another_threads_batch_waits_for_it() {
    let s = Arc::new(session());
    let d = s
        .open(docx(&paragraph("11111111", "one")), DocKind::Docx, "a.docx")
        .unwrap();
    let context = as_caller("w1");
    let group = s.open_group(&d).unwrap();
    assert!(s.open_group(&d).is_err(), "this thread's batch is open already");
    let set = |text: &str| {
        call(
            &s,
            "word_set_text",
            json!({"doc": "d1", "target": "p:11111111", "text": text}),
            &context,
        )
    };
    let first = set("two");
    let second = set("three");
    // each op's version is the step the batch will make, read here alone
    assert_eq!(first["ok"], json!(true), "{first}");
    assert_eq!(
        (first["version"].clone(), second["version"].clone()),
        (json!(1), json!(1))
    );
    assert!(body_of(&s, &d).contains(">three<"));
    let elsewhere = std::thread::scope(|scope| scope.spawn(|| body_of(&s, &d)).join().unwrap());
    assert!(elsewhere.contains(">one<"), "another thread reads the head");
    let waiting = {
        let (s, d) = (Arc::clone(&s), d.clone());
        std::thread::spawn(move || {
            let theirs = s.open_group(&d).unwrap();
            let seen = body_of(&s, &d);
            s.close_group(&d, &theirs, &CallContext::default(), None).unwrap();
            seen
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(!waiting.is_finished(), "another thread's batch waits");
    let made = s.close_group(&d, &group, &context, Some("batch")).unwrap();
    assert_eq!(made, Some(s.snapshot(&d).unwrap().version()));
    assert!(waiting.join().unwrap().contains(">three<"));
    assert_eq!(s.snapshot(&d).unwrap().number(), 1);
    assert_eq!(s.numbers(&d).unwrap().steps.len(), 1, "one step");
    // a batch let go unclosed publishes nothing
    let dropped = s.open_group(&d).unwrap();
    set("four");
    assert!(s.abandon_group(&d, &dropped).unwrap());
    assert!(body_of(&s, &d).contains(">three<"));
    assert!(!s.abandon_group(&d, &dropped).unwrap());
}

#[test]
fn a_package_batch_is_one_held_transaction_too() {
    let s = session();
    let d = s
        .open(docx(&paragraph("11111111", "one")), DocKind::Pptx, "a.pptx")
        .unwrap();
    let context = as_caller("w1");
    let group = s.open_group(&d).unwrap();
    put(&s, &d, &paragraph("11111111", "two"), &context);
    put(&s, &d, &paragraph("11111111", "three"), &context);
    let elsewhere = std::thread::scope(|scope| scope.spawn(|| s.snapshot(&d).unwrap().number()).join().unwrap());
    assert_eq!((s.snapshot(&d).unwrap().number(), elsewhere), (1, 0));
    let made = s.close_group(&d, &group, &context, Some("batch")).unwrap();
    assert_eq!(made, Some(s.snapshot(&d).unwrap().version()));
    assert_eq!(s.numbers(&d).unwrap().steps.len(), 1, "one step");
    let dropped = s.open_group(&d).unwrap();
    put(&s, &d, &paragraph("11111111", "four"), &context);
    assert!(s.abandon_group(&d, &dropped).unwrap());
    assert!(body_of(&s, &d).contains(">three<"));
}

#[test]
fn the_history_is_listed_in_state_numbers() {
    let s = session();
    let d = s
        .open(docx(&paragraph("11111111", "one")), DocKind::Docx, "a.docx")
        .unwrap();
    let (ann, bo) = (as_caller("ann"), as_caller("bo"));
    put(&s, &d, &paragraph("11111111", "by ann"), &ann);
    put(&s, &d, &paragraph("11111111", "by bo"), &bo);
    let numbers = s.numbers(&d).unwrap();
    assert_eq!(numbers.highest, 2);
    let steps: Vec<_> = numbers
        .steps
        .iter()
        .map(|s| (s.before, s.after, s.caller.clone()))
        .collect();
    assert_eq!(
        steps,
        vec![(0, 1, Some("ann".to_owned())), (1, 2, Some("bo".to_owned()))]
    );
    // an undo brings back state 1 (its number again); redo would bring back 2
    let undone = s.undo(&d, 1, &CallContext::default()).unwrap();
    assert_eq!((undone.count, undone.version), (1, 1));
    assert_eq!(s.snapshot(&d).unwrap().number(), 1);
    let numbers = s.numbers(&d).unwrap();
    assert_eq!((numbers.highest, numbers.steps.len()), (2, 1));
    assert_eq!(numbers.redo.last().map(|r| r.after), Some(2));
    assert_eq!(s.highest(&d).unwrap(), 2);
    // a state found by its number: the head, an earlier one, none once let go
    assert_eq!(s.numbered(&d, 1).unwrap().map(|o| o.number()), Some(1));
    assert!(s.numbered(&d, 0).unwrap().is_some_and(|o| o.number() == 0));
    // a new step takes the next number; redo is gone
    put(&s, &d, &paragraph("11111111", "by ann again"), &ann);
    let numbers = s.numbers(&d).unwrap();
    assert_eq!((numbers.highest, numbers.redo.len()), (3, 0));
    assert_eq!(numbers.steps.last().map(|s| (s.before, s.after)), Some((1, 3)));
    assert!(s.numbered(&d, 2).unwrap().is_none(), "state 2 is out of reach");
}

#[test]
fn nothing_to_undo_lists_the_steps_nearest_the_head_and_says_how_many_were_left_out() {
    let s = session();
    s.open(docx(&paragraph("11111111", "one")), DocKind::Docx, "a.docx")
        .unwrap();
    let a = as_writer("a");
    for n in 0..12 {
        written(&s, &format!("step {n}"), &a);
    }
    let refused = call(&s, "undo", json!({"doc": "d1"}), &as_writer("c"));
    let error = &refused["error"];
    assert_eq!(error["code"], json!("refused"), "{refused}");
    let listed = error["valid_options"].as_array().cloned().unwrap_or_default();
    assert_eq!(listed.len(), 10, "{refused}");
    assert_eq!(
        listed[0],
        json!({"version": 12, "tool": "word_set_text", "context": {"turn": 1, "agent": "a"}})
    );
    assert_eq!(error["details"], json!({"left_out": {"undo": 2, "redo": 0}}));
    assert!(error.get("try_call").is_none(), "{refused}");
    // a's own redo with nothing to redo: a's undo works
    let redo = call(&s, "undo", json!({"doc": "d1", "redo": true}), &a);
    assert_eq!(
        redo["error"]["try_call"],
        json!({"tool": "undo", "arguments": {"doc": "d1"}})
    );
    assert_eq!(redo["error"]["valid_options"][0]["yours"], json!(true));
}

/// Every paragraph's id the session reads, in order.
fn paragraph_ids(session: &Session, doc: &DocId) -> Vec<String> {
    let head = session.snapshot(doc).unwrap();
    let docx = head.word().unwrap().docx().unwrap();
    docx.body().unwrap().ids.paragraphs().iter().map(|p| p.id).collect()
}

#[test]
fn parts_given_without_ids_are_read_stamped_and_keep_the_ids_of_what_they_kept() {
    // the stamp-on-open design's refresh rule for a host's commit of parts: a story part
    // whose blocks lack ids is stamped in memory (saved as given until the next edit), a
    // block the part held the same before keeping its ids, a command never meeting a block
    // without one
    let unstamped = |text: &str| format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>");
    let s = session();
    let first = format!("{}{}{}", unstamped("one"), unstamped("two"), unstamped("three"));
    let d = s.open(docx(&first), DocKind::Docx, "a.docx").unwrap();
    let opened = paragraph_ids(&s, &d);
    assert!(opened.iter().all(|id| id.starts_with("p:")), "{opened:?}");
    // given again without ids: "two" changed, a paragraph added before it
    let given = format!(
        "{}{}{}{}",
        unstamped("one"),
        unstamped("new"),
        unstamped("TWO"),
        unstamped("three")
    );
    put(&s, &d, &given, &CallContext::default());
    let now = paragraph_ids(&s, &d);
    assert!(now.iter().all(|id| id.starts_with("p:")), "{now:?}");
    assert_eq!(
        (&now[0], &now[3]),
        (&opened[0], &opened[2]),
        "kept: {opened:?} -> {now:?}"
    );
    assert!(
        !opened.contains(&now[1]) && !opened.contains(&now[2]),
        "{opened:?} -> {now:?}"
    );
    // saved as given, until an edit writes it
    let saved = body_of(&s, &d);
    assert!(saved.contains("<w:t>new</w:t>") && !saved.contains("paraId"), "{saved}");
    let done = call(
        &s,
        "word_set_text",
        json!({"doc": "d1", "target": now[3], "text": "Three"}),
        &CallContext::default(),
    );
    assert_eq!(done["changed"], json!([now[3]]), "{done}");
    assert_eq!(paragraph_ids(&s, &d), now);
    assert!(body_of(&s, &d).contains(&format!("w14:paraId=\"{}\"", &now[1][2..])));
}
