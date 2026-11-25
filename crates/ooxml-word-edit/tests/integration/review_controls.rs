//! An inline content control a review empties shows its placeholder, as Word 16.106's
//! Accept All and Reject All leave one: `w:showingPlcHdr` set and, the control naming no
//! placeholder text, five spaces as its content (`controls.docx`'s control 309, whose
//! content is one tracked insertion, after Word's Reject All). The placeholder run takes the
//! control's run properties and the text of the glossary placeholder it names, and a
//! block-level control gets one in its paragraph (Word 16.106 on made-up variants).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use ooxml_store::Caller;
use ooxml_word_edit::{Review, Selection};
use ooxml_zip::{Method, NewEntry, Writer};
use std::sync::Arc;

/// Control 309's `w:sdt` after a review of everything.
fn control_after(accept: bool) -> String {
    let editor = editor(&fixture("generated/controls.docx"));
    let review = Review {
        accept,
        selection: Selection::All,
    };
    editor.run(&Caller::anonymous(), &review).unwrap();
    let head = editor.store().snapshot();
    let body = String::from_utf8(head.doc().tree("word/document.xml").unwrap().unwrap().write()).unwrap();
    let at = body.find(r#"<w:id w:val="309"/>"#).unwrap();
    let start = body[..at].rfind("<w:sdt>").unwrap();
    let end = at + body[at..].find("</w:sdt>").unwrap();
    body[start..end].to_owned()
}

#[test]
fn a_control_emptied_by_reject_all_shows_its_placeholder() {
    let control = control_after(false);
    assert!(control.contains("<w:showingPlcHdr/>"), "{control}");
    assert!(
        control.contains(r#"<w:sdtContent><w:r><w:t xml:space="preserve">     </w:t></w:r></w:sdtContent>"#),
        "{control}"
    );
}

#[test]
fn a_control_keeping_its_content_shows_no_placeholder() {
    let control = control_after(true);
    assert!(!control.contains("showingPlcHdr"), "{control}");
    assert!(control.contains("Inserted words"), "{control}");
}

/// A made-up document: an inline bold control, an inline control naming a glossary
/// placeholder, and a block-level control, each holding only a tracked insertion.
fn variants() -> Arc<[u8]> {
    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let record = |id: u32| format!(r#"w:id="{id}" w:author="Reviewer" w:date="2026-10-01T09:00:00Z""#);
    let inserted = |id: u32, words: &str| {
        format!(
            r#"<w:ins {}><w:r><w:t xml:space="preserve">{words}</w:t></w:r></w:ins>"#,
            record(id)
        )
    };
    let body = format!(
        concat!(
            r#"<w:p><w:r><w:t xml:space="preserve">bold: </w:t></w:r><w:sdt><w:sdtPr><w:rPr><w:b/></w:rPr><w:id w:val="504"/><w:text/></w:sdtPr><w:sdtContent>{}</w:sdtContent></w:sdt></w:p>"#,
            r#"<w:p><w:r><w:t xml:space="preserve">named: </w:t></w:r><w:sdt><w:sdtPr><w:id w:val="506"/><w:placeholder><w:docPart w:val="FixturePlaceholder"/></w:placeholder><w:text/></w:sdtPr><w:sdtContent>{}</w:sdtContent></w:sdt></w:p>"#,
            r#"<w:sdt><w:sdtPr><w:id w:val="510"/></w:sdtPr><w:sdtContent><w:p>{}</w:p></w:sdtContent></w:sdt>"#,
            r#"<w:sectPr/>"#
        ),
        inserted(901, "bold words"),
        inserted(902, "named words"),
        inserted(903, "block words"),
    );
    let glossary = format!(
        r#"<w:glossaryDocument xmlns:w="{W}"><w:docParts><w:docPart><w:docPartPr><w:name w:val="FixturePlaceholder"/><w:category><w:name w:val="General"/><w:gallery w:val="placeholder"/></w:category><w:types><w:type w:val="bbPlcHdr"/></w:types><w:behaviors><w:behavior w:val="content"/></w:behaviors></w:docPartPr><w:docPartBody><w:p><w:r><w:rPr><w:rStyle w:val="PlaceholderText"/></w:rPr><w:t>Type a name here.</w:t></w:r></w:p></w:docPartBody></w:docPart></w:docParts></w:glossaryDocument>"#
    );
    let parts = [
        ("[Content_Types].xml", r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/glossary/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.glossary+xml"/></Types>"#.to_owned()),
        ("_rels/.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/></Relationships>"#)),
        ("word/_rels/document.xml.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/glossaryDocument" Target="glossary/document.xml"/></Relationships>"#)),
        ("word/document.xml", format!(r#"<w:document xmlns:w="{W}"><w:body>{body}</w:body></w:document>"#)),
        ("word/glossary/document.xml", glossary),
    ];
    let mut writer = Writer::new();
    for (name, data) in &parts {
        let entry = NewEntry {
            name: (*name).to_owned(),
            method: Method::Deflated,
            dos_time: 0,
            dos_date: 33,
            external_attr: 0,
            create_system: 3,
        };
        writer.add(&entry, data.as_bytes()).unwrap();
    }
    Arc::from(writer.finish(b"").unwrap())
}

/// The variants' controls after Reject All, by `w:id`.
fn rejected_variant(id: &str) -> String {
    let editor = editor(&variants());
    let review = Review {
        accept: false,
        selection: Selection::All,
    };
    editor.run(&Caller::anonymous(), &review).unwrap();
    let head = editor.store().snapshot();
    let body = String::from_utf8(head.doc().tree("word/document.xml").unwrap().unwrap().write()).unwrap();
    let at = body.find(&format!(r#"<w:id w:val="{id}"/>"#)).unwrap();
    let start = body[..at].rfind("<w:sdt>").unwrap();
    let end = at + body[at..].find("</w:sdt>").unwrap();
    body[start..end].to_owned()
}

#[test]
fn an_emptied_control_s_placeholder_run_takes_the_control_s_formatting() {
    let control = rejected_variant("504");
    assert!(control.contains("<w:showingPlcHdr/>"), "{control}");
    assert!(
        control.contains(
            r#"<w:sdtContent><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">     </w:t></w:r></w:sdtContent>"#
        ),
        "{control}"
    );
}

#[test]
fn an_emptied_control_naming_a_placeholder_shows_its_text() {
    let control = rejected_variant("506");
    assert!(control.contains("<w:showingPlcHdr/>"), "{control}");
    assert!(
        control.contains(r#"<w:sdtContent><w:r><w:t>Type a name here.</w:t></w:r></w:sdtContent>"#),
        "{control}"
    );
}

#[test]
fn an_emptied_block_level_control_shows_its_placeholder_in_its_paragraph() {
    let control = rejected_variant("510");
    assert!(control.contains("<w:showingPlcHdr/>"), "{control}");
    assert!(
        control.contains(r#"<w:r><w:t xml:space="preserve">     </w:t></w:r></w:p></w:sdtContent>"#),
        "{control}"
    );
}
