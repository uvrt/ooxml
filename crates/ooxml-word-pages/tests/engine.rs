//! The page engine over made-up packages: a version read again from its bytes keeps the
//! parts whose stored bytes did not change and is laid out from the version before; a
//! version given as its touched parts is the version given whole; a document the layout
//! does not lay out yet is a fallback with its reason; columns with a line between them,
//! line numbers and a drop cap are laid out and painted in Rust; a floating shape is
//! painted in Rust and said of as docx2svg says it. The faces are the open ones the repository ships.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ooxml_derive::CacheBudget;
use ooxml_fonts::{Folder, Location, WordFontOptions};
use ooxml_word_pages::{support, DiskFonts, EngineOptions, FontSetup, PageEngine};
use ooxml_zip::{Method, NewEntry, Writer};
use std::sync::Arc;

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

struct Unlimited;

impl CacheBudget for Unlimited {
    fn reserve(&self, _bytes: usize) -> bool {
        true
    }
    fn release(&self, _bytes: usize) {}
    fn used(&self) -> usize {
        0
    }
}

fn document_xml(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W}><w:body>{body}</w:body></w:document>"#
    )
}

fn package(body: &str) -> Vec<u8> {
    package_styled(body, None)
}

/// A package of `body` with `styles` (`w:styles`' content) where given.
fn package_styled(body: &str, styles: Option<&str>) -> Vec<u8> {
    package_parts(body, styles, None)
}

