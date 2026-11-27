//! The page engine over made-up packages: a version read again from its bytes keeps the
//! parts whose stored bytes did not change and is laid out from the version before; a
//! version given as its touched parts is the version given whole; a document the layout
//! does not lay out yet is a fallback with its reason; a layout stopped lays nothing out
//! and the next is laid out from the version before; a trial is laid out from a version
//! and kept nowhere; columns with a line between them,
//! line numbers and a drop cap are laid out and painted in Rust; a floating shape is
//! painted in Rust and said of as docx2svg says it. The faces are the open ones the repository ships.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ooxml_derive::CacheBudget;
use ooxml_fonts::{Folder, Location, WordFontOptions};
use ooxml_word_model::ids::{
    DocumentId, PartsDelta, StoryChange, StoryDelta, StoryIx, Sub, VersionDelta, VersionRef, VersionSpan,
};
use ooxml_word_pages::{support, DiskFonts, EngineBudgets, EngineOptions, FontSetup, LaidVersion, PageEngine};
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
    engine_in(EngineBudgets {
        held: Arc::new(Unlimited),
        paint: Arc::new(Unlimited),
    })
}

fn engine_in(budgets: EngineBudgets) -> PageEngine {
    engine_with(budgets, false)
}

/// [`engine_in`], a face the folder lacks laid out with a fallback where `fallbacks`.
fn engine_with(budgets: EngineBudgets, fallbacks: bool) -> PageEngine {
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
        fallbacks,
    };
    let setup = Arc::new(FontSetup::new(Arc::new(DiskFonts), &[folder], options));
    PageEngine::new(setup, EngineOptions::default(), budgets)
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

/// What a new engine says it did: nothing.
fn engine_fresh_stats() -> ooxml_word_pages::EngineStats {
    engine().stats()
}

#[test]
fn a_poisoned_engine_is_made_again_from_its_seed() {
    // R2 #11: an engine a panic left mid-layout is not reused; the next lock gets one made
    // again from its seed (nothing laid out), which lays the version out whole, as before
    let bytes = package(&paragraphs(40, None));
    let engine = std::sync::Mutex::new(engine());
    let first = {
        let mut held = ooxml_word_pages::lock_engine(&engine);
        held.lay_out_bytes(Arc::from(bytes.clone())).unwrap().unwrap()
    };
    let fresh = format!("{:?}", engine_fresh_stats());
    assert_ne!(format!("{:?}", ooxml_word_pages::lock_engine(&engine).stats()), fresh);
    let poisoned = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let _held = engine.lock().unwrap();
                panic!("a panic while the engine is held");
            })
            .join()
    });
    assert!(poisoned.is_err() && engine.is_poisoned());
    let mut held = ooxml_word_pages::lock_engine(&engine);
    assert!(!engine.is_poisoned());
    // made again: nothing laid out yet, as a new engine
    assert_eq!(format!("{:?}", held.stats()), fresh);
    let again = held.lay_out_bytes(Arc::from(bytes)).unwrap().unwrap();
    assert!(!again.relaid());
    assert_eq!(again.facts(), first.facts());
}

