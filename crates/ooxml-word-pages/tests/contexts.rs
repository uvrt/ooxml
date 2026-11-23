//! A version laid out before a style edit is painted and measured in its own styles (lane
//! PG1): its pages' SVG, paint, lines' text, reads, digests, stories' lines and what they
//! draw beyond the facts are a fresh engine's of the same bytes, after the engine laid the
//! edit out; the edit's version is painted as before (nothing of the older context read
//! in it); a comparison across the edit reports what fresh engines' whole summaries do;
//! a trial from the older version is not laid out in the current context and leaves the
//! engine laying the next version out from the last. Made-up documents of headings (a
//! style the edit changes: its size or its colour), plain paragraphs, a header holding a
//! heading and a footnote.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ooxml_derive::CacheBudget;
use ooxml_fonts::{Folder, Location, WordFontOptions};
use ooxml_word_pages::{DiskFonts, EngineBudgets, EngineOptions, FontSetup, Laid, LaidVersion, PageEngine};
use ooxml_zip::{Method, NewEntry, Writer};
use std::collections::HashMap;
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

/// Never says stop.
struct Never;

impl ooxml_base::Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

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
    let budgets = EngineBudgets {
        held: Arc::new(Unlimited),
        paint: Arc::new(Unlimited),
    };
    PageEngine::new(setup, EngineOptions::default(), budgets)
}

/// The headings' style: its size (half-points) and colour.
#[derive(Clone, Copy)]
struct Heading {
    size: u32,
    colour: &'static str,
}

/// The body: `paragraphs` paragraphs, every fifth a heading (`edited`: that one's text
/// changed), a footnote on the second, a section of a default header.
fn body(paragraphs: usize, edited: Option<usize>) -> String {
    let mut body = String::new();
    for k in 0..paragraphs {
        let words = if Some(k) == edited {
            "changed words here"
        } else {
            "some words on a line of the body that runs on"
        };
        let style = if k % 5 == 0 {
            r#"<w:pPr><w:pStyle w:val="Heading1"/></w:pPr>"#
        } else {
            ""
        };
        let note = if k == 1 {
            r#"<w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:footnoteReference w:id="1"/></w:r>"#
        } else {
            ""
        };
        body.push_str(&format!("<w:p>{style}<w:r><w:t>{k} {words}</w:t></w:r>{note}</w:p>"));
    }
    body.push_str(r#"<w:sectPr xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:headerReference w:type="default" r:id="rIdH"/></w:sectPr>"#);
    body
}

/// A document of `paragraphs` paragraphs, every fifth a heading (`edited`: that one's text
/// changed), a header holding a heading, a footnote on the second paragraph.
fn package(heading: Heading, paragraphs: usize, edited: Option<usize>) -> Arc<[u8]> {
    let entry = |name: &str| NewEntry {
        name: name.into(),
        method: Method::Deflated,
        dos_time: 0x6000,
        dos_date: 0x5a21,
        external_attr: 0,
        create_system: 0,
    };
    let rel = |id: &str, kind: &str, target: &str| {
        format!(
            r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#
        )
    };
    let rels = |inner: String| {
        format!(
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{inner}</Relationships>"#
        )
    };
    let body = body(paragraphs, edited);
    let styles = format!(
        r#"<w:styles {W}><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:rPr><w:sz w:val="20"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/></w:pPr><w:rPr><w:b/><w:color w:val="{}"/><w:sz w:val="{}"/></w:rPr></w:style></w:styles>"#,
        heading.colour, heading.size
    );
    let footnotes = format!(
        r#"<w:footnotes {W}><w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:type="continuationSeparator" w:id="0"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote><w:footnote w:id="1"><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:footnoteRef/></w:r><w:r><w:t> a note in a heading</w:t></w:r></w:p></w:footnote></w:footnotes>"#
    );
    let files = [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#.to_owned(),
        ),
        ("_rels/.rels", rels(rel("rId1", "officeDocument", "word/document.xml"))),
        (
            "word/document.xml",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W}><w:body>{body}</w:body></w:document>"#
            ),
        ),
        (
            "word/_rels/document.xml.rels",
            rels(
                [
                    rel("rId1", "styles", "styles.xml"),
                    rel("rIdH", "header", "header1.xml"),
                    rel("rIdF", "footnotes", "footnotes.xml"),
                ]
                .concat(),
            ),
        ),
        ("word/styles.xml", styles),
        (
            "word/header1.xml",
            format!(
                r#"<w:hdr {W}><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>A heading in the header</w:t></w:r></w:p></w:hdr>"#
            ),
        ),
        ("word/footnotes.xml", footnotes),
    ];
    let mut w = Writer::new();
    for (name, data) in files {
        w.add(&entry(name), data.as_bytes()).unwrap();
    }
    Arc::from(w.finish(b"").unwrap())
}

const PLAIN: Heading = Heading {
    size: 28,
    colour: "000000",
};
const LARGE: Heading = Heading {
    size: 56,
    colour: "000000",
};
const RED: Heading = Heading {
    size: 28,
    colour: "C00000",
};

/// Everything a reader takes of `version`'s pages from `engine`, page by page.
#[derive(Debug, PartialEq)]
struct Drawn {
    svgs: Vec<String>,
    texts: Vec<Vec<ooxml_word_pages::line_text::LineText>>,
    reads: Vec<ooxml_word_pages::page_read::PageRead>,
    digests: Vec<ooxml_word_pages::PageDigests>,
    stories: Vec<Vec<ooxml_word_layout::facts::StoryLineFact>>,
    beyond: Vec<ooxml_word_pages::layout_facts::BeyondFacts>,
}