/// A package of `body` with `styles` where given and a default header of `header`
/// (`w:hdr`'s content, related as `rIdH`) where given.
fn package_parts(body: &str, styles: Option<&str>, header: Option<&str>) -> Vec<u8> {
    let entry = |name: &str| NewEntry {
        name: name.into(),
        method: Method::Deflated,
        dos_time: 0x6000,
        dos_date: 0x5a21,
        external_attr: 0,
        create_system: 0,
    };
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let header_rel = r#"<Relationship Id="rIdH" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>"#;
    let styles_rel = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>{}</Relationships>"#,
        if header.is_some() { header_rel } else { "" }
    );
    let mut files = vec![
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#.to_owned(),
        ),
        ("_rels/.rels", rels.to_owned()),
        ("word/document.xml", document_xml(body)),
        ("word/_rels/document.xml.rels", styles_rel),
        (
            "word/styles.xml",
            format!(r#"<w:styles {W}>{}</w:styles>"#, styles.unwrap_or_default()),
        ),
    ];
    if let Some(header) = header {
        files.push(("word/header1.xml", format!(r#"<w:hdr {W}>{header}</w:hdr>"#)));
    }
    let mut w = Writer::new();
    for (name, data) in files {
        w.add(&entry(name), data.as_bytes()).unwrap();
    }
    w.finish(b"").unwrap()
}

fn paragraphs(n: usize, edited: Option<usize>) -> String {
    (0..n)
        .map(|k| {
            let text = if Some(k) == edited {
                "changed words here"
            } else {
                "some words on a line of the body"
            };
            format!("<w:p><w:r><w:t>{k} {text}</w:t></w:r></w:p>")
        })
        .collect()
}

/// The open faces the repository ships (pptx2svg-fonts), Calibri laid out with Carlito.
fn engine() -> PageEngine {
    let folder = Folder {
        location: Location::Folder,
        path: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../python/pptx2svg/packages/pptx2svg-fonts/src/pptx2svg_fonts/files"
        )
        .to_owned(),
    };
    let options = WordFontOptions {
        substitutes: vec![("calibri".to_owned(), "Carlito".to_owned())],
        recorded_faces: false,
        fallbacks: false,
    };
    let setup = Arc::new(FontSetup::new(Arc::new(DiskFonts), &[folder], options));
    PageEngine::new(setup, EngineOptions::default(), Arc::new(Unlimited))
}

fn svgs(engine: &mut PageEngine, version: &ooxml_word_pages::LaidVersion) -> Vec<String> {
    (0..version.page_count())
        .map(|k| engine.page_svg(version, k).unwrap().0)
        .collect()
}

#[test]
fn a_version_is_laid_out_from_the_one_before_reading_only_what_changed() {
    let mut kept = engine();
    let first = kept
        .lay_out_bytes(Arc::from(package(&paragraphs(120, None))))
        .unwrap()
        .unwrap();
    assert!(!first.relaid());
    let edited = package(&paragraphs(120, Some(60)));
    let second = kept.lay_out_bytes(Arc::from(edited.clone())).unwrap().unwrap();
    assert!(second.relaid());
    let stats = kept.stats();
    assert_eq!(stats.parts_read, 1, "only word/document.xml changed: {stats:?}");
    assert_eq!(stats.parts_kept, 4);
    let mut fresh = engine();
    let whole = fresh.lay_out_bytes(Arc::from(edited)).unwrap().unwrap();
    assert_eq!(svgs(&mut kept, &second), svgs(&mut fresh, &whole));
    assert_eq!(second.facts(), whole.facts());
}

#[test]
fn a_version_given_by_its_touched_parts_is_the_version_given_whole() {
    let mut kept = engine();
    kept.lay_out_bytes(Arc::from(package(&paragraphs(40, None))))
        .unwrap()
        .unwrap();
    let touched: Arc<[u8]> = Arc::from(document_xml(&paragraphs(40, Some(3))).into_bytes());
    let updated = kept
        .update(&[("word/document.xml".to_owned(), Some(touched))])
        .unwrap()
        .unwrap();
    let mut fresh = engine();
    let whole = fresh
        .lay_out_bytes(Arc::from(package(&paragraphs(40, Some(3)))))
        .unwrap()
        .unwrap();
    assert_eq!(svgs(&mut kept, &updated), svgs(&mut fresh, &whole));
}

#[test]
fn a_floating_table_falls_back_with_its_reason() {
    let body = format!(
        r#"<w:tbl><w:tblPr><w:tblpPr w:tblpX="500" w:tblpY="500"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>{}"#,
        paragraphs(2, None)
    );
    let fallback = engine().lay_out_bytes(Arc::from(package(&body))).unwrap().unwrap_err();
    assert_eq!(fallback.reason, support::FLOATING_TABLE);
    assert!(support::REASONS.contains(&fallback.reason));
}

#[test]
fn columns_line_numbers_and_a_drop_cap_are_laid_out_and_painted_in_rust() {
    let body = format!(
        r#"<w:p><w:pPr><w:framePr w:dropCap="drop" w:lines="3" w:wrap="around" w:vAnchor="text" w:hAnchor="text"/></w:pPr><w:r><w:t>D</w:t></w:r></w:p>{}<w:sectPr xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:headerReference w:type="default" r:id="rIdH"/><w:lnNumType w:countBy="5"/><w:cols w:num="2" w:space="720" w:sep="1"/></w:sectPr>"#,
        paragraphs(60, None)
    );
    let header = "<w:p><w:r><w:t>A header of two words</w:t></w:r></w:p>";
    let mut kept = engine();
    let version = kept
        .lay_out_bytes(Arc::from(package_parts(&body, None, Some(header))))
        .unwrap()
        .unwrap();
    for k in 0..version.page_count() {
        let page = kept.page(&version, k).unwrap();
        assert!(page.unpainted.is_empty(), "page {k}: {:?}", page.unpainted);
        assert!(page.rules.iter().any(|r| r.kind == "column-separator"), "page {k}");
    }
    // The header's lines are facts of their own, as painted.
    let painted = kept.page(&version, 0).unwrap();
    let story_lines = kept.story_lines(&version, 0).unwrap();
    let drawn: Vec<_> = painted
        .lines
        .iter()
        .filter(|l| l.story.is_some())
        .map(|l| (l.path.clone(), l.baseline))
        .collect();
    let facts: Vec<_> = story_lines
        .iter()
        .map(|l| (l.line.path.clone(), l.line.baseline))
        .collect();
    assert!(!facts.is_empty());
    assert_eq!(drawn, facts);
    // A section of several columns is not numbered: docx2svg says so.
    let said = version.said();
    assert!(
        said.warnings.iter().any(|w| w.code == "line-numbers-not-drawn"),
        "{said:?}"
    );
}

#[test]
fn a_page_of_another_generation_is_not_painted() {
    let mut first = engine();
    let version = first
        .lay_out_bytes(Arc::from(package(&paragraphs(3, None))))
        .unwrap()
        .unwrap();
    let mut other = engine();
    assert!(other.paint(&version, 0).is_none());
    assert!(first.paint(&version, 0).is_some());
}

#[test]
fn a_change_of_the_styles_lays_the_version_out_whole() {
    let body = paragraphs(60, None);
    let large = r#"<w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val="40"/></w:rPr></w:rPrDefault></w:docDefaults>"#;
    let mut kept = engine();
    kept.lay_out_bytes(Arc::from(package_styled(&body, None)))
        .unwrap()
        .unwrap();
    let styled = package_styled(&body, Some(large));
    let again = kept.lay_out_bytes(Arc::from(styled.clone())).unwrap().unwrap();
    assert!(!again.relaid(), "laid out whole: {:?}", kept.stats());
    let cause = again.relaid_cause().expect("laid out whole from a version before");
    assert_eq!(
        (cause.reason, cause.part.as_deref()),
        ("styles", Some("word/styles.xml"))
    );
    let mut fresh = engine();
    let whole = fresh.lay_out_bytes(Arc::from(styled)).unwrap().unwrap();
    assert_eq!(again.page_count(), whole.page_count());
    assert_eq!(svgs(&mut kept, &again), svgs(&mut fresh, &whole));
}

#[test]
fn a_tables_grid_mismatch_and_a_cached_field_are_said_as_docx2svg_says_them() {
    let row = |cells: usize| {
        format!(
            "<w:tr>{}</w:tr>",
            "<w:tc><w:p><w:r><w:t>x</w:t></w:r></w:p></w:tc>".repeat(cells)
        )
    };
    let body = format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>{}{}</w:tbl>{}"#,
        row(2),
        row(3),
        paragraphs(2, None)
    );
    let version = engine().lay_out_bytes(Arc::from(package(&body))).unwrap().unwrap();
    let said = &version.said().warnings;
    let grid = said
        .iter()
        .find(|w| w.code == "table-grid-mismatch")
        .expect("the mismatch said");
    assert_eq!(grid.page, Some(1));
    assert!(grid
        .message
        .starts_with("w:body/w:tbl[1]: rows off its w:tblGrid of 2 column(s) (w:tr[2] reaches 3"));
    assert!(version.relaid_cause().is_none());
}

