//! A session end to end on made-up packages: open, describe through dispatch, the
//! bridge's updates (snapshots unchanged by later commits, parallel writers equal to a
//! serial count), save, close, refusals with their way out, cancellation and deadlines.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::context::{FaceData, FaceRequest, Timestamp};
use ooxml_base::{BaseError, CacheBudget, Cancel, Clock, Context, FontSource, Limits, Locale, PartStore};
use ooxml_session::{CallContext, Caller, CancelToken, DocKind, Session, SessionConfig, SessionError};
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
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
/// A clock that moves one second each time it is read.
#[derive(Default)]
struct Ticking(AtomicU64);
impl Clock for Ticking {
    fn now(&self) -> Timestamp {
        Timestamp::default()
    }
    fn monotonic(&self) -> Duration {
        Duration::from_secs(self.0.fetch_add(1, Ordering::SeqCst))
    }
}

fn session() -> Session {
    let context = Context {
        fonts: Arc::new(NoFonts),
        parts: Arc::new(NoParts),
        clock: Arc::new(Ticking::default()),
        budget: Arc::new(Unlimited),
        cancel: Arc::new(Never),
        limits: Limits::default(),
        locale: Locale::default(),
    };
    Session::new(SessionConfig::default(), context)
}

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

fn docx() -> Vec<u8> {
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!("<w:document {W}><w:body><w:sdt/></w:body></w:document>");
    let mut w = Writer::new();
    for (name, data) in [("_rels/.rels", rels.to_owned()), ("word/document.xml", doc)] {
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
    w.finish(b"").unwrap()
}

fn call(session: &Session, name: &str, arguments: Value) -> Value {
    let result = session.call(name, arguments, &CallContext::default());
    Value::Object(result.to_json())
}

fn put(part: &str, xml: &str) -> (String, Option<Arc<[u8]>>) {
    (part.to_owned(), Some(Arc::from(xml.as_bytes())))
}

#[test]
fn describe_through_the_session() {
    let s = session();
    let d = s.open(Arc::from(docx()), DocKind::Docx, "a.docx").unwrap();
    assert_eq!(d.to_string(), "d1");
    let reply = call(&s, "describe", json!({"doc": "d1"}));
    assert_eq!(reply["ok"], json!(true));
    assert_eq!(
        (reply["doc"].clone(), reply["version"].clone()),
        (json!("d1"), json!(0))
    );
    assert_eq!(reply["summary"], json!("Described a.docx"));
    assert_eq!(reply["data"]["controls"], json!(1));
    // arguments as JSON text, as providers send them
    let text = call(&s, "describe", Value::String(r#"{"doc":"d1"}"#.into()));
    assert_eq!(text, reply);
    assert_eq!(s.stats().calls, 2);
}

#[test]
fn read_batch_is_defined_as_today() {
    use ooxml_tools::ToolArgs;
    let golden = include_str!("../../ooxml-tools-word/tests/golden/read_batch.json");
    let export = ooxml_session::ReadBatch::spec().export(Some(&["docx"]));
    assert_eq!(format!("{export}\n"), golden);
}

#[test]
fn read_batch_answers_each_op_as_alone() {
    let s = session();
    s.open(Arc::from(docx()), DocKind::Docx, "a.docx").unwrap();
    let alone = call(&s, "describe", json!({"doc": "d1"}));
    let reply = call(
        &s,
        "read_batch",
        json!({"ops": [
            {"tool": "describe", "arguments": {"doc": "d1"}},
            {"tool": "word_read", "arguments": {"doc": "d1", "range": "p:0000FFFF"}},
            {"tool": "read_batch", "arguments": {"ops": []}}
        ]}),
    );
    assert_eq!(reply["ok"], json!(true));
    let ops = reply["data"]["ops"].as_array().unwrap();
    assert_eq!(ops.len(), 3);
    assert_eq!(ops[0]["op"], json!(0));
    assert_eq!(ops[0]["data"], alone["data"]);
    assert!(ops[0].get("ok").is_none() && ops[0].get("changed").is_none());
    assert_eq!(ops[1]["error"]["code"], json!("not_found"));
    assert_eq!(ops[2]["error"]["field"], json!("tool"));
    assert!(reply["summary"]
        .as_str()
        .unwrap()
        .starts_with("Read 1 of 3 op(s). 2 failed: ops[1] (word_read): "));
    // a batch with nothing in it is refused with the way out
    let empty = call(&s, "read_batch", json!({"ops": []}));
    assert_eq!(empty["ok"], json!(false));
}

#[test]
fn refusals_carry_their_way_out() {
    let s = session();
    let none = call(&s, "describe", json!({"doc": "d1"}));
    assert_eq!(none["error"]["code"], json!("not_found"));
    assert_eq!(none["error"]["try_call"]["tool"], json!("open_document"));
    s.open(Arc::from(docx()), DocKind::Docx, "a.docx").unwrap();
    let missing = call(&s, "describe", json!({"doc": "d7"}));
    assert_eq!(missing["error"]["message"], json!("no open document 'd7'"));
    assert_eq!(missing["error"]["valid_options"], json!(["d1"]));
    let tool = call(&s, "descibe", json!({"doc": "d1"}));
    assert_eq!(tool["error"]["code"], json!("invalid_arguments"));
    assert_eq!(
        tool["error"]["valid_options"],
        json!([
            "describe",
            "find_text",
            "read_batch",
            "replace_text",
            "word_changes",
            "word_comments",
            "word_edit_table",
            "word_fields",
            "word_format",
            "word_format_table",
            "word_inspect",
            "word_lists",
            "word_read",
            "word_set_text",
            "word_set_tracking",
            "word_styles"
        ])
    );
    let extra = call(&s, "describe", json!({"doc": "d1", "x": 1}));
    assert_eq!(extra["error"]["field"], json!("x"));
    let not_json = call(&s, "describe", Value::String("{doc".into()));
    assert_eq!(not_json["error"]["code"], json!("invalid_arguments"));
    // a kind no tool of that name serves: refused, naming the documents it serves
    s.open(Arc::from(docx()), DocKind::Pptx, "deck.pptx").unwrap();
    let deck = call(&s, "describe", json!({"doc": "d2"}));
    assert_eq!(deck["error"]["code"], json!("refused"));
    assert_eq!(deck["error"]["valid_options"], json!(["d1"]));
    assert_eq!(s.stats().failed, 6);
}

#[test]
fn updates_make_versions_and_snapshots_stay() {
    let s = session();
    let d = s.open(Arc::from(docx()), DocKind::Docx, "a.docx").unwrap();
    let before = s.snapshot(&d).unwrap();
    let doc = format!("<w:document {W}><w:body><w:sdt/><w:sdt/></w:body></w:document>");
    let context = CallContext {
        caller: Caller::named("w1"),
        ..CallContext::default()
    };
    let parts = vec![
        put("word/document.xml", &doc),
        ("word/media/x.bin".to_owned(), Some(Arc::from(&b"\x00\x01"[..]))),
    ];
    let updated = s.update(&d, parts, &context).unwrap();
    assert_eq!((updated.version, updated.changed), (1, true));
    assert_eq!(call(&s, "describe", json!({"doc": "d1"}))["data"]["controls"], json!(2));
    // the snapshot taken before still reads version 0
    assert_eq!(before.version(), 0);
    assert_eq!(
        before
            .package()
            .tree("word/document.xml")
            .unwrap()
            .unwrap()
            .root()
            .children()
            .len(),
        1
    );
    let removed = s.update(&d, vec![("word/media/x.bin".into(), None)], &context).unwrap();
    assert_eq!(removed.version, 2);
    // a part that does not parse changes nothing
    let bad = s.update(&d, vec![put("word/document.xml", "<w:document")], &context);
    assert!(matches!(bad, Err(SessionError::Package(_))));
    let gone = s.update(&d, vec![("word/media/x.bin".into(), None)], &context);
    assert!(matches!(gone, Err(SessionError::Package(_))));
    assert_eq!(s.snapshot(&d).unwrap().version(), 2);
    let saved = s.save(&d).unwrap();
    let again = s.open(Arc::from(saved), DocKind::Docx, "b.docx").unwrap();
    assert_eq!(
        call(&s, "describe", json!({"doc": again.to_string()}))["data"]["controls"],
        json!(2)
    );
    assert_eq!(s.store_stats(&d).unwrap().commits, 2);
}

#[test]
fn an_unchanged_document_saves_as_it_was_read() {
    let s = session();
    let bytes = docx();
    let d = s.open(Arc::from(bytes.clone()), DocKind::Docx, "a.docx").unwrap();
    call(&s, "describe", json!({"doc": "d1"}));
    assert_eq!(s.save(&d).unwrap(), bytes);
    s.close(&d).unwrap();
    assert!(matches!(s.close(&d), Err(SessionError::NotOpen { .. })));
    assert_eq!(s.stats().open, 0);
    // ids are never given twice
    assert_eq!(
        s.open(Arc::from(bytes), DocKind::Docx, "a.docx").unwrap().to_string(),
        "d2"
    );
}

#[test]
fn a_cancelled_or_late_call_stops_with_timeout() {
    let s = session();
    let d = s.open(Arc::from(docx()), DocKind::Docx, "a.docx").unwrap();
    let token = Arc::new(CancelToken::new());
    token.cancel();
    let cancelled = CallContext {
        cancel: Some(token),
        ..CallContext::default()
    };
    let reply = Value::Object(s.call("describe", json!({"doc": "d1"}), &cancelled).to_json());
    assert_eq!(reply["error"]["code"], json!("timeout"));
    // the ticking clock passes a one-second deadline at the first check
    let late = CallContext {
        deadline: Some(Duration::from_millis(500)),
        ..CallContext::default()
    };
    let reply = Value::Object(s.call("describe", json!({"doc": "d1"}), &late).to_json());
    assert_eq!(reply["error"]["code"], json!("timeout"));
    let update = s.update(&d, vec![put("word/x.xml", "<x/>")], &late);
    assert!(matches!(update, Err(SessionError::Base(BaseError::Cancelled))));
    assert_eq!(s.snapshot(&d).unwrap().version(), 0);
    assert_eq!(s.stats().cancelled, 3);
}

#[test]
fn parallel_writers_equal_a_serial_count() {
    let s = Arc::new(session());
    let d = s.open(Arc::from(docx()), DocKind::Docx, "a.docx").unwrap();
    let threads = 8;
    let each = 25;
    std::thread::scope(|scope| {
        for t in 0..threads {
            let (s, d) = (Arc::clone(&s), d.clone());
            scope.spawn(move || {
                let context = CallContext {
                    caller: Caller::named(&format!("w{t}")),
                    ..CallContext::default()
                };
                for k in 0..each {
                    // every writer its own part, and one part they all write
                    let own = put(&format!("word/w{t}.xml"), &format!("<n v=\"{k}\"/>"));
                    let shared = put("word/shared.xml", &format!("<n t=\"{t}\" v=\"{k}\"/>"));
                    let parts = if k % 5 == 0 { vec![own, shared] } else { vec![own] };
                    assert!(s.update(&d, parts, &context).unwrap().changed);
                    let reply = s.call("describe", json!({"doc": "d1"}), &CallContext::default());
                    assert!(!reply.is_error());
                }
            });
        }
    });
    let head = s.snapshot(&d).unwrap();
    assert_eq!(head.version(), threads * each);
    for t in 0..threads {
        let tree = head.package().tree(&format!("word/w{t}.xml")).unwrap().unwrap();
        assert_eq!(tree.root().attribute(None, "v"), Some("24"));
    }
    assert_eq!(s.store_stats(&d).unwrap().commits, threads * each);
}

#[test]
fn a_call_reads_the_version_its_context_names() {
    let s = session();
    let d = s.open(Arc::from(docx()), DocKind::Docx, "a.docx").unwrap();
    let context = CallContext::default();
    let two = format!("<w:document {W}><w:body><w:sdt/><w:sdt/></w:body></w:document>");
    let (updated, first) = s.commit(&d, vec![put("word/document.xml", &two)], &context).unwrap();
    assert_eq!((updated.version, first.version()), (1, 1));
    let three = format!("<w:document {W}><w:body><w:sdt/><w:sdt/><w:sdt/></w:body></w:document>");
    s.update(&d, vec![put("word/document.xml", &three)], &context).unwrap();
    let at = CallContext {
        at: vec![first.clone()],
        ..CallContext::default()
    };
    let read = Value::Object(s.call("describe", json!({"doc": "d1"}), &at).to_json());
    assert_eq!(
        (read["version"].clone(), read["data"]["controls"].clone()),
        (json!(1), json!(2))
    );
    assert_eq!(call(&s, "describe", json!({"doc": "d1"}))["data"]["controls"], json!(3));
    let batch = json!({"ops": [{"tool": "describe", "arguments": {"doc": "d1"}}]});
    let read = Value::Object(s.call("read_batch", batch, &at).to_json());
    assert_eq!(read["data"]["ops"][0]["data"]["controls"], json!(2));
    // nothing to commit: the version it ran on, held
    let (same, held) = s.commit(&d, Vec::new(), &context).unwrap();
    assert_eq!((same.changed, held.version()), (false, 2));
}

struct OnePage;
impl ooxml_tools_word::Placer for OnePage {
    fn place(&self, asked: &[Vec<String>]) -> Option<Vec<serde_json::Map<String, Value>>> {
        Some(
            asked
                .iter()
                .map(|_| json!({"placements": [{"page": 1}]}).as_object().cloned().unwrap())
                .collect(),
        )
    }
}

#[test]
fn inspect_with_layout_asks_the_context_s_placer() {
    let s = session();
    let doc = format!(
        r#"<w:document {W} xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:body><w:p w14:paraId="00000001"><w:r><w:t>a</w:t></w:r></w:p></w:body></w:document>"#
    );
    let d = s.open(Arc::from(docx()), DocKind::Docx, "a.docx").unwrap();
    s.update(&d, vec![put("word/document.xml", &doc)], &CallContext::default())
        .unwrap();
    let arguments = json!({"doc": "d1", "target": "body", "layout": true});
    let refused = call(&s, "word_inspect", arguments.clone());
    assert_eq!(refused["error"]["code"], json!("refused"));
    let placed = CallContext {
        placer: Some(Arc::new(OnePage)),
        ..CallContext::default()
    };
    let read = Value::Object(s.call("word_inspect", arguments, &placed).to_json());
    let block = &read["data"]["blocks"][0];
    assert_eq!(block["placements"], json!([{"page": 1}]));
    let keys: Vec<&String> = block.as_object().unwrap().keys().collect();
    assert_eq!(keys.last().map(|k| k.as_str()), Some("placements"));
}

#[test]
fn a_part_parsed_again_shares_what_did_not_change() {
    let s = session();
    let d = s.open(Arc::from(docx()), DocKind::Docx, "a.docx").unwrap();
    let context = CallContext::default();
    let body = |middle: &str| {
        format!("<w:document {W}><w:body><w:p><w:r><w:t>one</w:t></w:r></w:p>{middle}<w:p><w:r><w:t>three</w:t></w:r></w:p></w:body></w:document>")
    };
    let (_, first) = s
        .commit(
            &d,
            vec![put("word/document.xml", &body("<w:p><w:r><w:t>two</w:t></w:r></w:p>"))],
            &context,
        )
        .unwrap();
    let (_, second) = s
        .commit(
            &d,
            vec![put("word/document.xml", &body("<w:p><w:r><w:t>2</w:t></w:r></w:p>"))],
            &context,
        )
        .unwrap();
    let blocks = |o: &ooxml_session::Opened| -> Vec<Arc<ooxml_xml::Element>> {
        let tree = o.package().tree("word/document.xml").unwrap().unwrap();
        let body = tree.root().child_elements().next().cloned().unwrap();
        body.child_elements().cloned().collect()
    };
    let (a, b) = (blocks(&first), blocks(&second));
    assert!(Arc::ptr_eq(&a[0], &b[0]) && Arc::ptr_eq(&a[2], &b[2]));
    assert!(!Arc::ptr_eq(&a[1], &b[1]));
    assert_eq!(b[1].text_content(), "2");
    let read = Value::Object(
        s.call(
            "word_read",
            json!({"doc": "d1"}),
            &CallContext {
                at: vec![second],
                ..CallContext::default()
            },
        )
        .to_json(),
    );
    assert!(read["data"].as_str().unwrap().contains("\n2\n"));
}

#[test]
fn a_read_says_what_it_does_not_give_as_docx_agent_does() {
    let s = session();
    let d = s.open(Arc::from(docx()), DocKind::Docx, "a.docx").unwrap();
    let chart = r#"<w:p><w:r><w:drawing><wp:inline><wp:docPr id="1" name="Chart 1"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"/></a:graphic></wp:inline></w:drawing></w:r></w:p>"#;
    let doc = format!(
        r#"<w:document {W} xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><w:body><w:p><w:r><w:t>a</w:t></w:r></w:p>{chart}</w:body></w:document>"#
    );
    s.update(&d, vec![put("word/document.xml", &doc)], &CallContext::default())
        .unwrap();
    let read = s.call("word_read", json!({"doc": "d1"}), &CallContext::default());
    assert_eq!(read.unported(), ["chart or SmartArt summary"]);
    let first = s.call(
        "word_read",
        json!({"doc": "d1", "range": "p@body/0"}),
        &CallContext::default(),
    );
    assert!(first.unported().is_empty());
}

/// A body of `n` numbered paragraphs (one list), the `k`th with `text`.
fn numbered(n: usize, edited: Option<(usize, &str)>) -> String {
    let mut body = String::new();
    for k in 0..n {
        let text = match edited {
            Some((at, t)) if at == k => t.to_owned(),
            _ => format!("item {k}"),
        };
        body.push_str(&format!(
            r#"<w:p w14:paraId="{:08X}"><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#,
            k + 1
        ));
    }
    format!(
        r#"<w:document {W} xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:body>{body}</w:body></w:document>"#
    )
}

#[test]
fn a_window_read_counts_lists_from_its_marks_across_versions() {
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/></Relationships>"#;
    let numbering = format!(
        r#"<w:numbering {W}><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
    );
    let s = session();
    let d = s.open(Arc::from(docx()), DocKind::Docx, "a.docx").unwrap();
    let context = CallContext::default();
    let first = vec![
        put("word/_rels/document.xml.rels", rels),
        put("word/numbering.xml", &numbering),
        put("word/document.xml", &numbered(300, None)),
    ];
    let (_, v1) = s.commit(&d, first, &context).unwrap();
    let span = |at: &ooxml_session::Opened, a: usize, b: usize| -> String {
        let context = CallContext {
            at: vec![at.clone()],
            ..CallContext::default()
        };
        let range = format!("p:{:08X}..p:{:08X}", a + 1, b + 1);
        let read = s.call("word_read", json!({"doc": "d1", "range": range}), &context);
        Value::Object(read.to_json())["data"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    // marks made along the way, then read from
    assert!(span(&v1, 250, 251).contains("251. item 250"));
    assert!(span(&v1, 200, 200).contains("201. item 200"));
    // an edit at 150: the marks before it kept, those after it counted again
    let (_, v2) = s
        .commit(
            &d,
            vec![put("word/document.xml", &numbered(300, Some((150, "changed"))))],
            &context,
        )
        .unwrap();
    assert!(span(&v2, 260, 260).contains("261. item 260"));
    assert!(span(&v2, 100, 100).contains("101. item 100"));
    assert!(span(&v2, 150, 150).contains("151. changed"));
    // a fresh session reads the same
    let fresh = session();
    let d2 = fresh.open(Arc::from(docx()), DocKind::Docx, "a.docx").unwrap();
    let all = vec![
        put("word/_rels/document.xml.rels", rels),
        put("word/numbering.xml", &numbering),
        put("word/document.xml", &numbered(300, Some((150, "changed")))),
    ];
    fresh.update(&d2, all, &context).unwrap();
    let whole = Value::Object(
        fresh
            .call("word_read", json!({"doc": "d1", "range": "p:00000105"}), &context)
            .to_json(),
    );
    assert_eq!(whole["data"], json!(span(&v2, 260, 260)));
}
