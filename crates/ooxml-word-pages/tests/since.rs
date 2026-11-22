//! A comparison of two versions page by page equals the comparison of their summaries made
//! whole by fresh engines (lane P5e): made-up documents of paragraphs of every length,
//! headings a header's `STYLEREF` shows, footnotes, floating drawings and text boxes
//! holding a `PAGE` field, section breaks of one and two columns with and without page
//! borders, and a header holding `PAGE`, `NUMPAGES` and, at times, a watermark -- edited
//! one step at a time (a paragraph's text, a paragraph inserted or deleted, a heading made
//! or unmade, a footnote added or its text changed, a section break's columns or borders,
//! the watermark, a drawing, the headings' style). At every step the engine that laid the
//! version out from the one before compares it page by page with the version before and
//! with the one before that, and summarises it from what it digested before; each answer
//! is the one a fresh engine's whole summaries give, and each page's body ids read from its
//! geometry are its summary's. Fixed seeds here; `many_seeds` (ignored) runs many.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_derive::CacheBudget;
use ooxml_fonts::{Folder, Location, WordFontOptions};
use ooxml_word_pages::summary::{Reflow, Summary};
use ooxml_word_pages::{DiskFonts, EngineBudgets, EngineOptions, FontSetup, LaidVersion, PageEngine};
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

/// xorshift64*: the test's own generator, seeded.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
    fn pick(&mut self, n: usize) -> usize {
        usize::try_from(self.below(u64::try_from(n).unwrap())).unwrap()
    }
}

/// A made-up block.
#[derive(Clone, Debug)]
enum Block {
    Para {
        words: u64,
        heading: bool,
        /// A footnote reference: the note's id.
        note: Option<u32>,
        /// A floating drawing anchored in it: a text box holding a `PAGE` field, or a shape.
        drawing: Option<bool>,
        page_break: bool,
    },
    /// A paragraph ending a section of `columns` columns, with page borders or not.
    Break {
        columns: u32,
        borders: bool,
        next_page: bool,
    },
}

/// A made-up document.
#[derive(Clone, Debug)]
struct Doc {
    blocks: Vec<Block>,
    notes: HashMap<u32, u64>,
    next_note: u32,
    watermark: bool,
    heading_size: u32,
}

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office""#;

const WORDS: [&str; 8] = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "theta", "kappa"];

fn text(words: u64, salt: usize) -> String {
    (0..words)
        .map(|k| WORDS[(usize::try_from(k).unwrap() + salt) % WORDS.len()])
        .collect::<Vec<_>>()
        .join(" ")
}

fn run(text: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{text}</w:t></w:r>"#)
}

fn field(instruction: &str, cached: &str) -> String {
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> {instruction} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{}<w:r><w:fldChar w:fldCharType="end"/></w:r>"#,
        run(cached)
    )
}

fn sect_pr(columns: u32, borders: bool, next_page: bool) -> String {
    let kind = if next_page { "nextPage" } else { "continuous" };
    let borders = if borders {
        r#"<w:pgBorders w:offsetFrom="page"><w:top w:val="single" w:sz="8" w:space="24" w:color="auto"/><w:left w:val="single" w:sz="8" w:space="24" w:color="auto"/><w:bottom w:val="single" w:sz="8" w:space="24" w:color="auto"/><w:right w:val="single" w:sz="8" w:space="24" w:color="auto"/></w:pgBorders>"#
    } else {
        ""
    };
    format!(
        r#"<w:sectPr><w:headerReference w:type="default" r:id="rIdH"/><w:type w:val="{kind}"/><w:pgSz w:w="5760" w:h="5040"/><w:pgMar w:top="900" w:right="540" w:bottom="720" w:left="540" w:header="300" w:footer="300" w:gutter="0"/>{borders}<w:cols w:num="{columns}" w:space="240"/></w:sectPr>"#
    )
}

fn drawing_xml(text_box: bool, k: usize) -> String {
    let content = if text_box {
        format!(
            r#"<wps:txbx><w:txbxContent><w:p>{}{}</w:p></w:txbxContent></wps:txbx>"#,
            run("page "),
            field("PAGE", "1")
        )
    } else {
        String::new()
    };
    format!(
        r#"<w:r><w:drawing><wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="{k}" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>1270000</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="914400" cy="381000"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapSquare wrapText="bothSides"/><wp:docPr id="{k}" name="d{k}"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="381000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></wps:spPr>{content}<wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"#
    )
}

