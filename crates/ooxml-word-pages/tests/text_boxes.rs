//! Text boxes the Rust layout lays out and paints itself (lane P5n): a chart inline in a
//! text box (docx-agent's `generated/charts/chart-places.docx`) is laid out, not sent back,
//! and painted inside the box's drawing, after the box's own paint, as docx2svg draws the
//! floats of a text box's scratch page; a text box holding a drawing with text of its own
//! is still sent back. A check's blank at a page's foot is measured with what the facts
//! leave out -- the lines its text boxes and footnotes draw -- for a version and a trial
//! alike (lane P5p).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ooxml_derive::CacheBudget;
use ooxml_fonts::{Folder, Location, WordFontOptions};
use ooxml_word_pages::layout_facts::{room, BeyondFacts};
use ooxml_word_pages::{support, DiskFonts, EngineBudgets, EngineOptions, FontSetup, Laid, PageEngine};
use std::sync::Arc;

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

/// The open faces the repository ships, with Word's substitutes and fallbacks (a session's).
fn engine() -> PageEngine {
    let folder = Folder {
        location: Location::Folder,
        path: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../python/pptx2svg/packages/pptx2svg-fonts/src/pptx2svg_fonts/files"
        )
        .to_owned(),
    };
    let options = WordFontOptions::default();
    let setup = Arc::new(FontSetup::new(Arc::new(DiskFonts), &[folder], options));
    let budgets = EngineBudgets {
        held: Arc::new(Unlimited),
        paint: Arc::new(Unlimited),
    };
    PageEngine::new(setup, EngineOptions::default(), budgets)
}

fn fixture() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/ooxml-oracle/fixtures/generated/charts/chart-places.docx"
    ))
    .unwrap()
}

/// The `<g …>…</g>` group of the drawing at `path` in `svg` (groups nest only as the
/// drawing's own paint nests them).
fn group<'s>(svg: &'s str, path: &str) -> &'s str {
    let start = svg
        .find(&format!(r#"<g data-docx-path="{path}""#))
        .unwrap_or_else(|| panic!("no drawing at {path}"));
    let mut depth = 0usize;
    let mut at = start;
    while let Some(k) = svg[at..].find(['<']) {
        at += k;
        let rest = &svg[at..];
        if rest.starts_with("<g ") || rest.starts_with("<g>") {
            depth += 1;
        } else if rest.starts_with("</g>") {
            depth -= 1;
            if depth == 0 {
                return &svg[start..at + 4];
            }
        }
        at += 1;
    }
    panic!("the drawing's group does not end")
}

#[test]
fn a_chart_in_a_text_box_is_laid_out_and_painted_with_the_box() {
    let mut engine = engine();
    let version = engine
        .lay_out_bytes(Arc::from(fixture()))
        .unwrap()
        .unwrap_or_else(|fallback| panic!("falls back: {} at {}", fallback.reason, fallback.at));
    let (svg, missing) = engine.page_svg(&version, 0).unwrap();
    assert!(missing.is_empty(), "{missing:?}");
    let boxed = group(&svg, "w:body/w:p[2]/w:r[2]/wp:anchor[1]");
    // The box's text, and the chart's paint after the box's own, inside the box's group.
    assert!(boxed.contains(">A chart in a text box:<"), "{boxed}");
    let text_at = boxed.find(">A chart in a text box:<").unwrap();
    let chart = r#"<g data-docx-path="w:body/w:p[2]/w:r[2]/wp:anchor[1]/wps:txbx/w:txbxContent/w:p[2]/w:r[1]""#;
    let chart_at = boxed
        .find(chart)
        .unwrap_or_else(|| panic!("no chart in the box: {boxed}"));
    let shape_at = boxed.find("<path").unwrap();
    assert!(
        shape_at < chart_at && chart_at < text_at,
        "shape, chart, then text: {boxed}"
    );
}

#[test]
fn a_text_box_holding_a_drawing_with_text_is_sent_back() {
    let package = fixture();
    let mut engine = engine();
    let inner = r#"<wps:wsp><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="457200"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></wps:spPr><wps:txbx><w:txbxContent><w:p><w:r><w:t>inner</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp>"#;
    let edited = with_part_edited(&package, "word/document.xml", |xml| {
        let chart_start = xml
            .find(r#"<a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">"#)
            .unwrap();
        let chart_end = chart_start + xml[chart_start..].find("</a:graphicData>").unwrap() + "</a:graphicData>".len();
        format!(
            r#"{}<a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">{inner}</a:graphicData>{}"#,
            &xml[..chart_start],
            &xml[chart_end..]
        )
    });
    let fallback = engine.lay_out_bytes(Arc::from(edited)).unwrap().unwrap_err();
    assert_eq!(fallback.reason, support::TEXT_BOX);
}

struct Never;

impl ooxml_base::Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

#[test]
fn a_page_measured_with_its_text_boxes_and_footnotes_for_a_version_and_a_trial() {
    let mut engine = engine();
    let bytes = fixture();
    let version = engine.lay_out_bytes(Arc::from(bytes.clone())).unwrap().unwrap();
    let beyond = engine.beyond_facts(Laid::Version(&version), 0);
    let page = &version.facts().pages[0];
    let body = page.lines.iter().map(|l| l.top + l.pitch).max().unwrap();
    // The box's lines are drawn (above the body's last here); the footnotes below it.
    let foot = beyond.text_boxes.expect("the box's lines");
    assert!(foot > page.frame.top && foot < body);
    let notes = beyond.notes.expect("the footnotes' lines");
    assert!(notes > body && notes < page.frame.bottom);
    // docx-agent's blank: from the body's last line to the footnote separator.
    assert_eq!(room(page, &beyond), (155.1, 0.3));
    assert_eq!(room(page, &BeyondFacts::default()), (268.1, 0.42));
    // A page whose paragraphs hold no text box is not painted for it.
    let last = version.page_count() - 1;
    assert_eq!(engine.beyond_facts(Laid::Version(&version), last).text_boxes, None);
    // A trial laid out from the version measures the same.
    let package = ooxml_word_model::WordPackage::open(
        ooxml_opc::Package::open(Arc::from(bytes), &ooxml_base::Limits::default()).unwrap(),
    )
    .0;
    let reading = ooxml_word_model::docx::Docx::open_after(&package, engine.given_reading().unwrap()).unwrap();
    let trial = engine.trial(&Arc::new(reading), &version, &Never).unwrap().unwrap();
    assert_eq!(engine.beyond_facts(Laid::Trial(&trial), 0), beyond);
}

/// `package` with part `name`'s text made by `edit`.
fn with_part_edited(package: &[u8], name: &str, edit: impl Fn(&str) -> String) -> Vec<u8> {
    let opened = ooxml_opc::Package::open(Arc::from(package.to_vec()), &ooxml_base::Limits::default()).unwrap();
    let xml = String::from_utf8(opened.read(name).unwrap().unwrap().to_vec()).unwrap();
    opened
        .with_bytes(name, Arc::from(edit(&xml).into_bytes()))
        .save()
        .unwrap()
}
