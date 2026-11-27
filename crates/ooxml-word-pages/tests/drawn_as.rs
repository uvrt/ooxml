//! A page drawn as one of the version before draws the same (lane CF1a): over random edits --
//! paragraphs inserted, deleted and split in the body, in a table's cells and in a text box,
//! table rows inserted and deleted, a paragraph's text changed -- every page of a version the
//! engine laid out again from the one before that keeps a page's drawing identity
//! (`LaidPage::drawn_as`: a page copied after a join, moved only in its flow's places) is
//! painted exactly as a fresh engine's layout of the version paints that page, and its
//! summary (path-free: lines by their blocks' ids), taken as digested before, is the fresh
//! layout's.
//! Fixed seeds here; `many_seeds` (ignored) runs more.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_derive::CacheBudget;
use ooxml_fonts::{Folder, Location, WordFontOptions};
use ooxml_word_pages::{DiskFonts, EngineBudgets, EngineOptions, FontSetup, PageEngine};
use ooxml_zip::{Method, NewEntry, Writer};
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
    fn pick(&mut self, n: usize) -> usize {
        usize::try_from(self.below(u64::try_from(n).unwrap())).unwrap()
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// A made-up block: a paragraph of words, a table of rows of cells of paragraphs, or a
/// paragraph anchoring a text box of paragraphs.
#[derive(Clone, Debug)]
enum Block {
    Para(u64),
    Table(Vec<Vec<Vec<u64>>>),
    Boxed(u64, Vec<u64>),
}

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape""#;

const WORDS: [&str; 8] = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "theta", "kappa"];

fn paragraph(words: u64, salt: usize) -> String {
    let text: Vec<&str> = (0..words)
        .map(|k| WORDS[(usize::try_from(k).unwrap() + salt) % WORDS.len()])
        .collect();
    format!(
        r#"<w:p><w:r><w:t xml:space="preserve">{salt} {}</w:t></w:r></w:p>"#,
        text.join(" ")
    )
}

fn text_box(words: u64, inside: &[u64], k: usize) -> String {
    let content: String = inside.iter().enumerate().map(|(n, w)| paragraph(*w, k + n)).collect();
    let run = format!(
        r#"<w:r><w:drawing><wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="{k}" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>1270000</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="1371600" cy="762000"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapSquare wrapText="bothSides"/><wp:docPr id="{k}" name="d{k}"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1371600" cy="762000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="DDEEFF"/></a:solidFill></wps:spPr><wps:txbx><w:txbxContent>{content}</w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"#
    );
    paragraph(words, k).replacen("</w:p>", &format!("{run}</w:p>"), 1)
}

fn table(rows: &[Vec<Vec<u64>>], k: usize) -> String {
    let mut out = String::from(
        r#"<w:tbl><w:tblPr><w:tblW w:w="4000" w:type="dxa"/><w:tblBorders><w:top w:val="single" w:sz="4" w:color="000000"/><w:bottom w:val="single" w:sz="4" w:color="000000"/><w:insideH w:val="single" w:sz="4" w:color="000000"/></w:tblBorders></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>"#,
    );
    for (r, row) in rows.iter().enumerate() {
        out.push_str("<w:tr>");
        for (c, cell) in row.iter().enumerate() {
            let shade = if (r + c) % 3 == 0 {
                r#"<w:shd w:val="clear" w:color="auto" w:fill="EEDDCC"/>"#
            } else {
                ""
            };
            out.push_str(&format!(
                r#"<w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/>{shade}</w:tcPr>"#
            ));
            for (n, words) in cell.iter().enumerate() {
                out.push_str(&paragraph(*words, k + r * 7 + c * 3 + n));
            }
            out.push_str("</w:tc>");
        }
        out.push_str("</w:tr>");
    }
    out.push_str("</w:tbl>");
    out
}

#[derive(Clone, Debug)]
struct Doc(Vec<Block>);

impl Doc {
    fn new(rng: &mut Rng) -> Doc {
        let count = 24 + rng.pick(16);
        Doc((0..count).map(|_| Doc::block(rng)).collect())
    }

    fn block(rng: &mut Rng) -> Block {
        match rng.below(10) {
            0 => Block::Table(
                (0..2 + rng.pick(3))
                    .map(|_| (0..2).map(|_| vec![rng.below(14); 1 + rng.pick(2)]).collect())
                    .collect(),
            ),
            1 => Block::Boxed(rng.below(40), (0..1 + rng.pick(3)).map(|_| rng.below(10)).collect()),
            _ => Block::Para(rng.below(70)),
        }
    }

    /// One random edit; `false` where it found nothing to do.
    fn edit(&mut self, rng: &mut Rng) -> bool {
        let n = self.0.len();
        let k = rng.pick(n);
        match (rng.below(9), &mut self.0[k]) {
            (0, _) => self.0.insert(k, Block::Para(rng.below(70))),
            (1, _) if n > 6 => {
                self.0.remove(k);
            }
            (2, Block::Para(words)) if *words > 4 => {
                // split
                let rest = *words / 2;
                *words -= rest;
                self.0.insert(k + 1, Block::Para(rest));
            }
            (3, Block::Para(words)) => *words = rng.below(70),
            (4, Block::Table(rows)) => {
                let (r, c) = (rng.pick(rows.len()), rng.pick(2));
                let cell = &mut rows[r][c];
                if cell.len() > 1 && rng.chance(50) {
                    cell.remove(rng.pick(cell.len()));
                } else {
                    let at = rng.pick(cell.len() + 1);
                    cell.insert(at, rng.below(14));
                }
            }
            (5, Block::Table(rows)) => {
                if rows.len() > 1 && rng.chance(50) {
                    rows.remove(rng.pick(rows.len()));
                } else {
                    let at = rng.pick(rows.len() + 1);
                    rows.insert(at, vec![vec![rng.below(14)], vec![rng.below(14)]]);
                }
            }
            (6, Block::Boxed(_, inside)) => {
                if inside.len() > 1 && rng.chance(50) {
                    inside.remove(rng.pick(inside.len()));
                } else {
                    let at = rng.pick(inside.len() + 1);
                    inside.insert(at, rng.below(10));
                }
            }
            (7, _) => self.0.insert(k, Doc::block(rng)),
            (8, Block::Boxed(words, _)) => *words = rng.below(40),
            _ => return false,
        }
        true
    }