impl Doc {
    fn new(rng: &mut Rng) -> Doc {
        let mut doc = Doc {
            blocks: Vec::new(),
            notes: HashMap::new(),
            next_note: 1,
            watermark: rng.chance(30),
            heading_size: 32,
        };
        let count = 18 + rng.pick(14);
        for _ in 0..count {
            let block = doc.block(rng);
            doc.blocks.push(block);
        }
        doc
    }

    fn block(&mut self, rng: &mut Rng) -> Block {
        if rng.chance(8) {
            return Block::Break {
                columns: 1 + u32::from(rng.chance(40)),
                borders: rng.chance(40),
                next_page: rng.chance(60),
            };
        }
        let note = rng.chance(15).then(|| {
            let id = self.next_note;
            self.next_note += 1;
            self.notes.insert(id, 2 + rng.below(20));
            id
        });
        Block::Para {
            words: rng.below(70),
            heading: rng.chance(15),
            note,
            drawing: rng.chance(8).then(|| rng.chance(50)),
            page_break: rng.chance(5),
        }
    }

    /// One random edit; `false` where it found nothing to do.
    fn edit(&mut self, rng: &mut Rng) -> bool {
        let n = self.blocks.len();
        let k = rng.pick(n.max(1));
        match rng.below(10) {
            0 | 1 => match self.blocks.get_mut(k) {
                Some(Block::Para { words, .. }) => *words = rng.below(70),
                _ => return false,
            },
            2 => {
                let block = self.block(rng);
                self.blocks.insert(k.min(n), block);
            }
            3 if n > 4 => {
                self.blocks.remove(k);
            }
            4 => match self.blocks.get_mut(k) {
                Some(Block::Para { heading, .. }) => *heading = !*heading,
                _ => return false,
            },
            5 => match self.blocks.get_mut(k) {
                Some(Block::Para { note: note @ None, .. }) => {
                    let id = self.next_note;
                    self.next_note += 1;
                    self.notes.insert(id, 2 + rng.below(20));
                    *note = Some(id);
                }
                Some(Block::Para { note: Some(id), .. }) => {
                    let id = *id;
                    self.notes.insert(id, 2 + rng.below(40));
                }
                _ => return false,
            },
            6 => match self.blocks.get_mut(k) {
                Some(Block::Break { columns, borders, .. }) => {
                    if rng.chance(50) {
                        *columns = 3 - *columns;
                    } else {
                        *borders = !*borders;
                    }
                }
                _ => return false,
            },
            7 => self.watermark = !self.watermark,
            8 => match self.blocks.get_mut(k) {
                Some(Block::Para { drawing, .. }) => {
                    *drawing = match drawing {
                        None => Some(rng.chance(50)),
                        Some(_) => None,
                    }
                }
                _ => return false,
            },
            _ => self.heading_size = if self.heading_size == 32 { 40 } else { 32 },
        }
        true
    }

