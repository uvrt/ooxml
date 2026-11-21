//! `word_read` on made-up documents: the Markdown it writes, its ranges, paging and
//! refusals (tool rules 1, 2 and 4), and its definition (the golden export docx-agent's
//! Python test holds to today's). Equality with docx-agent on the fixtures is
//! `crates/ooxml-oracle/python/word_reads_diff.py`'s and `test_rust_word_read.py`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{Never, R, W};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_tools::ToolArgs;
use ooxml_tools_word::Version;
use ooxml_tools_word::{word_read, Story, View, WordRead, KIND};
use ooxml_word_model::docx::Versions;
use ooxml_zip::{Method, NewEntry, Writer};
use std::sync::Arc;

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

fn args(range: Option<&str>) -> WordRead {
    WordRead {
        doc: "d1".into(),
        range: range.map(str::to_owned),
        view: None,
        stories: None,
        cursor: None,
    }
}

fn sample() -> Package {
    let body = [
        heading("00000001", "Results"),
        p("00000002", "Plain *star* text."),
        r#"<w:tbl><w:tblGrid><w:gridCol/><w:gridCol/></w:tblGrid><w:tr w14:paraId="00000010"><w:tc><w:p w14:paraId="00000011"><w:r><w:t>a</w:t></w:r></w:p></w:tc><w:tc><w:p w14:paraId="00000012"><w:r><w:t>b</w:t></w:r></w:p></w:tc></w:tr><w:tr w14:paraId="00000013"><w:tc><w:p w14:paraId="00000014"><w:r><w:t>c</w:t></w:r></w:p></w:tc><w:tc><w:p w14:paraId="00000015"><w:r><w:t>d</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#.to_owned(),
        heading("00000003", "Next"),
        p("00000004", "After."),
    ]
    .concat();
    package(&body)
}

fn read(args: &WordRead) -> String {
    let done = word_read(
        args,
        &Version {
            name: "a.docx",
            package: &sample(),
            number: 1,
            versions: &Versions::new(),
            placer: None,
            owned: None,
            others: ooxml_tools_word::Others::default(),
            word: None,
            counts: None,
        },
        &Never,
    )
    .unwrap();
    done.data.unwrap().as_str().unwrap().to_owned()
}

#[test]
fn the_body_is_markdown_with_ids() {
    let text = read(&args(None));
    assert_eq!(
        text,
        "<!-- p:00000001 -->\n# Results\n\n<!-- p:00000002 -->\nPlain \\*star\\* text.\n\n<!-- t:00000010 -->\n| a | b |\n| --- | --- |\n| c | d |\n\n<!-- p:00000003 -->\n# Next\n\n<!-- p:00000004 -->\nAfter.\n"
    );
}

#[test]
fn ranges_select_blocks_spans_headings_and_cells() {
    assert_eq!(read(&args(Some("p:00000004"))), "<!-- p:00000004 -->\nAfter.\n");
    assert!(read(&args(Some("p:00000002..t:00000010"))).starts_with("<!-- p:00000002 -->"));
    let heading = read(&args(Some("h:p:00000001")));
    assert!(heading.contains("# Results") && heading.contains("| c | d |") && !heading.contains("# Next"));
    assert_eq!(read(&args(Some("t:00000010/c1,1"))), "<!-- p:00000015 -->\nd\n");
    // a paragraph in a table names its table
    assert!(read(&args(Some("p:00000014"))).starts_with("<!-- t:00000010 -->"));
}

#[test]
fn a_range_naming_nothing_is_refused_with_the_way_out() {
    let error = word_read(
        &args(Some("p:0000FFFF")),
        &Version {
            name: "a.docx",
            package: &sample(),
            number: 1,
            versions: &Versions::new(),
            placer: None,
            owned: None,
            others: ooxml_tools_word::Others::default(),
            word: None,
            counts: None,
        },
        &Never,
    )
    .unwrap_err();
    let json = error.to_json();
    assert_eq!(json["code"], "not_found");
    assert_eq!(json["field"], "range");
    assert!(json["valid_options"]
        .as_array()
        .is_some_and(|o| o.iter().any(|v| v == "p:00000001")));
    assert_eq!(json["try_call"]["arguments"]["doc"], "d1");
}

#[test]
fn a_range_with_every_story_is_refused_with_the_call_that_works() {
    let mut a = args(Some("p:00000001"));
    a.stories = Some(Story::All);
    let json = word_read(
        &a,
        &Version {
            name: "a.docx",
            package: &sample(),
            number: 1,
            versions: &Versions::new(),
            placer: None,
            owned: None,
            others: ooxml_tools_word::Others::default(),
            word: None,
            counts: None,
        },
        &Never,
    )
    .unwrap_err()
    .to_json();
    assert_eq!(json["code"], "invalid_arguments");
    assert_eq!(json["try_call"]["arguments"]["stories"], "all");
}

#[test]
fn a_long_document_is_paged_at_line_ends() {
    let body: String = (0..3000)
        .map(|k| p(&format!("{:08X}", k + 1), "A line of text that repeats."))
        .collect();
    let pkg = package(&body);
    let mut a = args(None);
    let first = word_read(
        &a,
        &Version {
            name: "a.docx",
            package: &pkg,
            number: 1,
            versions: &Versions::new(),
            placer: None,
            owned: None,
            others: ooxml_tools_word::Others::default(),
            word: None,
            counts: None,
        },
        &Never,
    )
    .unwrap();
    let cursor = first.next_cursor.clone().unwrap();
    let page = first.data.unwrap().as_str().unwrap().to_owned();
    assert!(page.ends_with('\n') && page.chars().count() <= 22_500);
    assert_eq!(format!("c{}", page.chars().count()), cursor);
    a.cursor = Some(cursor);
    let second = word_read(
        &a,
        &Version {
            name: "a.docx",
            package: &pkg,
            number: 1,
            versions: &Versions::new(),
            placer: None,
            owned: None,
            others: ooxml_tools_word::Others::default(),
            word: None,
            counts: None,
        },
        &Never,
    )
    .unwrap();
    assert!(second.data.unwrap().as_str().unwrap().starts_with("<!-- p:"));
    a.cursor = Some("x1".into());
    let json = word_read(
        &a,
        &Version {
            name: "a.docx",
            package: &pkg,
            number: 1,
            versions: &Versions::new(),
            placer: None,
            owned: None,
            others: ooxml_tools_word::Others::default(),
            word: None,
            counts: None,
        },
        &Never,
    )
    .unwrap_err()
    .to_json();
    assert_eq!(
        (json["code"].as_str(), json["field"].as_str()),
        (Some("invalid_arguments"), Some("cursor"))
    );
}

#[test]
fn views_name_the_summary() {
    let mut a = args(None);
    a.view = Some(View::Markup);
    let done = word_read(
        &a,
        &Version {
            name: "a.docx",
            package: &sample(),
            number: 1,
            versions: &Versions::new(),
            placer: None,
            owned: None,
            others: ooxml_tools_word::Others::default(),
            word: None,
            counts: None,
        },
        &Never,
    )
    .unwrap();
    assert_eq!(done.summary, "Read the document (markup)");
}

#[test]
fn the_definition_is_the_golden_export() {
    assert!(WordRead::problems().is_empty(), "{:?}", WordRead::problems());
    let golden = include_str!("../golden/word_read.json");
    let export = WordRead::spec().export(Some(&[KIND]));
    assert_eq!(
        format!("{export}\n"),
        golden,
        "run `cargo run -p ooxml-tools-word --example export_specs`"
    );
}