fn drawn(engine: &mut PageEngine, version: &LaidVersion) -> Drawn {
    let pages = 0..version.page_count();
    Drawn {
        svgs: pages.clone().map(|k| engine.page_svg(version, k).unwrap().0).collect(),
        texts: pages.clone().map(|k| engine.line_texts(version, k).unwrap()).collect(),
        reads: pages
            .clone()
            .map(|k| engine.page_read(version, k).unwrap().read().clone())
            .collect(),
        digests: pages
            .clone()
            .map(|k| engine.page_digests(version, k).unwrap())
            .collect(),
        stories: pages.clone().map(|k| engine.story_lines(version, k).unwrap()).collect(),
        beyond: pages.map(|k| engine.beyond_facts(Laid::Version(version), k)).collect(),
    }
}

/// `bytes` laid out by a fresh engine, and all it draws.
fn fresh(bytes: &Arc<[u8]>) -> Drawn {
    let mut fresh = engine();
    let version = fresh.lay_out_bytes(Arc::clone(bytes)).unwrap().unwrap();
    drawn(&mut fresh, &version)
}

fn whole_summary(bytes: &Arc<[u8]>) -> Arc<ooxml_word_pages::summary::Summary> {
    let mut fresh = engine();
    let version = fresh.lay_out_bytes(Arc::clone(bytes)).unwrap().unwrap();
    fresh.summary(&version).unwrap()
}

#[test]
fn a_version_laid_out_before_a_style_edit_is_drawn_in_its_own_styles() {
    for edit in [LARGE, RED] {
        let (first, second) = (package(PLAIN, 40, None), package(edit, 40, None));
        let mut kept = engine();
        let before = kept.lay_out_bytes(Arc::clone(&first)).unwrap().unwrap();
        let after = kept.lay_out_bytes(Arc::clone(&second)).unwrap().unwrap();
        let (old, new) = (fresh(&first), fresh(&second));
        assert_ne!(old.svgs, new.svgs, "the edit draws otherwise");
        // painted before (cold) and again (its paint kept): its own styles both times
        assert_eq!(drawn(&mut kept, &before), old);
        assert_eq!(drawn(&mut kept, &before), old);
        // the edit's version, painted after the older one: nothing of the older read in it
        assert_eq!(drawn(&mut kept, &after), new);
        assert_eq!(drawn(&mut kept, &before), old);
    }
}

#[test]
fn a_comparison_across_a_style_edit_reports_its_change() {
    for edit in [LARGE, RED] {
        let (first, second) = (package(PLAIN, 40, None), package(edit, 40, None));
        let mut kept = engine();
        let before = kept.lay_out_bytes(Arc::clone(&first)).unwrap().unwrap();
        let after = kept.lay_out_bytes(Arc::clone(&second)).unwrap().unwrap();
        let expected = whole_summary(&second).compare(&whole_summary(&first), &HashMap::new());
        assert_eq!(kept.compare(&after, &before, &HashMap::new()), Some(expected.clone()));
        // and back: the older compared from the edit's
        let back = whole_summary(&first).compare(&whole_summary(&second), &HashMap::new());
        assert_eq!(kept.compare(&before, &after, &HashMap::new()), Some(back));
        assert_eq!(kept.summary(&before).unwrap().pages, whole_summary(&first).pages);
    }
    // a size edit moves lines: a reflow is reported
    let (first, second) = (package(PLAIN, 40, None), package(LARGE, 40, None));
    let expected = whole_summary(&second).compare(&whole_summary(&first), &HashMap::new());
    assert_ne!(expected, ooxml_word_pages::summary::Reflow::default());
}

#[test]
fn a_trial_from_a_version_before_a_style_edit_is_not_laid_out_and_changes_nothing() {
    let (first, second) = (package(PLAIN, 40, None), package(LARGE, 40, None));
    let mut kept = engine();
    let before = kept.lay_out_bytes(Arc::clone(&first)).unwrap().unwrap();
    let after = kept.lay_out_bytes(Arc::clone(&second)).unwrap().unwrap();
    let opened =
        |bytes: &Arc<[u8]>| ooxml_opc::Package::open(Arc::clone(bytes), &ooxml_base::Limits::default()).unwrap();
    let never = Never;
    let trial = kept
        .trial(&opened(&package(PLAIN, 40, Some(7))), &before, &never)
        .unwrap();
    assert!(trial.is_none(), "a trial in a context the engine left");
    assert!(Arc::ptr_eq(kept.last().unwrap(), &after));
    // the last version is still drawn in its own styles, the next laid out from it
    assert_eq!(drawn(&mut kept, &after), fresh(&second));
    let edited = package(LARGE, 40, Some(7));
    let next = kept.lay_out_bytes(Arc::clone(&edited)).unwrap().unwrap();
    assert!(next.relaid(), "laid out from the last: {:?}", kept.stats());
    assert_eq!(drawn(&mut kept, &next), fresh(&edited));
    // a trial from the last version is laid out as before
    let trial = kept
        .trial(&opened(&package(LARGE, 40, Some(9))), &next, &never)
        .unwrap();
    assert!(trial.is_some());
}

#[test]
fn a_version_of_a_context_left_is_painted_after_more_style_edits() {
    let first = package(PLAIN, 40, None);
    let mut kept = engine();
    let before = kept.lay_out_bytes(Arc::clone(&first)).unwrap().unwrap();
    // versions of three more contexts laid out, each let go
    for (k, heading) in [LARGE, RED, LARGE].into_iter().enumerate() {
        let _ = kept.lay_out_bytes(package(heading, 40, Some(k))).unwrap().unwrap();
    }
    assert_eq!(drawn(&mut kept, &before), fresh(&first));
}
