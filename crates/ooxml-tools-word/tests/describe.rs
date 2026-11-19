//! `describe` on made-up Word packages: what each field counts, and its definition held to
//! the golden export that `python/docx-agent/tests/test_rust_describe.py` holds to today's
//! Python definition byte for byte.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::{Cancel, Limits};
use ooxml_opc::Package;
use ooxml_tools::{ToolArgs, ToolResult};
use ooxml_tools_word::{describe, Describe, Version, DESCRIBED, KIND};
use ooxml_word_model::docx::Versions;
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";

fn rels(items: &[(&str, &str, &str)]) -> String {
    let body: String = items
        .iter()
        .map(|(id, kind, target)| format!(r#"<Relationship Id="{id}" Type="{kind}" Target="{target}"/>"#))
        .collect();
    format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
    )
}

fn package(parts: &[(&str, String)]) -> Package {
    let mut w = Writer::new();
    for (name, data) in parts {
        let entry = NewEntry {
            name: (*name).into(),
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

/// A document with comments (a thread with a reply, a resolved one, one without an
/// author), footnotes and separators, content controls (one under `mc:Fallback`), a
/// header, a compatibility mode and core properties.
fn made_up() -> Package {
    let root = rels(&[
        ("rId1", &format!("{R}officeDocument"), "word/document.xml"),
        (
            "rId2",
            "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties",
            "docProps/core.xml",
        ),
    ]);
    let main = rels(&[
        ("rId1", &format!("{R}settings"), "settings.xml"),
        ("rId2", &format!("{R}comments"), "comments.xml"),
        (
            "rId3",
            "http://schemas.microsoft.com/office/2011/relationships/commentsExtended",
            "commentsExtended.xml",
        ),
        (
            "rId4",
            "http://schemas.microsoft.com/office/2016/09/relationships/commentsIds",
            "commentsIds.xml",
        ),
        ("rId5", &format!("{R}footnotes"), "footnotes.xml"),
        ("rId6", &format!("{R}header"), "header1.xml"),
    ]);
    let core = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Ann</dc:creator><dc:title>Plan<!-- draft --> two</dc:title><dc:description>x</dc:description></cp:coreProperties>"#;
    let settings = format!(
        r#"<w:settings {W}><w:compat><w:compatSetting w:name="other" w:val="1"/><w:compatSetting w:name="compatibilityMode" w:val=" 14 "/></w:compat></w:settings>"#
    );
    let document = format!(
        r#"<w:document {W} xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><w:body><w:sdt><w:sdtContent><w:p/></w:sdtContent></w:sdt><mc:AlternateContent><mc:Choice Requires="w14"><w:sdt/></mc:Choice><mc:Fallback><w:sdt/></mc:Fallback></mc:AlternateContent><w:p/></w:body></w:document>"#
    );
    let w14 = r#"xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#;
    let comments = format!(
        r#"<w:comments {W} {w14}><w:comment w:id="0" w:author="Ann"><w:p w14:paraId="0000000a"/></w:comment><w:comment w:id="1" w:author="Bob"><w:p w14:paraId="0000000B"/></w:comment><w:comment w:id="2"><w:p/><w:p w14:paraId="0000000C"/></w:comment></w:comments>"#
    );
    let extended = r#"<w15:commentsEx xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w15:commentEx w15:paraId="0000000A" w15:done="0"/><w15:commentEx w15:paraId="0000000B" w15:paraIdParent="0000000a" w15:done="0"/><w15:commentEx w15:paraId="0000000C" w15:done="1"/></w15:commentsEx>"#;
    let ids = r#"<w16cid:commentsIds xmlns:w16cid="http://schemas.microsoft.com/office/word/2016/wordml/cid"><w16cid:commentId w16cid:paraId="0000000A" w16cid:durableId="1A2B3C4D"/></w16cid:commentsIds>"#;
    let notes = format!(
        r#"<w:footnotes {W}><w:footnote w:type="separator" w:id="-1"/><w:footnote w:type="continuationSeparator" w:id="0"/><w:footnote w:id="1"/><w:footnote w:type="normal" w:id="2"/></w:footnotes>"#
    );
    let header = format!(r#"<w:hdr {W}><w:sdt/></w:hdr>"#);
    package(&[
        ("_rels/.rels", root),
        ("docProps/core.xml", core.into()),
        ("word/document.xml", document),
        ("word/_rels/document.xml.rels", main),
        ("word/settings.xml", settings),
        ("word/comments.xml", comments),
        ("word/commentsExtended.xml", extended.into()),
        ("word/commentsIds.xml", ids.into()),
        ("word/footnotes.xml", notes),
        ("word/header1.xml", header),
    ])
}

struct Never;
impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

struct After(AtomicBool);
impl Cancel for After {
    fn is_cancelled(&self) -> bool {
        self.0.swap(true, Ordering::SeqCst)
    }
}

fn args() -> Describe {
    Describe { doc: "d1".into() }
}

#[test]
fn describes_what_the_parts_say() {
    let done = describe(
        &args(),
        &Version {
            name: "plan.docx",
            package: &made_up(),
            number: 1,
            versions: &Versions::new(),
            placer: None,
            others: ooxml_tools_word::Others::default(),
        },
        &Never,
    )
    .unwrap();
    assert_eq!(done.summary, "Described plan.docx");
    let data = done.data.unwrap();
    assert_eq!(
        data,
        json!({
            "name": "plan.docx",
            "compatibility_mode": 14,
            "properties": {"title": "Plan", "author": "Ann"},
            "tracking": null,
            "headings": [],
            "sections": [],
            "styles_in_use": [],
            "comments": {"threads": 2, "open": 1, "by_author": {"Ann": 1, "Bob": 1, "null": 1}},
            "revisions": {"records": 0, "changes": 0, "by_author": {}},
            "fields": {},
            "tables": [],
            "drawings": [],
            "notes": {"footnote": 2},
            "controls": 3,
        })
    );
    let keys: Vec<&str> = data.as_object().unwrap().keys().map(String::as_str).collect();
    let listed: Vec<&str> = DESCRIBED
        .iter()
        .copied()
        .filter(|k| !["tagged", "headings_note"].contains(k))
        .collect();
    assert_eq!(keys, listed);
}

#[test]
fn a_bare_package_has_the_defaults() {
    let root = rels(&[("rId1", &format!("{R}officeDocument"), "word/document.xml")]);
    let p = package(&[
        ("_rels/.rels", root),
        ("word/document.xml", format!("<w:document {W}><w:body/></w:document>")),
    ]);
    let data = describe(
        &args(),
        &Version {
            name: "a.docx",
            package: &p,
            number: 1,
            versions: &Versions::new(),
            placer: None,
            others: ooxml_tools_word::Others::default(),
        },
        &Never,
    )
    .unwrap()
    .data
    .unwrap();
    assert_eq!(data["compatibility_mode"], json!(12));
    assert_eq!(data["properties"], json!({}));
    assert_eq!(data["comments"], json!({"threads": 0, "open": 0, "by_author": {}}));
    assert_eq!(data["notes"], json!({}));
    assert_eq!(data["controls"], json!(0));
}

#[test]
fn cancellation_stops_it_with_timeout() {
    // the first check passes, the next one stops it
    let error = describe(
        &args(),
        &Version {
            name: "plan.docx",
            package: &made_up(),
            number: 1,
            versions: &Versions::new(),
            placer: None,
            others: ooxml_tools_word::Others::default(),
        },
        &After(AtomicBool::new(false)),
    )
    .unwrap_err();
    assert_eq!(error.code().as_str(), "timeout");
    let result = ToolResult::<Value>::failed(error);
    assert!(result.to_text(None).contains("\"code\":\"timeout\""));
}

#[test]
fn not_a_word_package_is_an_internal_error() {
    let p = package(&[("other.xml", "<a/>".into())]);
    let error = describe(
        &args(),
        &Version {
            name: "x.docx",
            package: &p,
            number: 1,
            versions: &Versions::new(),
            placer: None,
            others: ooxml_tools_word::Others::default(),
        },
        &Never,
    )
    .unwrap_err();
    assert_eq!(error.code().as_str(), "internal");
}

#[test]
fn the_definition_is_the_golden_export() {
    assert!(Describe::problems().is_empty(), "{:?}", Describe::problems());
    let golden = include_str!("golden/describe.json");
    let export = Describe::spec().export(Some(&[KIND]));
    assert_eq!(
        format!("{export}\n"),
        golden,
        "run `cargo run -p ooxml-tools-word --example export_specs`"
    );
}
