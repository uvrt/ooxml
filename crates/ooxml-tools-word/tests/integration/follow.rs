//! `word_follow_sample` on the made-up document of `compare.rs` (`golden/compare_body.xml`):
//! a table following the other, untracked and tracked; a paragraph and a table following a
//! paragraph of the document opened again (`d2`), the copies' own left out; a preview with
//! a cell address and the sample among its copies. The replies and the document after each
//! are docx-agent's (`golden/follow_replies.jsonl`, written by docx-agent's tool on the
//! same document). Its equality with docx-agent on the corpus is
//! `word_edits_diff.py --style`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, Still, Unlimited, R, W};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, StoreConfig};
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{word_follow_sample, CallEditor, Editing, Others, Version, WordFollowSample};
use ooxml_word_edit::{Editor, Tracking};
use ooxml_word_model::docx::Versions;
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::{json, Value};
use std::sync::Arc;

fn package() -> Package {
    let body = include_str!("../golden/compare_body.xml").trim();
    let parts = [
        ("[Content_Types].xml".to_owned(), r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#.to_owned()),
        ("_rels/.rels".to_owned(), format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}officeDocument" Target="word/document.xml"/></Relationships>"#)),
        ("word/_rels/document.xml.rels".to_owned(), format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}styles" Target="styles.xml"/></Relationships>"#)),
        ("word/styles.xml".to_owned(), format!(r#"<w:styles {W}><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style></w:styles>"#)),
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

/// The tool's reply (summary, data, changed, warnings) and the document part after it, on
/// a fresh copy of the document, `d2` the document opened again.
fn follow(mut arguments: Value) -> Value {
    let tracked = arguments.as_object_mut().unwrap().remove("tracked");
    let tracking = tracked.map(|author| Tracking {
        author: author.as_str().unwrap().to_owned(),
        date: "2026-11-17T10:00:00Z".to_owned(),
    });
    let editor = Editor::open(package(), StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let opened = editor.store().snapshot().doc().clone();
    let caller = Caller::anonymous();
    let copy = package();
    let versions = Versions::new();
    let copy = crate::common::read(&copy);
    let other = Version::new("b.docx", &copy, 1, &versions);
    let others = [("d2", other)];
    let call_editor = CallEditor::new(&editor);
    let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-17T10:00:00Z")
        .with_tracking(tracking.as_ref())
        .with_others(Others::new(&others));
    let (args, _) = WordFollowSample::accept(arguments, Some("docx")).unwrap();
    let reply = Value::Object(word_follow_sample(&args, &editing).to_json());
    let head = editor.store().snapshot();
    // the part as saved (a version no edit made is the bytes it was opened from)
    let saved = Package::open(Arc::from(head.doc().package().save().unwrap()), &Limits::default()).unwrap();
    let document = saved.read("word/document.xml").unwrap().unwrap();
    let keys = ["summary", "data", "changed", "warnings"];
    json!({
        "reply": keys.iter().map(|k| ((*k).to_owned(), reply.get(*k).cloned().unwrap_or(Value::Null))).collect::<serde_json::Map<_, _>>(),
        "document": String::from_utf8_lossy(&document),
    })
}

#[test]
fn copies_follow_as_docx_agent_makes_them_follow() {
    let golden: Vec<Value> = include_str!("../golden/follow_replies.jsonl")
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let calls = [
        json!({"doc": "d1", "sample": {"block": "t@body/0"}, "copies": ["t@body/1"]}),
        json!({"doc": "d1", "sample": {"block": "t@body/0"}, "copies": ["t@body/1"], "tracked": "Writer A"}),
        json!({"doc": "d1", "sample": {"doc": "d2", "block": "p:10000008"}, "copies": ["p:20000010", "t@body/1"],
               "report": "actionable"}),
        json!({"doc": "d1", "sample": {"block": "t@body/1"}, "copies": ["t@body/0/c1,1", "t@body/1"], "preview": true}),
    ];
    // the calls name the made-up document's blocks as an unstamped reading did: as it is
    // read now (stamped at open), the same blocks
    let now = |value: Value| crate::common::now(&package(), value);
    // a copy asked for by the id it has is not said again (`asked`: the address given, where
    // it names the copy otherwise)
    let asked_as_named = |mut reply: Value| {
        let copies = reply.get_mut("data").and_then(|d| d.get_mut("copies")).and_then(Value::as_array_mut);
        for copy in copies.into_iter().flatten() {
            if copy.get("asked").is_some() && copy["asked"] == copy["copy"] {
                copy.as_object_mut().unwrap().remove("asked");
            }
        }
        reply
    };
    for (call, want) in calls.into_iter().zip(golden) {
        let have = follow(now(call.clone()));
        if want["reply"]["summary"].is_null() {
            // refused when the sample and its copies were named by place (a preview leaves
            // the document unstamped): named by the ids an open gives, it is previewed
            assert!(have["reply"]["summary"].as_str().unwrap().contains("would follow"), "{have}");
            assert_eq!(have["document"], want["document"], "{call}");
            continue;
        }
        assert_eq!(have["reply"], asked_as_named(now(want["reply"].clone())), "{call}");
        assert_eq!(have["document"], want["document"], "{call}");
    }
}

#[test]
#[ignore = "a timing on a long document: FOLLOW_BENCH=<docx> cargo test --release -- --ignored"]
fn follow_on_a_long_document() {
    let Ok(path) = std::env::var("FOLLOW_BENCH") else {
        return;
    };
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
    let open = || {
        let package = Package::open(Arc::clone(&bytes), &Limits::default()).unwrap();
        Editor::open(package, StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap()
    };
    let docx = open().store().snapshot().doc().docx().unwrap();
    let body = docx.body().unwrap();
    let tables: Vec<String> = body.ids.tables().iter().take(11).map(|t| t.id).collect();
    let paragraphs: Vec<String> = body.ids.paragraphs().iter().take(51).map(|p| p.id).collect();
    for (what, blocks) in [("10 tables", &tables), ("50 paragraphs", &paragraphs)] {
        let Some((sample, copies)) = blocks.split_first() else {
            continue;
        };
        let mut times = Vec::new();
        for _ in 0..3 {
            let editor = open();
            editor.store().snapshot().doc().docx().unwrap();
            let opened = editor.store().snapshot().doc().clone();
            let caller = Caller::anonymous();
            let call_editor = CallEditor::new(&editor);
            let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-17T10:00:00Z");
            let arguments = json!({"doc": "d1", "sample": {"block": sample}, "copies": copies});
            let (args, _) = WordFollowSample::accept(arguments, Some("docx")).unwrap();
            let start = std::time::Instant::now();
            let reply = word_follow_sample(&args, &editing);
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            assert!(reply.to_json().contains_key("data"));
        }
        let best = times.iter().copied().fold(f64::MAX, f64::min);
        println!("follow {what}: {best:.1} ms (best of 3; the version read before)");
    }
}
