//! `word_compare_blocks` on a made-up document (`golden/compare_body.xml`): two tables and
//! two paragraphs that differ in every way the tool reports, compared in the document and
//! with the document opened again as another; the replies are docx-agent's
//! (`golden/compare_replies.jsonl`, written by docx-agent's tool on the same document).
//! Its equality with docx-agent on the corpus is `word_edits_diff.py --style`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, R, W};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_tools::ToolArgs;
use ooxml_tools_word::{word_compare_blocks, FollowReport, Others, Version, WordCompareBlocks, WordFollowSample};
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

/// The tool's reply (its summary, data and total) on the document, `d2` the same document
/// opened again.
fn reply(arguments: Value) -> Value {
    let (mine, copy) = (package(), package());
    let versions = (Versions::new(), Versions::new());
    let copy = crate::common::read(&copy);
    let other = Version::new("b.docx", &copy, 1, &versions.1);
    let others = [("d2", other)];
    let mine = crate::common::read(&mine);
    let version = Version::new("a.docx", &mine, 1, &versions.0).with_others(Others::new(&others));
    let (args, _) = WordCompareBlocks::accept(arguments, Some("docx")).unwrap();
    match word_compare_blocks(&args, &version, &Never) {
        Ok(done) => json!({"summary": done.summary, "data": done.data, "total": done.total}),
        Err(error) => json!({"error": error.message()}),
    }
}

#[test]
fn tables_and_paragraphs_compare_as_docx_agent_compares_them() {
    let golden: Vec<Value> = include_str!("../golden/compare_replies.jsonl")
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let calls = [
        json!({"doc": "d1", "blocks": ["t@body/1", "t@body/0"], "reference": {"block": "t@body/0"}}),
        json!({"doc": "d1", "blocks": ["p:20000010", "t@body/1"], "reference": {"doc": "d2", "block": "p:10000008"}}),
    ];
    // the replies name the made-up document's blocks as an unstamped reading did: as it
    // is read now (stamped at open), the same blocks
    let now = |value: Value| crate::common::now(&package(), value);
    for (call, want) in calls.into_iter().zip(golden) {
        assert_eq!(reply(now(call)), now(want));
    }
}

#[test]
fn word_follow_sample_reads_an_empty_report_as_the_default() {
    // Tool rule 1: "" is a strict decoder's placeholder for a report not given (docx-agent:
    // `report or "all"`); a report it does not know is still refused.
    let call = |report: Value| {
        WordFollowSample::accept(
            json!({"doc": "d1", "sample": {"block": "p:1"}, "copies": ["p:2"], "report": report}),
            Some("docx"),
        )
    };
    let (args, _) = call(json!("")).unwrap();
    assert_eq!(args.report, None);
    let (args, _) = call(json!("actionable")).unwrap();
    assert_eq!(args.report, Some(FollowReport::Actionable));
    assert!(call(json!("some")).is_err());
}

#[test]
fn an_unknown_block_is_refused_and_a_cursor_pages() {
    let refused = reply(json!({"doc": "d1", "blocks": ["p:NOSUCH"], "reference": {"block": "p:10000008"}}));
    assert!(refused["error"].as_str().unwrap().contains("p:NOSUCH"), "{refused}");
    let paged = reply(crate::common::now(
        &package(),
        json!({"doc": "d1", "blocks": ["t@body/1"], "reference": {"block": "t@body/0"}, "cursor": "c15"}),
    ));
    assert_eq!(paged["data"]["differences"].as_array().unwrap().len(), 4);
}

#[test]
#[ignore = "a timing on a long document: COMPARE_BENCH=<docx> cargo test --release -- --ignored"]
fn compare_on_a_long_document() {
    let Ok(path) = std::env::var("COMPARE_BENCH") else {
        return;
    };
    let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
    let package = Package::open(Arc::clone(&bytes), &Limits::default()).unwrap();
    let versions = Versions::new();
    let package = crate::common::read(&package);
    let version = Version::new("long.docx", &package, 1, &versions);
    let start = std::time::Instant::now();
    let docx = version.docx().unwrap();
    println!("the version read: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
    let body = docx.body().unwrap();
    let tables: Vec<String> = body.ids.tables().iter().take(10).map(|t| t.id).collect();
    let paragraphs: Vec<String> = body.ids.paragraphs().iter().take(50).map(|p| p.id).collect();
    for (what, blocks) in [("10 tables", &tables), ("50 paragraphs", &paragraphs)] {
        let Some(first) = blocks.first() else { continue };
        let arguments = json!({"doc": "d1", "blocks": blocks, "reference": {"block": first}});
        let (args, _) = WordCompareBlocks::accept(arguments, Some("docx")).unwrap();
        let mut times = Vec::new();
        for _ in 0..4 {
            let start = std::time::Instant::now();
            let done = word_compare_blocks(&args, &version, &Never).unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            assert!(done.total.is_some());
        }
        let best = times.iter().skip(1).copied().fold(f64::MAX, f64::min);
        println!(
            "compare {what}: first {:.1} ms, then {best:.1} ms (best of 3)",
            times[0]
        );
    }
}