    fn body(&self) -> String {
        let mut out = String::new();
        for (k, block) in self.blocks.iter().enumerate() {
            match block {
                Block::Para {
                    words,
                    heading,
                    note,
                    drawing,
                    page_break,
                } => {
                    let style = if *heading {
                        r#"<w:pStyle w:val="Heading1"/>"#
                    } else {
                        ""
                    };
                    let before = if *page_break { r#"<w:pageBreakBefore/>"# } else { "" };
                    let reference = note.map_or(String::new(), |id| {
                        format!(r#"<w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:footnoteReference w:id="{id}"/></w:r>"#)
                    });
                    let drawn = drawing.map_or(String::new(), |text_box| drawing_xml(text_box, k + 1));
                    out.push_str(&format!(
                        r#"<w:p><w:pPr>{style}{before}</w:pPr>{}{reference}{drawn}</w:p>"#,
                        run(&format!("{k} {}", text(*words, k)))
                    ));
                }
                Block::Break {
                    columns,
                    borders,
                    next_page,
                } => out.push_str(&format!(
                    r#"<w:p><w:pPr>{}</w:pPr>{}</w:p>"#,
                    sect_pr(*columns, *borders, *next_page),
                    run("end of a section")
                )),
            }
        }
        out.push_str(&sect_pr(1, false, true));
        out
    }

    fn header(&self) -> String {
        let watermark = if self.watermark {
            r##"<w:r><w:pict><v:shapetype id="_x0000_t136" coordsize="21600,21600" o:spt="136"><v:textpath on="t" fitshape="t"/></v:shapetype><v:shape id="Made1" type="#_x0000_t136" style="position:absolute;width:200pt;height:80pt;rotation:315" fillcolor="silver" stroked="f"><v:textpath style="font-family:&quot;Carlito&quot;;font-size:1pt" string="DRAFT"/></v:shape></w:pict></w:r>"##
        } else {
            ""
        };
        format!(
            r#"<w:hdr {W}><w:p>{watermark}{}{}{}{}{}</w:p></w:hdr>"#,
            field("STYLEREF \"Heading 1\"", "heading"),
            run(" page "),
            field("PAGE", "1"),
            run(" of "),
            field("NUMPAGES", "1")
        )
    }

    fn footnotes(&self) -> String {
        let mut xml = format!(
            r#"<w:footnotes {W}><w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:type="continuationSeparator" w:id="0"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>"#
        );
        let mut ids: Vec<_> = self.notes.iter().collect();
        ids.sort();
        for (id, words) in ids {
            xml.push_str(&format!(
                r#"<w:footnote w:id="{id}"><w:p><w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:footnoteRef/></w:r>{}</w:p></w:footnote>"#,
                run(&format!(" {}", text(*words, usize::try_from(*id).unwrap())))
            ));
        }
        xml.push_str("</w:footnotes>");
        xml
    }

    fn styles(&self) -> String {
        format!(
            r#"<w:styles {W}><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:rPr><w:sz w:val="20"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/></w:pPr><w:rPr><w:b/><w:sz w:val="{}"/></w:rPr></w:style></w:styles>"#,
            self.heading_size
        )
    }

    fn package(&self) -> Arc<[u8]> {
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
        let files = [
            (
                "[Content_Types].xml",
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#.to_owned(),
            ),
            ("_rels/.rels", rels(rel("rId1", "officeDocument", "word/document.xml"))),
            (
                "word/document.xml",
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W}><w:body>{}</w:body></w:document>"#,
                    self.body()
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
            ("word/styles.xml", self.styles()),
            ("word/header1.xml", self.header()),
            ("word/footnotes.xml", self.footnotes()),
        ];
        let mut w = Writer::new();
        for (name, data) in files {
            w.add(&entry(name), data.as_bytes()).unwrap();
        }
        Arc::from(w.finish(b"").unwrap())
    }
}

/// A version laid out whole by a fresh engine, and its summary.
fn whole(bytes: &Arc<[u8]>) -> Arc<Summary> {
    let mut fresh = engine();
    let version = fresh.lay_out_bytes(Arc::clone(bytes)).unwrap().unwrap();
    fresh.summary(&version).unwrap()
}

/// Each page's body ids, in drawing order, as `summary` names its lines (the ids a
/// comparison reads: a line no id names, such as a note separator's, is not among them).
fn summary_ids(summary: &Summary) -> Vec<Vec<String>> {
    summary
        .pages
        .iter()
        .map(|page| {
            page.lines
                .iter()
                .filter(|l| l.body)
                .filter_map(|l| l.id.as_deref().map(str::to_owned))
                .collect()
        })
        .collect()
}

fn run_seed(seed: u64, steps: usize) {
    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut doc = Doc::new(&mut rng);
    let mut lazy = engine();
    let mut summarising = engine();
    let none = HashMap::new();
    let first = doc.package();
    let mut laid: Vec<Arc<LaidVersion>> = vec![lazy.lay_out_bytes(Arc::clone(&first)).unwrap().unwrap()];
    let mut summarised = summarising.lay_out_bytes(Arc::clone(&first)).unwrap().unwrap();
    let mut wholes: Vec<Arc<Summary>> = vec![whole(&first)];
    lazy.watch(&laid[0]);
    for step in 0..steps {
        while !doc.edit(&mut rng) {}
        let bytes = doc.package();
        let version = lazy.lay_out_bytes(Arc::clone(&bytes)).unwrap().unwrap();
        let alone = whole(&bytes);
        let at = format!("seed {seed} step {step}: {doc:?}");
        for back in 1..=2.min(laid.len()) {
            let before = &laid[laid.len() - back];
            let found = lazy.compare(&version, before, &none);
            let expected = alone.compare(&wholes[wholes.len() - back], &none);
            assert_eq!(found.as_ref(), Some(&expected), "{back} back, {at}");
        }
        let read: Vec<Vec<String>> = lazy
            .body_ids(&version)
            .unwrap()
            .into_iter()
            .map(|page| page.into_iter().flatten().collect())
            .collect();
        assert_eq!(read, summary_ids(&alone), "{at}");
        summarised = summarising.lay_out_bytes(Arc::clone(&bytes)).unwrap().unwrap();
        let made = summarising.summary(&summarised).unwrap();
        assert_eq!(made.pages.len(), alone.pages.len(), "{at}");
        for (k, (a, b)) in made.pages.iter().zip(&alone.pages).enumerate() {
            assert_eq!(**a, **b, "page {k}, {at}");
        }
        lazy.watch(&version);
        laid.push(version);
        wholes.push(alone);
        if laid.len() > 3 {
            laid.remove(0);
            wholes.remove(0);
        }
    }
    drop(summarised);
}

#[test]
fn a_comparison_page_by_page_is_the_comparison_of_whole_summaries() {
    for seed in 1..=3 {
        run_seed(seed, 8);
    }
}

#[test]
#[ignore = "many seeds: run by hand (cargo test -- --ignored)"]
fn many_seeds() {
    for seed in 1..=60 {
        run_seed(seed, 14);
    }
}

#[test]
fn a_version_laid_out_the_same_is_compared_without_painting_and_the_cache_is_bounded() {
    let mut rng = Rng(7);
    let mut doc = Doc::new(&mut rng);
    doc.blocks.truncate(24);
    let mut engine = engine();
    let first = engine.lay_out_bytes(doc.package()).unwrap().unwrap();
    // The last paragraph's text, edited: only the last pages are painted, in both versions.
    let last = doc
        .blocks
        .iter()
        .rposition(|b| matches!(b, Block::Para { .. }))
        .unwrap();
    if let Block::Para { words, .. } = &mut doc.blocks[last] {
        *words = if *words > 30 { 3 } else { 60 };
    }
    let second = engine.lay_out_bytes(doc.package()).unwrap().unwrap();
    let painted = engine.stats().pages_painted;
    let found = engine.compare(&second, &first, &HashMap::new()).unwrap();
    let paints = engine.stats().pages_painted - painted;
    let pages = u64::try_from(first.page_count() + second.page_count()).unwrap();
    assert!(paints < pages, "{paints} of {pages} painted: {found:?}");
    // Compared again: every page is digested already.
    let painted = engine.stats().pages_painted;
    assert_eq!(engine.compare(&second, &first, &HashMap::new()), Some(found));
    assert_eq!(engine.stats().pages_painted, painted);
    let (entries, bytes) = engine.digest_cache();
    assert!(entries > 0 && bytes > 0);
    // A bound of one page's digests holds no more than that.
    engine.set_digest_limit(bytes / entries);
    let _ = engine.summary(&second);
    let (_, held) = engine.digest_cache();
    assert!(held <= bytes / entries, "{held}");
}

#[test]
fn a_version_laid_out_before_a_style_change_is_compared_after_it_watched_or_not() {
    let mut rng = Rng(11);
    let mut doc = Doc::new(&mut rng);
    let mut engine = engine();
    let first = doc.package();
    let before = engine.lay_out_bytes(Arc::clone(&first)).unwrap().unwrap();
    engine.watch(&before);
    doc.heading_size = 48;
    let bytes = doc.package();
    let after = engine.lay_out_bytes(Arc::clone(&bytes)).unwrap().unwrap();
    let found = engine.compare(&after, &before, &HashMap::new());
    assert_eq!(found, Some(whole(&bytes).compare(&whole(&first), &HashMap::new())));
    // Unwatched and never summarised, a version laid out before a style change is
    // compared in its own styles, as fresh engines' whole summaries compare.
    doc.heading_size = 40;
    let older = doc.package();
    let unwatched = engine.lay_out_bytes(Arc::clone(&older)).unwrap().unwrap();
    doc.heading_size = 32;
    let newer = doc.package();
    let again = engine.lay_out_bytes(Arc::clone(&newer)).unwrap().unwrap();
    let expected = whole(&newer).compare(&whole(&older), &HashMap::new());
    assert_ne!(expected, Reflow::default(), "the headings' size moves lines");
    assert_eq!(engine.compare(&again, &unwatched, &HashMap::new()), Some(expected));
}
