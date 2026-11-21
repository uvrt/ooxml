//! A version written as a file (`serialise`) and cleaned (`clean`) on made-up packages: the
//! main part retyped for a template, `app.xml`'s statistics written, the macro refusal; rsids,
//! web divisions, an external template link and a stray entry taken out, a drawn division
//! kept. Equality with docx-agent's files, as Word reads them, is docx-agent's
//! `tests/test_rust_save.py`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_word_edit::{clean, serialise, AppStatistics, FileKind};
use ooxml_zip::{Method, NewEntry, Writer};
use std::sync::Arc;

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";
const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn zip(parts: &[(&str, String)]) -> Vec<u8> {
    let mut w = Writer::new();
    for (name, data) in parts {
        let entry = NewEntry {
            name: (*name).to_owned(),
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

/// A document with settings, web settings and `app.xml`; `extra` parts beside them.
fn document(margin: &str, vba: bool) -> Vec<u8> {
    let vba_rel = if vba {
        r#"<Relationship Id="rId9" Type="http://schemas.microsoft.com/office/2006/relationships/vbaProject" Target="vbaProject.bin"/>"#
    } else {
        ""
    };
    let parts = vec![
        ("[Content_Types].xml", r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="bin" ContentType="application/vnd.ms-office.vbaProject"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/><Override PartName="/word/webSettings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.webSettings+xml"/><Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/></Types>"#.to_owned()),
        ("_rels/.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="{REL}extended-properties" Target="docProps/app.xml"/></Relationships>"#)),
        ("docProps/app.xml", r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Pages>1</Pages><Words>0</Words><Characters>0</Characters><Lines>1</Lines><Paragraphs>0</Paragraphs></Properties>"#.to_owned()),
        ("word/_rels/document.xml.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}settings" Target="settings.xml"/><Relationship Id="rId2" Type="{REL}webSettings" Target="webSettings.xml"/>{vba_rel}</Relationships>"#)),
        ("word/document.xml", format!(r#"<w:document xmlns:w="{W}"><w:body><w:p w:rsidR="00A1B2C3"><w:pPr><w:divId w:val="7"/></w:pPr><w:r w:rsidRPr="00D4E5F6"><w:t>Hi there</w:t></w:r></w:p></w:body></w:document>"#)),
        ("word/settings.xml", format!(r#"<w:settings xmlns:w="{W}" xmlns:r="{}"><w:attachedTemplate r:id="rId1"/><w:rsids><w:rsidRoot w:val="00A1B2C3"/><w:rsid w:val="00A1B2C3"/></w:rsids></w:settings>"#, REL.trim_end_matches('/'))),
        ("word/_rels/settings.xml.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}attachedTemplate" Target="file:///C:/Normal.dotm" TargetMode="External"/></Relationships>"#)),
        ("word/webSettings.xml", format!(r#"<w:webSettings xmlns:w="{W}"><w:divs><w:div w:id="7"><w:marLeft w:val="{margin}"/></w:div></w:divs></w:webSettings>"#)),
        ("word/vbaProject.bin", "vba".to_owned()),
        ("[trash]/0000.dat", "x".repeat(64)),
    ];
    zip(&parts)
}

fn open(data: &[u8]) -> Package {
    Package::open(Arc::from(data), &Limits::default()).unwrap()
}

fn part(package: &Package, name: &str) -> String {
    String::from_utf8(package.read(name).unwrap().unwrap().to_vec()).unwrap()
}

#[test]
fn a_template_is_retyped_and_the_statistics_written() {
    let package = open(&document("0", false));
    let stats = AppStatistics {
        words: 2,
        characters: 7,
        characters_with_spaces: 8,
        paragraphs: 1,
        pages: Some(3),
        lines: None,
    };
    let saved = open(&serialise(&package, FileKind::Dotx, Some(&stats)).unwrap());
    let types = part(&saved, "[Content_Types].xml");
    assert!(types.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n"));
    assert!(types.contains(r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml"/>"#));
    assert_eq!(FileKind::of(&saved).unwrap(), Some(FileKind::Dotx));
    let app = part(&saved, "docProps/app.xml");
    // a statistic the layout does not know (lines) stays as it was
    assert!(app.contains(
        "<Pages>3</Pages><Words>2</Words><Characters>7</Characters><Lines>1</Lines><Paragraphs>1</Paragraphs>"
    ));
    assert!(
        !app.contains("CharactersWithSpaces"),
        "a statistic the part lacks is not added"
    );
    // nothing asked: the same file
    let same = serialise(&package, FileKind::Docx, None).unwrap();
    assert_eq!(same, package.save().unwrap());
}

#[test]
fn macros_a_docx_cannot_carry_are_refused() {
    let package = open(&document("0", true));
    let refused = serialise(&package, FileKind::Docx, None).unwrap_err();
    assert!(
        refused.to_string().contains("macros, which a .docx cannot carry"),
        "{refused}"
    );
}

#[test]
fn clean_takes_out_what_word_never_draws() {
    let data = document("0", false);
    let (out, cleaned) = clean(&data, &Limits::default()).unwrap();
    let report = cleaned.to_json();
    assert_eq!(report["attached_template"]["target"], "file:///C:/Normal.dotm");
    assert_eq!(report["rsids"]["attributes"], 2);
    assert_eq!(report["rsids"]["listed"], 2);
    assert_eq!(report["web_divs"]["count"], 1);
    // the stray entry, and the VBA part nothing relates to without its relationship
    assert_eq!(
        report["orphaned_parts"]["parts"],
        serde_json::json!(["[trash]/0000.dat", "word/vbaProject.bin"])
    );
    assert_eq!(report["bytes"]["after"], out.len());
    let package = open(&out);
    assert!(!package.has_part("[trash]/0000.dat"));
    let body = part(&package, "word/document.xml");
    assert!(!body.contains("rsid") && !body.contains("divId"), "{body}");
    assert!(!part(&package, "word/settings.xml").contains("attachedTemplate"));
    assert!(!part(&package, "word/_rels/settings.xml.rels").contains("attachedTemplate"));
    assert!(!part(&package, "word/webSettings.xml").contains("w:divs"));
    let types = part(&package, "[Content_Types].xml");
    assert!(
        !types.contains("Extension=\"bin\""),
        "a Default no part has any more goes: {types}"
    );
}

#[test]
fn a_drawn_division_stays_and_a_clean_file_cleans_to_itself() {
    let (out, cleaned) = clean(&document("1440", false), &Limits::default()).unwrap();
    assert!(cleaned.web_divs.is_none());
    assert!(part(&open(&out), "word/document.xml").contains("divId"));
    let (again, report) = clean(&out, &Limits::default()).unwrap();
    assert_eq!(again, out);
    assert_eq!(
        report.to_json(),
        serde_json::json!({"bytes": {"before": out.len(), "after": out.len()}})
    );
}