/// A rectangle floating behind its paragraph: `fill` its `a:spPr` fill, `extra` more of it.
fn floating_shape(fill: &str, extra: &str) -> String {
    format!(
        r#"<w:p><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" simplePos="0" relativeHeight="1" behindDoc="1" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="914400" cy="457200"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapNone/><wp:docPr id="1" name="Shape 1"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="457200"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom>{fill}{extra}</wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r><w:r><w:t>beside a shape</w:t></w:r></w:p>"#
    )
}

#[test]
fn a_floating_shape_is_painted_in_rust_behind_the_text() {
    let body = floating_shape(r#"<a:solidFill><a:srgbClr val="4472c4"/></a:solidFill>"#, "");
    let mut engine = engine();
    let version = engine.lay_out_bytes(Arc::from(package(&body))).unwrap().unwrap();
    let (svg, missing) = engine.page_svg(&version, 0).unwrap();
    assert!(missing.is_empty(), "{missing:?}");
    let group = svg
        .split('\n')
        .position(|l| l.contains(r#"data-docx-float="behind""#))
        .expect("a float group");
    let text = svg.split('\n').position(|l| l.contains("<text")).expect("text");
    assert!(group < text, "behind the text");
    assert!(svg.contains(r##"fill="#4472C4""##), "{svg}");
    assert!(version.said().warnings.is_empty());
}

#[test]
fn what_a_painter_cannot_draw_is_said_as_docx2svg_says_it() {
    let fill = r#"<a:solidFill><a:schemeClr val="accent1"/></a:solidFill>"#;
    let body = floating_shape(fill, "<a:effectLst><a:reflection/></a:effectLst>");
    let mut engine = engine();
    let version = engine.lay_out_bytes(Arc::from(package(&body))).unwrap().unwrap();
    let said: Vec<(String, String)> = version
        .said()
        .warnings
        .iter()
        .map(|w| (w.code.clone(), w.message.clone()))
        .collect();
    let path = "w:body/w:p[1]/w:r[1]/wp:anchor[1]";
    assert_eq!(
        said,
        vec![
            (
                "drawing-not-drawn".to_owned(),
                format!("a floating drawing's effect reflection is not drawn faithfully ({path})")
            ),
            (
                "drawing-colour-unknown".to_owned(),
                format!("a floating drawing's accent1 is not drawn faithfully ({path})")
            ),
        ]
    );
}

#[test]
fn smartart_falls_back_with_its_reason() {
    let body = r#"<w:p><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="914400" cy="457200"/><wp:docPr id="1" name="Diagram 1"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"><dgm:relIds xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:dm="rId9" r:lo="rId10" r:qs="rId11" r:cs="rId12"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#;
    let fallback = engine().lay_out_bytes(Arc::from(package(body))).unwrap().unwrap_err();
    assert_eq!(fallback.reason, support::SMARTART);
}