    fn package(&self) -> Arc<[u8]> {
        let mut body = String::new();
        for (k, block) in self.0.iter().enumerate() {
            body.push_str(&match block {
                Block::Para(words) => paragraph(*words, k),
                Block::Table(rows) => table(rows, k),
                Block::Boxed(words, inside) => text_box(*words, inside, k + 1),
            });
        }
        body.push_str(r#"<w:sectPr><w:pgSz w:w="5760" w:h="5040"/><w:pgMar w:top="720" w:right="540" w:bottom="720" w:left="540" w:header="300" w:footer="300" w:gutter="0"/></w:sectPr>"#);
        let entry = |name: &str| NewEntry {
            name: name.into(),
            method: Method::Deflated,
            dos_time: 0x6000,
            dos_date: 0x5a21,
            external_attr: 0,
            create_system: 0,
        };
        let files = [
            (
                "[Content_Types].xml",
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#.to_owned(),
            ),
            (
                "_rels/.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_owned(),
            ),
            (
                "word/document.xml",
                format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W}><w:body>{body}</w:body></w:document>"#),
            ),
        ];
        let mut w = Writer::new();
        for (name, data) in files {
            w.add(&entry(name), data.as_bytes()).unwrap();
        }
        Arc::from(w.finish(b"").unwrap())
    }
}

/// Runs `steps` edits from `seed`; the pages kept drawn as one before, checked.
fn run_seed(seed: u64, steps: usize) -> usize {
    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut doc = Doc::new(&mut rng);
    let mut lazy = engine();
    let mut before = lazy.lay_out_bytes(doc.package()).unwrap().unwrap();
    let mut old = lazy.summary(&before).unwrap();
    let mut kept = 0;
    for step in 0..steps {
        while !doc.edit(&mut rng) {}
        let bytes = doc.package();
        let version = lazy.lay_out_bytes(Arc::clone(&bytes)).unwrap().unwrap();
        let mut fresh = engine();
        let alone = fresh.lay_out_bytes(Arc::clone(&bytes)).unwrap().unwrap();
        assert_eq!(version.page_count(), alone.page_count(), "seed {seed} step {step}");
        // each page kept drawn as one before is taken as digested before
        let summary = lazy.summary(&version).unwrap();
        let from_scratch = fresh.summary(&alone).unwrap();
        for k in 0..version.page_count() {
            let drawn_as = version.layout().laid_page(k).map(|p| Arc::clone(&p.drawn_as));
            let was = drawn_as.and_then(|d| {
                (0..before.page_count()).find(|j| {
                    before
                        .layout()
                        .laid_page(*j)
                        .is_some_and(|p| Arc::ptr_eq(&p.drawn_as, &d))
                })
            });
            let Some(j) = was else { continue };
            kept += 1;
            let at = format!("seed {seed} step {step}: page {k} drawn as page {j} before, {doc:?}");
            assert_eq!(summary.pages[k], from_scratch.pages[k], "{at}");
            assert_eq!(old.pages[j], from_scratch.pages[k], "{at}");
            let painted = lazy.paint(&version, k).unwrap();
            assert_eq!(*painted, *fresh.paint(&alone, k).unwrap(), "{at}");
        }
        before = version;
        old = summary;
    }
    kept
}

#[test]
fn a_page_drawn_as_one_before_draws_what_a_fresh_layout_draws() {
    let kept: usize = (1..=3).map(|seed| run_seed(seed, 8)).sum();
    println!("{kept} pages kept their drawing identity");
    assert!(
        kept > 0,
        "no page kept its drawing identity: the property was not exercised"
    );
}

#[test]
#[ignore = "many seeds: run by hand (cargo test -- --ignored)"]
fn many_seeds() {
    let kept: usize = (1..=40).map(|seed| run_seed(seed, 14)).sum();
    println!("{kept} pages kept their drawing identity");
    assert!(kept > 0);
}

#[test]
fn an_insert_before_an_early_paragraph_digests_only_the_pages_it_changed() {
    let mut doc = Doc((0..80).map(|k| Block::Para(20 + (k * 7) % 50)).collect());
    let mut engine = engine();
    let first = engine.lay_out_bytes(doc.package()).unwrap().unwrap();
    let _ = engine.summary(&first);
    doc.0.insert(1, Block::Para(3));
    let second = engine.lay_out_bytes(doc.package()).unwrap().unwrap();
    assert!(second.relaid(), "laid out again from the version before");
    let digested = engine.stats().pages_digested.0;
    let found = engine
        .compare(&second, &first, &std::collections::HashMap::new())
        .unwrap();
    let painted = engine.stats().pages_digested.0 - digested;
    let pages = second.page_count();
    assert!(pages > 10, "{pages} pages");
    assert!(
        painted <= u64::try_from(found.changed.len()).unwrap() + 2,
        "{painted} pages digested for {} changed of {pages}: {found:?}",
        found.changed.len()
    );
}
