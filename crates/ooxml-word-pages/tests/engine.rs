//! The page engine over made-up packages: a version read again from its bytes keeps the
//! parts whose stored bytes did not change and is laid out from the version before; a
//! version given as its touched parts is the version given whole; a document the layout
//! does not lay out yet is a fallback with its reason. The faces are the open ones the
//! repository ships.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ooxml_derive::CacheBudget;
use ooxml_fonts::{Folder, Location, WordFontOptions};
use ooxml_word_layout::paint::svg::NoDrawings;
use ooxml_word_pages::{support, DiskFonts, EngineBudgets, EngineOptions, FontSetup, PageEngine};
use ooxml_zip::{Method, NewEntry, Writer};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Grants every reservation and counts what is held.
#[derive(Default)]
struct Counted(AtomicUsize);

impl CacheBudget for Counted {
    fn reserve(&self, bytes: usize) -> bool {
        self.0.fetch_add(bytes, Ordering::SeqCst);
        true
    }
    fn release(&self, bytes: usize) {
        self.0.fetch_sub(bytes, Ordering::SeqCst);
    }
    fn used(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

/// Grants nothing.
struct Nothing;

impl CacheBudget for Nothing {
    fn reserve(&self, _bytes: usize) -> bool {
        false
    }
    fn release(&self, _bytes: usize) {}
    fn used(&self) -> usize {
        0
    }
}

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
    let entry = |name: &str| NewEntry {
        name: name.into(),
        method: Method::Deflated,
        dos_time: 0x6000,
        dos_date: 0x5a21,
        external_attr: 0,
        create_system: 0,
    };
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let styles_rel = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let files = [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#.to_owned(),
        ),
        ("_rels/.rels", rels.to_owned()),
        ("word/document.xml", document_xml(body)),
        ("word/_rels/document.xml.rels", styles_rel.to_owned()),
        (
            "word/styles.xml",
            format!(r#"<w:styles {W}>{}</w:styles>"#, styles.unwrap_or_default()),
        ),
    ];
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
    engine_in(EngineBudgets {
        held: Arc::new(Unlimited),
        paint: Arc::new(Unlimited),
    })
}

fn engine_in(budgets: EngineBudgets) -> PageEngine {
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
    PageEngine::new(setup, EngineOptions::default(), budgets)
}

fn svgs(engine: &mut PageEngine, version: &ooxml_word_pages::LaidVersion) -> Vec<String> {
    (0..version.page_count())
        .map(|k| engine.page_svg(version, k, &NoDrawings).unwrap().0)
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

#[test]
fn a_kept_version_holds_what_it_changed_and_not_the_document_part() {
    let mut kept = engine();
    let mut versions = vec![kept
        .lay_out_bytes(Arc::from(package(&paragraphs(200, None))))
        .unwrap()
        .unwrap()];
    let first = versions[0].weight();
    for k in 0..6 {
        let next = kept
            .lay_out_bytes(Arc::from(package(&paragraphs(200, Some(20 + 30 * k)))))
            .unwrap()
            .unwrap();
        assert!(next.relaid());
        // Laid out from the version before: its own containers and the one block it changed.
        assert!(next.weight() * 4 < first, "{} of {first}", next.weight());
        let pictures = next.pictures().unwrap().package();
        assert!(
            !pictures.has_part("word/document.xml"),
            "the text is the version's model, not a tree"
        );
        assert!(pictures.has_part("word/_rels/document.xml.rels"));
        versions.push(next);
    }
}

#[test]
fn the_memos_hold_what_the_versions_in_use_need_however_many_versions() {
    let held = Arc::new(Counted::default());
    let mut kept = engine_in(EngineBudgets {
        held: Arc::clone(&held) as Arc<dyn CacheBudget>,
        paint: Arc::new(Unlimited),
    });
    kept.lay_out_bytes(Arc::from(package(&paragraphs(150, None))))
        .unwrap()
        .unwrap();
    let after_first = held.used();
    for k in 0..40 {
        // Each version changes another paragraph and drops the one before.
        let edited = package(&paragraphs(150, Some(k * 3)));
        kept.lay_out_bytes(Arc::from(edited)).unwrap().unwrap();
    }
    let after = held.used();
    assert!(
        after < after_first + after_first / 10,
        "memos grew from {after_first} to {after} bytes over 40 versions"
    );
    assert!(kept.stats().swept > 0);
}

#[test]
fn a_page_painted_is_kept_within_the_paint_budget() {
    let mut kept = engine();
    let version = kept
        .lay_out_bytes(Arc::from(package(&paragraphs(80, None))))
        .unwrap()
        .unwrap();
    let once = kept.paint(&version, 0).unwrap();
    let again = kept.paint(&version, 0).unwrap();
    assert!(Arc::ptr_eq(&once, &again));
    assert_eq!((kept.stats().pages_painted, kept.stats().paint_hits), (1, 1));
    let mut none = engine_in(EngineBudgets {
        held: Arc::new(Unlimited),
        paint: Arc::new(Nothing),
    });
    let version = none
        .lay_out_bytes(Arc::from(package(&paragraphs(80, None))))
        .unwrap()
        .unwrap();
    let once = none.paint(&version, 0).unwrap();
    let again = none.paint(&version, 0).unwrap();
    assert_eq!(*once, *again, "painted again, the same paint");
    assert_eq!((none.stats().pages_painted, none.stats().paint_hits), (2, 0));
}
