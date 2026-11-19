//! `find_text` and `word_inspect` on made-up documents: their definitions (the golden
//! exports docx-agent's Python test holds to today's), matches and paging, the formatting
//! inspected, and their refusals with the way out (tool rules 1, 2 and 4). Equality with
//! docx-agent on the fixtures is `crates/ooxml-oracle/python/word_reads_diff.py`'s and
//! `test_rust_reads.py`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::{Cancel, Limits};
use ooxml_opc::Package;
use ooxml_tools_word::{find_text, word_inspect, FindStories, FindText, Version, WordInspect};
use ooxml_word_model::docx::Versions;
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::{json, Value};
use std::sync::Arc;

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#;
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";

struct Never;

impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

fn package(body: &str) -> Package {
    let rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}officeDocument" Target="word/document.xml"/></Relationships>"#
    );
    let styles_rel = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}styles" Target="styles.xml"/></Relationships>"#
    );
    let styles = format!(
        r#"<w:styles {W}><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/></w:style></w:styles>"#
    );
    let document = format!("<w:document {W}><w:body>{body}</w:body></w:document>");
    let mut w = Writer::new();
    for (name, data) in [
        ("_rels/.rels", rels),
        ("word/_rels/document.xml.rels", styles_rel),
        ("word/styles.xml", styles),
        ("word/document.xml", document),
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
    Package::open(Arc::from(w.finish(b"").unwrap()), &Limits::default()).unwrap()
}

fn p(id: &str, text: &str) -> String {
    format!(r#"<w:p w14:paraId="{id}"><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>"#)
}

fn heading(id: &str, text: &str) -> String {
    format!(r#"<w:p w14:paraId="{id}"><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#)
}

fn sample() -> Package {
    let body = [
        heading("00000001", "Results"),
        p("00000002", "The cat sat on the cat mat."),
        r#"<w:tbl><w:tblPr><w:tblStyle w:val="Grid"/></w:tblPr><w:tblGrid><w:gridCol/></w:tblGrid><w:tr w14:paraId="00000010"><w:tc><w:tcPr><w:shd w:val="clear" w:color="auto" w:fill="1F3864"/></w:tcPr><w:p w14:paraId="00000011"><w:r><w:rPr><w:b/></w:rPr><w:t>cat</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#.to_owned(),
    ]
    .concat();
    package(&body)
}

fn find(args: &FindText) -> Result<Value, Value> {
    let versions = Versions::new();
    let package = sample();
    let version = Version {
        name: "a.docx",
        package: &package,
        number: 1,
        versions: &versions,
        placer: None,
        others: ooxml_tools_word::Others::default(),
    };
    find_text(args, &version, &Never)
        .map(|d| json!({"summary": d.summary, "data": d.data, "next": d.next_cursor, "total": d.total}))
        .map_err(|e| e.to_json())
}

fn inspect(target: &str, layout: Option<bool>) -> Result<Value, Value> {
    let versions = Versions::new();
    let package = sample();
    let version = Version {
        name: "a.docx",
        package: &package,
        number: 1,
        versions: &versions,
        placer: None,
        others: ooxml_tools_word::Others::default(),
    };
    let args = WordInspect {
        doc: "d1".into(),
        target: target.into(),
        layout,
    };
    word_inspect(&args, &version, &Never)
        .map(|d| d.data.unwrap())
        .map_err(|e| e.to_json())
}

fn looking_for(text: &str) -> FindText {
    FindText {
        doc: "d1".into(),
        text: text.into(),
        regex: None,
        slides: None,
        range: None,
        stories: None,
        cursor: None,
    }
}

#[test]
fn every_match_is_a_text_range_with_its_context() {
    let found = find(&looking_for("cat")).unwrap();
    assert_eq!(found["summary"], "3 match(es)");
    assert_eq!(
        found["data"][0],
        json!({"address": "p:00000002@4:7", "kind": "text", "text": "cat", "context": "The [cat] sat on the cat mat."})
    );
    assert_eq!(found["data"][2]["address"], "p:00000011@0:3");
    let mut within = looking_for("cat");
    within.range = Some("t:00000010".into());
    assert_eq!(find(&within).unwrap()["total"], 1);
    let mut all = looking_for("cat");
    all.stories = Some(FindStories::All);
    assert_eq!(find(&all).unwrap()["total"], 3);
}

#[test]
fn find_refusals_carry_the_way_out_and_a_python_pattern_is_matched() {
    let empty = find(&looking_for("")).unwrap_err();
    assert_eq!(empty["code"], "unit");
    let mut pattern = looking_for("c.t");
    pattern.regex = Some(true);
    assert_eq!(find(&pattern).unwrap()["total"], 3);
    let mut empty_match = looking_for("c?a*");
    empty_match.regex = Some(true);
    let refused = find(&empty_match).unwrap_err();
    assert_eq!(refused["code"], "refused");
    assert_eq!(refused["field"], "regex");
    assert_eq!(refused["try_call"]["arguments"], json!({"doc": "d1", "text": "c?a*"}));
    let mut plain = looking_for("cat");
    plain.regex = Some(true);
    assert_eq!(find(&plain).unwrap()["total"], 3);
    let mut missing = looking_for("cat");
    missing.range = Some("p:0000FFFF".into());
    let refused = find(&missing).unwrap_err();
    assert_eq!(refused["code"], "not_found");
    assert_eq!(refused["field"], "range");
    let mut cursor = looking_for("cat");
    cursor.cursor = Some("x".into());
    assert_eq!(
        find(&cursor).unwrap_err()["try_call"]["arguments"],
        json!({"doc": "d1", "text": "cat"})
    );
}

#[test]
fn a_paragraph_and_a_cell_are_inspected() {
    let data = inspect("p:00000002", None).unwrap();
    let block = &data["blocks"][0];
    assert_eq!(block["id"], "p:00000002");
    assert_eq!(block["t"], "The cat sat on the cat mat.");
    assert_eq!(block["effective"]["keep_lines"], false);
    assert_eq!(block["runs"][0]["t"], "The cat sat on the cat mat.");
    let table = inspect("t:00000010", None).unwrap();
    let cell = &table["blocks"][0]["rows"][0][0];
    assert_eq!(cell["fill"], "1F3864");
    assert_eq!(cell["text_color"], "FFFFFF");
    assert_eq!(cell["blocks"][0]["runs"][0]["b"], true);
    let one = inspect("t:00000010/c0,0", None).unwrap();
    assert_eq!(one["cell"]["properties"]["shading"]["fill"], "1F3864");
}

#[test]
fn inspect_refusals_carry_the_way_out() {
    let missing = inspect("p:0000FFFF", None).unwrap_err();
    assert_eq!(missing["code"], "not_found");
    assert_eq!(missing["try_call"]["arguments"]["target"], "body");
    let laid = inspect("body", Some(true)).unwrap_err();
    assert_eq!(laid["code"], "refused");
    assert_eq!(laid["try_call"]["arguments"], json!({"doc": "d1", "target": "body"}));
}