#[test]
fn facts_are_made_when_asked_never_while_laying_out_from_the_last_version_whose_were() {
    let mut kept = engine();
    let first = kept
        .lay_out_bytes(Arc::from(package(&paragraphs(120, None))))
        .unwrap()
        .unwrap();
    assert!(!first.has_facts(), "laying out makes no facts");
    let _ = first.facts();
    // Two edits, neither version asked: the second's facts are made from the first's.
    let second = kept
        .lay_out_bytes(Arc::from(package(&paragraphs(120, Some(60)))))
        .unwrap()
        .unwrap();
    let edited = package(&paragraphs(120, Some(90)));
    let third = kept.lay_out_bytes(Arc::from(edited.clone())).unwrap().unwrap();
    assert!(!second.has_facts() && !third.has_facts());
    let whole = engine().lay_out_bytes(Arc::from(edited)).unwrap().unwrap();
    assert_eq!(third.facts(), whole.facts());
    assert!(!second.has_facts());
    assert_eq!(second.facts().pages.len(), whole.facts().pages.len());
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

/// Says stop from its `n`-th question on.
struct After(AtomicUsize);

impl ooxml_base::Cancel for After {
    fn is_cancelled(&self) -> bool {
        let left = self.0.load(Ordering::SeqCst);
        if left == 0 {
            return true;
        }
        self.0.store(left - 1, Ordering::SeqCst);
        false
    }
}

fn after(n: usize) -> After {
    After(AtomicUsize::new(n))
}

#[test]
fn a_layout_stopped_lays_nothing_out_and_the_next_is_laid_out_from_the_one_before() {
    let opened = |bytes: Vec<u8>| {
        ooxml_word_model::WordPackage::open(
            ooxml_opc::Package::open(Arc::from(bytes), &ooxml_base::Limits::default()).unwrap(),
        )
        .0
    };
    let mut kept = engine();
    let first = kept
        .lay_out(&opened(package(&paragraphs(120, None))), None)
        .unwrap()
        .unwrap();
    let edited = opened(package(&paragraphs(120, Some(60))));
    for n in [0, 1] {
        // stopped before its first block, and after it
        let stopped = kept.lay_out_until(&edited, &after(n));
        assert!(
            matches!(stopped, Err(ooxml_word_pages::PagesError::Stopped)),
            "after {n}"
        );
        assert!(
            Arc::ptr_eq(kept.last().unwrap(), &first),
            "the version before is still the last"
        );
    }
    assert_eq!(kept.stats().stopped, 2);
    let second = kept.lay_out_until(&edited, &after(usize::MAX)).unwrap().unwrap();
    assert!(second.relaid(), "laid out from the version before");
    // the version laid out last is answered, nothing laid out to stop
    let again = kept.lay_out_until(&edited, &after(0)).unwrap().unwrap();
    assert!(Arc::ptr_eq(&again, &second));
    // asked once per block and once per page: stopped at its last page
    let asked = after(usize::MAX);
    engine().lay_out_until(&edited, &asked).unwrap().unwrap();
    let asks = usize::MAX - asked.0.load(Ordering::SeqCst);
    assert!(asks > 120, "{asks} asks");
    let mut fresh = engine();
    assert!(matches!(
        fresh.lay_out_until(&edited, &after(asks - 1)),
        Err(ooxml_word_pages::PagesError::Stopped)
    ));
    let whole = fresh.lay_out(&edited, None).unwrap().unwrap();
    assert!(!whole.relaid());
    assert_eq!(svgs(&mut kept, &second), svgs(&mut fresh, &whole));
    assert_eq!(second.facts(), whole.facts());
    let summary = fresh.summary(&whole).unwrap();
    assert_eq!(summary.pages.len(), whole.page_count());
    assert_eq!(summary.pages, kept.summary(&second).unwrap().pages);
}

/// A delta of the main part alone, between the versions `base` and `version` of `document`.
fn body_delta(document: DocumentId, base: u64, version: u64) -> VersionDelta {
    let at = |seq| VersionRef { document, seq };
    VersionDelta {
        span: None,
        parts: PartsDelta::default(),
        stories: vec![StoryChange {
            story: StoryIx::BODY,
            part: "word/document.xml".to_owned(),
            delta: StoryDelta::Whole,
        }],
    }
    .spanning(VersionSpan {
        base: at(base),
        version: at(version),
    })
}

#[test]
fn a_delta_is_taken_for_the_change_only_from_the_version_given_last() {
    let opened = |bytes: Vec<u8>| {
        ooxml_word_model::WordPackage::open(
            ooxml_opc::Package::open(Arc::from(bytes), &ooxml_base::Limits::default()).unwrap(),
        )
        .0
    };
    let (document, other) = (DocumentId::fresh(), DocumentId::fresh());
    let versions: Vec<_> = (0..6).map(|k| opened(package(&paragraphs(60, Some(10 + k))))).collect();
    let mut kept = engine();
    kept.lay_out(&versions[0], None).unwrap().unwrap();
    // the version given last was named by no delta: the packages are compared
    kept.lay_out(&versions[1], Some(&body_delta(document, 0, 1)))
        .unwrap()
        .unwrap();
    assert_eq!(kept.stats().changes_from, (0, 1));
    // its base is the version given last: the delta says what changed
    let taken = kept
        .lay_out(&versions[2], Some(&body_delta(document, 1, 2)))
        .unwrap()
        .unwrap();
    assert_eq!(kept.stats().changes_from, (1, 1));
    // its base is not the version given last (a version before it)
    let skipped = kept
        .lay_out(&versions[3], Some(&body_delta(document, 1, 3)))
        .unwrap()
        .unwrap();
    assert_eq!(kept.stats().changes_from, (1, 2));
    // another document's numbers never match: the same number of another document
    kept.lay_out(&versions[4], Some(&body_delta(other, 3, 4)))
        .unwrap()
        .unwrap();
    assert_eq!(kept.stats().changes_from, (1, 3));
    // a version given with no delta names none: the next delta's base cannot be it
    kept.lay_out(&versions[5], None).unwrap().unwrap();
    kept.lay_out(&versions[4], Some(&body_delta(other, 5, 6)))
        .unwrap()
        .unwrap();
    assert_eq!(kept.stats().changes_from, (1, 5));
    // laid out alike either way: each as a whole layout lays it out
    for (version, k) in [(&taken, 2), (&skipped, 3)] {
        let whole = engine().lay_out(&versions[k], None).unwrap().unwrap();
        assert_eq!(version.facts(), whole.facts(), "version {k}");
    }
}

#[test]
fn a_trial_is_laid_out_from_the_version_checked_and_kept_nowhere() {
    let opened = |bytes: Vec<u8>| {
        ooxml_word_model::WordPackage::open(
            ooxml_opc::Package::open(Arc::from(bytes), &ooxml_base::Limits::default()).unwrap(),
        )
        .0
    };
    let mut kept = engine();
    let first = kept
        .lay_out(&opened(package(&paragraphs(120, None))), None)
        .unwrap()
        .unwrap();
    let edited = opened(package(&paragraphs(120, Some(60))));
    let trial = kept.trial(&edited, &first, &after(usize::MAX)).unwrap().unwrap();
    assert!(
        Arc::ptr_eq(kept.last().unwrap(), &first),
        "the version checked is still the last"
    );
    assert_eq!(kept.stats().trials, 1);
    let mut fresh = engine();
    let whole = fresh.lay_out(&edited, None).unwrap().unwrap();
    assert_eq!(
        trial.facts(),
        whole.facts(),
        "page for page the layout of those bytes alone"
    );
    // stopped: nothing laid out, the last version kept
    assert!(matches!(
        kept.trial(&edited, &first, &after(0)),
        Err(ooxml_word_pages::PagesError::Stopped)
    ));
    assert!(Arc::ptr_eq(kept.last().unwrap(), &first));
    // the next version is laid out from the one the engine had
    let second = kept.lay_out(&edited, None).unwrap().unwrap();
    assert!(second.relaid());
    assert_eq!(second.facts(), whole.facts());
}

#[test]
fn a_version_with_the_last_ones_parts_is_the_last_version() {
    let mut kept = engine();
    let bytes: Arc<[u8]> = Arc::from(package(&paragraphs(40, None)));
    let opened =
        ooxml_word_model::WordPackage::open(ooxml_opc::Package::open(bytes, &ooxml_base::Limits::default()).unwrap()).0;
    let first = kept.lay_out(&opened, None).unwrap().unwrap();
    let again = kept.lay_out(&opened.clone(), None).unwrap().unwrap();
    assert!(Arc::ptr_eq(&first, &again));
    let stats = kept.stats();
    assert_eq!((stats.kept, stats.full, stats.relaid), (1, 1, 0), "{stats:?}");
    let touched: Arc<[u8]> = Arc::from(document_xml(&paragraphs(40, Some(3))).into_bytes());
    let edited = read_after(opened.with_bytes("word/document.xml", touched), &opened);
    let next = kept.lay_out(&edited, None).unwrap().unwrap();
    assert!(!Arc::ptr_eq(&first, &next));
    assert_eq!(kept.stats().kept, 1);
    // The version before is no longer the last: it is laid out again, not kept.
    let back = kept.lay_out(&opened, None).unwrap().unwrap();
    assert!(!Arc::ptr_eq(&first, &back));
    assert_eq!(back.facts(), first.facts());
}

#[test]
fn a_floating_table_is_laid_out_in_rust() {
    let body = format!(
        r#"<w:tbl><w:tblPr><w:tblpPr w:tblpX="500" w:tblpY="500"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>{}"#,
        paragraphs(2, None)
    );
    let laid = engine().lay_out_bytes(Arc::from(package(&body))).unwrap();
    assert!(laid.is_ok(), "{:?}", laid.err());
}

#[test]
fn a_drop_cap_in_a_cell_falls_back_with_its_reason() {
    let body = format!(
        r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:p><w:pPr><w:framePr w:dropCap="drop" w:lines="2" w:wrap="around" w:vAnchor="text" w:hAnchor="text"/></w:pPr><w:r><w:t>D</w:t></w:r></w:p><w:p><w:r><w:t>rop</w:t></w:r></w:p></w:tc></w:tr></w:tbl>{}"#,
        paragraphs(2, None)
    );
    let fallback = engine().lay_out_bytes(Arc::from(package(&body))).unwrap().unwrap_err();
    assert_eq!(fallback.reason, support::DROP_CAP);
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
        .map(|l| (l.name.anchor(), l.baseline))
        .collect();
    let facts: Vec<_> = story_lines.iter().map(|l| (l.line.anchor, l.line.baseline)).collect();
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
fn line_numbers_and_column_separators_are_anchored_on_their_paragraph_and_their_section() {
    // section 1 numbers its lines and ends in its last paragraph's mark; section 2 (the
    // body's own `w:sectPr`) has two columns with a line between them
    let numbered = r#"<w:p><w:pPr><w:sectPr><w:lnNumType w:countBy="1"/></w:sectPr></w:pPr><w:r><w:t>end of one</w:t></w:r></w:p>"#;
    let body = format!(
        r#"{}{numbered}{}<w:sectPr><w:cols w:num="2" w:space="720" w:sep="1"/></w:sectPr>"#,
        paragraphs(3, None),
        paragraphs(120, None)
    );
    let mut kept = engine();
    let version = kept.lay_out_bytes(Arc::from(package(&body))).unwrap().unwrap();
    let paint = kept.paint(&version, 0).unwrap();
    let numbers: Vec<&ooxml_word_layout::paint::Span> = paint
        .lines
        .iter()
        .flat_map(|l| &l.spans)
        .filter(|s| s.name.anchor().is_some_and(|a| a.sub == Sub::LineNumbers(0)))
        .collect();
    assert_eq!(numbers.len(), 4, "every line of section 1 numbered");
    let names = &version.layout().document.names;
    assert!(numbers.iter().all(|s| s.name.named(names) == "w:sectPr[1]/w:lnNumType"));
    let mut separators = 0;
    for k in 0..version.page_count() {
        let page = kept.paint(&version, k).unwrap();
        for rule in page.rules.iter().filter(|r| r.kind == "column-separator") {
            let anchor = rule.name.anchor().expect("anchored on the body's last block");
            assert_eq!(anchor.sub, Sub::Cols(1));
            assert_eq!(
                version.layout().document.names.path(anchor).as_deref(),
                Some("w:sectPr[2]/w:cols")
            );
            separators += 1;
        }
    }
    assert!(separators > 0, "{} pages", version.page_count());
    // the SVG writes them under the same names (asserted against their paths in debug)
    let (svg, _) = kept.page_svg(&version, 0).unwrap();
    assert!(svg.contains(r#"data-docx-path="w:sectPr[1]/w:lnNumType""#));
}

/// `n` paragraphs and a table of `rows` rows after them, each paragraph and row given a
/// paraId where `stamped` (the paths the same either way).
fn stampable(n: usize, rows: usize, stamped: bool) -> String {
    const W14: &str = r#"xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#;
    let id = |k: usize| {
        if stamped {
            format!(r#" {W14} w14:paraId="{:08X}""#, 0x1000_0000 + k)
        } else {
            String::new()
        }
    };
    let mut out: String = (0..n)
        .map(|k| {
            format!(
                "<w:p{}><w:r><w:t>{k} some words on a line of the body</w:t></w:r></w:p>",
                id(k)
            )
        })
        .collect();
    out.push_str("<w:tbl><w:tblGrid><w:gridCol w:w=\"4000\"/></w:tblGrid>");
    for r in 0..rows {
        out.push_str(&format!(
            "<w:tr{}><w:tc><w:tcPr><w:tcBorders><w:top w:val=\"single\" w:sz=\"4\"/></w:tcBorders></w:tcPr><w:p{}><w:r><w:t>row {r}</w:t></w:r></w:p></w:tc></w:tr>",
            id(10_000 + r),
            id(20_000 + r)
        ));
    }
    out.push_str(&format!("</w:tbl><w:p{}/>", id(30_000)));
    out
}

#[test]
fn a_first_edit_that_stamps_an_unstamped_document_names_every_block_by_its_new_key() {
    // the version before names its blocks by ordinal keys; the next, the same paths
    // stamped, by durable ones: nothing kept from the version before may name a block by
    // a key it no longer has (the paint's names are asserted against their paths in a
    // debug build; a page tier carrying anchors must be keyed by them -- LK2)
    let mut kept = engine();
    let before = kept
        .lay_out_bytes(Arc::from(package(&stampable(80, 30, false))))
        .unwrap()
        .unwrap();
    for k in 0..before.page_count() {
        kept.paint(&before, k).unwrap();
        kept.page_svg(&before, k).unwrap();
        kept.page_read(&before, k).unwrap();
    }
    let _ = before.facts();
    kept.summary(&before).unwrap();
    kept.body_ids(&before).unwrap();
    let after = kept
        .lay_out_bytes(Arc::from(package(&stampable(80, 30, true))))
        .unwrap()
        .unwrap();
    assert_eq!(after.page_count(), before.page_count());
    let names = &after.layout().document.names;
    let (mut lines, mut rules) = (0, 0);
    for k in 0..after.page_count() {
        let paint = kept.paint(&after, k).unwrap();
        for line in &paint.lines {
            let anchor = line.name.anchor().expect("a body line is anchored");
            assert!(anchor.block.durable(), "{anchor:?}");
            let path = names.path(anchor).expect("named");
            assert_eq!(names.id(anchor.block).map(|id| id.to_string()), after.id_of(&path));
            lines += 1;
        }
        for rule in paint.rules.iter().filter(|r| r.kind == "table-border") {
            let anchor = rule.name.anchor().expect("a cell's rule is anchored");
            assert!(anchor.block.durable());
            assert!(names.path(anchor).is_some());
            rules += 1;
        }
        kept.page_svg(&after, k).unwrap();
        // what a check read of the page names every block by its new key
        let read = kept.page_read(&after, k).unwrap();
        for text in &read.read().texts {
            assert!(text.place.key().is_some_and(|key| key.durable()), "{:?}", text.place);
        }
        for rule in &read.read().boxes.rules {
            assert!(rule.row.is_none_or(|row| row.table.durable()));
        }
    }
    assert!(lines > 100 && rules > 10, "{lines} lines, {rules} rules");
    summary_names_by_ids(&mut kept, &after);
    facts_name_by_new_keys(&mut kept, &before, &after);
}

/// `after`'s facts (a first edit that stamped `before`'s blocks) and the layout half of its
/// page tier name every block by its new key: the facts are a whole layout's, none kept from
/// `before` by a key it no longer has.
fn facts_name_by_new_keys(kept: &mut PageEngine, before: &LaidVersion, after: &LaidVersion) {
    let names = &after.layout().document.names;
    for page in &after.facts().pages {
        for line in &page.lines {
            assert!(line.anchor.is_some_and(|a| a.block.durable()), "{line:?}");
        }
        for block in &page.blocks {
            assert!(block.block.durable());
            assert!(names.path(*block).is_some(), "{block:?}");
        }
        // the layout half of the page tier: its foot by the new keys
        for key in kept.laid_entry(after, page.page).unwrap().foot().keys() {
            assert!(key.durable(), "{key:?}");
        }
    }
    // the facts by key are a whole layout's: none kept from the version before by a key it
    // no longer has
    let whole = engine()
        .lay_out_bytes(Arc::from(package(&stampable(80, 30, true))))
        .unwrap()
        .unwrap();
    assert_eq!(after.facts(), whole.facts());
    for (page, old) in after.facts().pages.iter().zip(&before.facts().pages) {
        assert!(!Arc::ptr_eq(page, old), "page {} kept by the ordinal keys", page.page);
    }
}

/// Every page of `version`'s summary and body ids (of a body with no header, footer or note)
/// names its lines by the version's ids, never by a version before's (its paint's anchors
/// named through its names).
fn summary_names_by_ids(kept: &mut PageEngine, version: &LaidVersion) {
    let names = &version.layout().document.names;
    let ids_of = |kept: &mut PageEngine, k: usize| -> Vec<Option<String>> {
        let paint = kept.paint(version, k).unwrap();
        paint
            .lines
            .iter()
            .map(|l| l.name.anchor().and_then(|a| names.id(a.block)).map(|id| id.to_string()))
            .collect()
    };
    let summary = kept.summary(version).unwrap();
    for (k, page) in summary.pages.iter().enumerate() {
        let said: Vec<Option<String>> = page.lines.iter().map(|l| l.id.as_deref().map(str::to_owned)).collect();
        assert_eq!(said, ids_of(kept, k), "page {k}");
    }
    for (k, page) in kept.body_ids(version).unwrap().iter().enumerate() {
        assert_eq!(*page, ids_of(kept, k), "page {k}");
    }
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
        // Laid out from the version before: its own containers and the one block it changed
        // (every measure it shares with the engine is not its own).
        assert!(next.weight() <= first + first / 4, "{} of {first}", next.weight());
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
fn smartart_with_no_data_model_is_a_stand_in_said_as_docx2svg_says_it() {
    let body = r#"<w:p><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="914400" cy="457200"/><wp:docPr id="1" name="Diagram 1"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"><dgm:relIds xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:dm="rId9" r:lo="rId10" r:qs="rId11" r:cs="rId12"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#;
    let version = engine().lay_out_bytes(Arc::from(package(body))).unwrap().unwrap();
    let codes: Vec<&str> = version.said().warnings.iter().map(|w| w.code.as_str()).collect();
    assert_eq!(codes, ["diagram-unreadable", "drawing-not-drawn"]);
}

const VML: &str = r#"xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office""#;

#[test]
fn a_vml_rectangle_is_painted_as_its_preset() {
    let body = format!(
        r##"<w:p><w:r><w:pict {VML}><v:rect style="position:absolute;width:100pt;height:50pt" fillcolor="#FF0000" strokecolor="blue" strokeweight="2pt"/></w:pict></w:r><w:r><w:t>text</w:t></w:r></w:p>"##
    );
    let mut engine = engine();
    let version = engine.lay_out_bytes(Arc::from(package(&body))).unwrap().unwrap();
    let (svg, missing) = engine.page_svg(&version, 0).unwrap();
    assert!(missing.is_empty(), "{missing:?}");
    assert!(svg.contains(r##"fill="#FF0000""##), "{svg}");
    assert!(svg.contains(r##"stroke="#0000FF""##), "{svg}");
    assert!(version.said().warnings.is_empty());
}

#[test]
fn a_watermark_text_path_is_drawn_in_its_box() {
    let body = format!(
        r##"<w:p><w:r><w:pict {VML}><v:shapetype id="_x0000_t136" coordsize="21600,21600" o:spt="136"><v:textpath on="t" fitshape="t"/></v:shapetype><v:shape id="Made1" type="#_x0000_t136" style="position:absolute;width:400pt;height:150pt;rotation:315" fillcolor="silver" stroked="f"><v:fill opacity=".5"/><v:textpath style="font-family:&quot;Carlito&quot;;font-size:1pt" string="DRAFT"/></v:shape></w:pict></w:r><w:r><w:t>text</w:t></w:r></w:p>"##
    );
    let mut engine = engine();
    let version = engine.lay_out_bytes(Arc::from(package(&body))).unwrap().unwrap();
    let (svg, _) = engine.page_svg(&version, 0).unwrap();
    assert!(svg.contains(">DRAFT</text>"), "{svg}");
    assert!(svg.contains(r##"fill="#C0C0C0" fill-opacity="0.5""##), "{svg}");
    assert!(svg.contains("rotate(315)"), "{svg}");
    assert!(svg.contains("matrix("), "fitted by its outlines: {svg}");
}

#[test]
fn a_text_path_its_shape_type_does_not_say_fits_is_an_approximation() {
    let body = format!(
        r##"<w:p><w:r><w:pict {VML}><v:shape id="Made1" type="#_x0000_t999" style="position:absolute;width:400pt;height:150pt" fillcolor="silver" stroked="f"><v:textpath style="font-family:&quot;Carlito&quot;" string="DRAFT"/></v:shape></w:pict></w:r><w:r><w:t>text</w:t></w:r></w:p>"##
    );
    let mut engine = engine();
    let version = engine.lay_out_bytes(Arc::from(package(&body))).unwrap().unwrap();
    let (svg, _) = engine.page_svg(&version, 0).unwrap();
    assert!(svg.contains("textLength="), "{svg}");
    let said = version.said();
    assert_eq!(said.approximations.len(), 1, "{said:?}");
    assert_eq!(said.approximations[0].code, "layout-approximate:text-path");
}

#[test]
fn a_kept_pages_warnings_are_said_again_without_walking_it() {
    let row = |cells: usize| {
        format!(
            "<w:tr>{}</w:tr>",
            "<w:tc><w:p><w:r><w:t>x</w:t></w:r></w:p></w:tc>".repeat(cells)
        )
    };
    // A table off its grid on the last page; the versions edit a paragraph on the first.
    let body = |edited: Option<usize>| {
        format!(
            r#"{}<w:tbl><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>{}{}</w:tbl>"#,
            paragraphs(160, edited),
            row(2),
            row(3)
        )
    };
    let mut kept = engine();
    kept.lay_out_bytes(Arc::from(package(&body(None)))).unwrap().unwrap();
    for k in 0..3 {
        let next = kept
            .lay_out_bytes(Arc::from(package(&body(Some(2 + k)))))
            .unwrap()
            .unwrap();
        let whole = engine()
            .lay_out_bytes(Arc::from(package(&body(Some(2 + k)))))
            .unwrap()
            .unwrap();
        assert_eq!(next.said(), whole.said(), "version {k}");
        assert!(next.said().warnings.iter().any(|w| w.code == "table-grid-mismatch"));
    }
    let (again, walked) = kept.stats().pages_said;
    assert!(
        again > walked,
        "the pages the edits did not reach are said again: {again} again, {walked} walked"
    );
}

#[test]
fn a_summary_made_from_the_one_before_is_the_one_made_whole_and_says_what_changed() {
    let mut kept = engine();
    let first = kept
        .lay_out_bytes(Arc::from(package(&paragraphs(160, None))))
        .unwrap()
        .unwrap();
    let before = kept.summary(&first).unwrap();
    let edited = package(&paragraphs(160, Some(100)));
    let second = kept.lay_out_bytes(Arc::from(edited.clone())).unwrap().unwrap();
    let after = kept.summary(&second).unwrap();
    let mut fresh = engine();
    let whole = fresh.lay_out_bytes(Arc::from(edited)).unwrap().unwrap();
    let alone = fresh.summary(&whole).unwrap();
    assert_eq!(after.pages.len(), alone.pages.len());
    for (a, b) in after.pages.iter().zip(&alone.pages) {
        assert_eq!(**a, **b);
    }
    // The pages before the edit are the very summaries of the version before.
    assert!(Arc::ptr_eq(&after.pages[0], &before.pages[0]));
    let reflow = after.compare(&before, &std::collections::HashMap::new());
    assert!(!reflow.changed.is_empty() && !reflow.changed.contains(&1), "{reflow:?}");
    assert!(
        reflow.changed.iter().all(|p| reflow.why.get(p) == Some(&"content")),
        "{reflow:?}"
    );
    assert!(after.pages[0]
        .lines
        .iter()
        .all(|l| l.id.as_deref().is_some_and(|id| id.starts_with('p'))));
}

/// A text box floating beside its paragraph, holding `text`.
fn text_box(text: &str) -> String {
    let shape = floating_shape(r#"<a:noFill/>"#, "");
    shape.replace(
        "<wps:bodyPr/>",
        &format!(r#"<wps:txbx><w:txbxContent><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr/>"#),
    )
}

#[test]
fn the_layouts_compared_read_their_text_boxes_lines_and_the_summary_does_not() {
    let mut engine = engine();
    let before = engine
        .lay_out_bytes(Arc::from(package(&text_box("Before"))))
        .unwrap()
        .unwrap();
    let (summary, contents) = (engine.summary(&before).unwrap(), engine.contents(&before).unwrap());
    assert!(
        contents.pages[0].lines.len() > summary.pages[0].lines.len(),
        "the text box's line"
    );
    let after = engine
        .lay_out_bytes(Arc::from(package(&text_box("Behind"))))
        .unwrap()
        .unwrap();
    let aliases = std::collections::HashMap::new();
    // docx-agent's layout summary (`Page.lines`) does not see a text box's text change;
    // its comparison of the layouts (`Page.text_lines`, word_template's reflow) does
    assert!(engine
        .summary(&after)
        .unwrap()
        .compare(&summary, &aliases)
        .changed
        .is_empty());
    assert_eq!(
        engine.contents(&after).unwrap().compare(&contents, &aliases).changed,
        vec![1]
    );
}

/// `package`, a version made from `before`, read as a session reads it: a part it shares
/// with `before` stamped as there.
fn read_after(package: ooxml_opc::Package, before: &ooxml_word_model::WordPackage) -> ooxml_word_model::WordPackage {
    ooxml_word_model::WordPackage::open_after(package, Some(before)).0
}

/// `bytes` opened as a package, as a session holds a version.
fn opened(bytes: Vec<u8>) -> ooxml_word_model::WordPackage {
    ooxml_word_model::WordPackage::open(
        ooxml_opc::Package::open(Arc::from(bytes), &ooxml_base::Limits::default()).unwrap(),
    )
    .0
}

#[test]
fn a_version_given_as_a_package_says_which_part_it_changed() {
    // A session lays out each version as its package: the parts changed are found from it.
    let body = paragraphs(60, None);
    let large = r#"<w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val="40"/></w:rPr></w:rPrDefault></w:docDefaults>"#;
    let first = opened(package_styled(&body, None));
    let styles = format!(r#"<w:styles {W}>{large}</w:styles>"#);
    let edited = read_after(
        first.with_bytes("word/styles.xml", Arc::from(styles.into_bytes())),
        &first,
    );
    let mut kept = engine();
    kept.lay_out(&first, None).unwrap().unwrap();
    let again = kept.lay_out(&edited, None).unwrap().unwrap();
    let cause = again.relaid_cause().expect("laid out whole from a version before");
    assert_eq!(
        (cause.reason, cause.part.as_deref()),
        ("styles", Some("word/styles.xml"))
    );
}

#[test]
fn a_version_given_as_a_package_after_another_part_changed_says_what_a_fresh_layout_says() {
    // SmartArt whose data model part is missing, then (a session's edit of the
    // relationships, a part added) with one: the second version's warnings are its own,
    // not the first's said again for the page the edit did not reach.
    let body = r#"<w:p><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="914400" cy="457200"/><wp:docPr id="1" name="Diagram 1"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"><dgm:relIds xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:dm="rId9" r:lo="rId10" r:qs="rId11" r:cs="rId12"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#;
    let first = opened(package(body));
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData" Target="diagrams/data1.xml"/></Relationships>"#;
    let data = r#"<dgm:dataModel xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram"><dgm:ptLst><dgm:pt modelId="0" type="doc"/></dgm:ptLst><dgm:cxnLst/></dgm:dataModel>"#;
    let edited = read_after(
        first
            .with_bytes("word/diagrams/data1.xml", Arc::from(data.as_bytes()))
            .with_bytes("word/_rels/document.xml.rels", Arc::from(rels.as_bytes())),
        &first,
    );
    let mut kept = engine();
    let before = kept.lay_out(&first, None).unwrap().unwrap();
    assert!(!before.said().warnings.is_empty());
    let again = kept.lay_out(&edited, None).unwrap().unwrap();
    let whole = engine().lay_out(&edited, None).unwrap().unwrap();
    assert_ne!(before.said(), whole.said(), "the edit changes what is said");
    assert_eq!(again.said(), whole.said());
}

/// A body of a paragraph in `early`, a few pages of paragraphs in `first`, a last
/// paragraph in `last`, and a header in `header`.
fn faces_package((early, first, last): (&str, &str, &str), header: &str) -> Vec<u8> {
    let run = |face: &str, text: &str| {
        format!(
            r#"<w:p><w:r><w:rPr><w:rFonts w:ascii="{face}" w:hAnsi="{face}"/></w:rPr><w:t>{text}</w:t></w:r></w:p>"#
        )
    };
    let mut body = run(early, "the first words");
    body.extend((0..400).map(|k| run(first, &format!("{k} some words on a line"))));
    body.push_str(&run(last, "the last words"));
    body.push_str(r#"<w:sectPr><w:headerReference w:type="default" r:id="rIdH" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/></w:sectPr>"#);
    package_parts(&body, None, Some(&run(header, "a header")))
}

/// The substituted faces in coverage's order, the version's pages, the pages painted for it.
fn substituted_order(engine: &mut PageEngine, bytes: Vec<u8>) -> (Vec<String>, usize, u64) {
    let version = engine.lay_out_bytes(bytes.into()).unwrap().unwrap();
    let before = engine.stats().pages_painted;
    let coverage = engine.coverage(&version).unwrap();
    let order = coverage.substituted.iter().map(|s| s.family.clone()).collect();
    (order, version.page_count(), engine.stats().pages_painted - before)
}

#[test]
fn substituted_faces_are_ordered_as_the_body_first_draws_them() {
    // A face the body never draws (the header's): every page is read, and it comes last.
    let fallback_engine = || {
        engine_with(
            EngineBudgets {
                held: Arc::new(Unlimited),
                paint: Arc::new(Unlimited),
            },
            true,
        )
    };
    let mut e = fallback_engine();
    let package = faces_package(("Absent One", "Absent One", "Absent Three"), "Absent Two");
    let (order, pages, painted) = substituted_order(&mut e, package);
    assert!(pages > 3, "{pages}");
    assert_eq!(order, ["Absent One", "Absent Three", "Absent Two"]);
    assert_eq!(painted, u64::try_from(pages).unwrap(), "every page read");
    // Every substituted face drawn on the first page: the pages after it are not read.
    let mut e = fallback_engine();
    let package = faces_package(("Absent Two", "Absent One", "Absent One"), "Absent Two");
    let (order, pages, painted) = substituted_order(&mut e, package);
    assert!(pages > 3, "{pages}");
    assert_eq!(order, ["Absent Two", "Absent One"]);
    assert_eq!(painted, 1, "only the first page read");
}

/// Every page of `version` as a check reads it.
fn reads(
    engine: &mut PageEngine,
    version: &ooxml_word_pages::LaidVersion,
) -> Vec<ooxml_word_pages::page_read::PageRead> {
    (0..version.page_count())
        .map(|k| engine.page_read(version, k).unwrap().read().clone())
        .collect()
}

#[test]
fn a_check_after_a_local_edit_reads_again_only_the_pages_it_changed() {
    let mut e = engine();
    let first = e
        .lay_out_bytes(package(&paragraphs(300, None)).into())
        .unwrap()
        .unwrap();
    let pages = first.page_count();
    assert!(pages > 4, "{pages}");
    let _ = reads(&mut e, &first);
    assert_eq!(e.stats().pages_read, (u64::try_from(pages).unwrap(), 0, 0));
    // A paragraph near the end edited in place: only its page is painted again.
    let bytes = package(&paragraphs(300, Some(290)));
    let edited = e.lay_out_bytes(bytes.clone().into()).unwrap().unwrap();
    let before = e.stats().pages_read;
    let warm = reads(&mut e, &edited);
    let (painted, taken) = (e.stats().pages_read.0 - before.0, e.stats().pages_read.1 - before.1);
    assert!(
        painted <= 2 && taken + painted == u64::try_from(pages).unwrap(),
        "{painted} {taken}"
    );
    let mut cold = engine();
    let whole = cold.lay_out_bytes(bytes.into()).unwrap().unwrap();
    assert_eq!(
        warm,
        reads(&mut cold, &whole),
        "the reads taken are the reads made cold"
    );
    // A paragraph added at the start: every later paragraph's path moved, so every page
    // is read again, and the reads are the cold ones.
    let mut moved = String::from("<w:p><w:r><w:t>added first</w:t></w:r></w:p>");
    moved.push_str(&paragraphs(300, Some(290)));
    let bytes = package(&moved);
    let shifted = e.lay_out_bytes(bytes.clone().into()).unwrap().unwrap();
    let warm = reads(&mut e, &shifted);
    let mut cold = engine();
    let whole = cold.lay_out_bytes(bytes.into()).unwrap().unwrap();
    assert_eq!(warm, reads(&mut cold, &whole));
}

#[test]
fn page_reads_are_held_within_the_paint_budget() {
    let mut e = engine_in(EngineBudgets {
        held: Arc::new(Unlimited),
        paint: Arc::new(Nothing),
    });
    let bytes = package(&paragraphs(120, None));
    let first = e.lay_out_bytes(bytes.clone().into()).unwrap().unwrap();
    let _ = reads(&mut e, &first);
    let again = reads(&mut e, &first);
    // Nothing granted: nothing kept, every page painted again, the reads the same.
    let pages = u64::try_from(first.page_count()).unwrap();
    assert_eq!(e.stats().pages_read, (2 * pages, 0, 0));
    let mut cold = engine();
    let whole = cold.lay_out_bytes(bytes.into()).unwrap().unwrap();
    assert_eq!(again, reads(&mut cold, &whole));
}

#[test]
fn page_reads_go_with_the_versions_that_held_their_pages() {
    let paint = Arc::new(Counted::default());
    let mut e = engine_in(EngineBudgets {
        held: Arc::new(Unlimited),
        paint: Arc::clone(&paint) as Arc<dyn CacheBudget>,
    });
    let first = e
        .lay_out_bytes(package(&paragraphs(120, None)).into())
        .unwrap()
        .unwrap();
    let _ = reads(&mut e, &first);
    assert!(paint.used() > 0, "the reads are counted");
    drop(first);
    // Other versions laid out (each sweeps): the reads of pages no version holds go.
    let other = |n| package(&format!("<w:p><w:r><w:t>another document {n}</w:t></w:r></w:p>"));
    let _ = e.lay_out_bytes(other(1).into()).unwrap().unwrap();
    let _ = e.lay_out_bytes(other(2).into()).unwrap().unwrap();
    assert_eq!(paint.used(), 0);
}

/// `n` paragraphs, a header showing the page count.
fn counted(n: usize) -> Vec<u8> {
    let mut body = paragraphs(n, None);
    body.push_str(r#"<w:sectPr><w:headerReference w:type="default" r:id="rIdH" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/></w:sectPr>"#);
    let header = r#"<w:p><w:pPr><w:pBdr><w:bottom w:val="single" w:sz="4" w:space="1" w:color="auto"/></w:pBdr></w:pPr><w:r><w:t xml:space="preserve">Pages: </w:t></w:r><w:fldSimple w:instr=" NUMPAGES "><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p>"#;
    package_parts(&body, None, Some(header))
}

#[test]
fn a_page_added_at_the_end_has_the_pages_before_it_paint_only_their_stories() {
    let mut e = engine();
    let first = e.lay_out_bytes(counted(300).into()).unwrap().unwrap();
    let _ = reads(&mut e, &first);
    // Paragraphs added at the end: one page more, so every header shows another count.
    let bytes = counted(330);
    let longer = e.lay_out_bytes(bytes.clone().into()).unwrap().unwrap();
    assert!(longer.page_count() > first.page_count());
    let before = e.stats().pages_read;
    let warm = reads(&mut e, &longer);
    let after = e.stats().pages_read;
    let (painted, taken, stories) = (after.0 - before.0, after.1 - before.1, after.2 - before.2);
    assert!(
        stories + 2 >= u64::try_from(first.page_count()).unwrap(),
        "{painted} {taken} {stories}"
    );
    assert!(painted <= 3, "{painted} {taken} {stories}");
    let mut cold = engine();
    let whole = cold.lay_out_bytes(bytes.into()).unwrap().unwrap();
    assert_eq!(
        warm,
        reads(&mut cold, &whole),
        "the reads joined are the reads made cold"
    );
}
