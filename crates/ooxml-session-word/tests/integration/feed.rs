//! The changes feed (lane CF1a): a client that starts empty and applies each answer -- the
//! page list, each changed page's groups, offsets, removals and order -- rebuilds every page's
//! feed-mode SVG byte for byte, over random edits (paragraphs, table rows and cells, text
//! boxes inserted, deleted, split and rewritten), undos and reflows; one letter sends one
//! group; a stale `since` (another session's, past the head, below the floor) gets
//! everything; an undo sends only its pages; parts and blocks without laying out; readers
//! during six writers each answered from one version; a small memory budget held, a page let
//! go written again correctly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{layout, Never, NoFonts, NoParts};
use crate::editing::{call, context};
use ooxml_base::{CacheBudget, Context};
use ooxml_session::{CallContext, DocId, DocKind, Session, SessionConfig};
use ooxml_session_word::prelude::*;
use ooxml_session_word::Changes;
use ooxml_tools_word::{EnginePages, WordLayout};
use ooxml_word_layout::paint::feed::{feed_svg, FeedFrame, FeedGroup};
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape""#;

const WORDS: [&str; 8] = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "theta", "kappa"];

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
}

/// A made-up paragraph: its id and words.
#[derive(Clone, Debug)]
struct Para(u32, u64);

#[derive(Clone, Debug)]
enum Block {
    Para(Para),
    /// Rows (each with its id) of two cells of paragraphs.
    Table(Vec<(u32, [Vec<Para>; 2])>),
    /// A paragraph anchoring a text box of paragraphs.
    Boxed(Para, Vec<Para>),
}

/// A made-up document whose blocks carry ids of their own.
#[derive(Clone, Debug)]
struct Doc {
    blocks: Vec<Block>,
    next: u32,
}

fn text(Para(id, words): &Para) -> String {
    let salt = usize::try_from(*id).unwrap();
    let words: Vec<&str> = (0..*words)
        .map(|k| WORDS[(usize::try_from(k).unwrap() + salt) % WORDS.len()])
        .collect();
    words.join(" ")
}

fn paragraph(p: &Para, extra: &str) -> String {
    format!(
        r#"<w:p w14:paraId="{:08X}" w14:textId="77777777"><w:r><w:t xml:space="preserve">{}</w:t></w:r>{extra}</w:p>"#,
        p.0,
        text(p)
    )
}

impl Doc {
    fn new(rng: &mut Rng) -> Doc {
        let mut doc = Doc {
            blocks: Vec::new(),
            next: 0x1000_0000,
        };
        for _ in 0..22 + rng.pick(10) {
            let block = doc.block(rng);
            doc.blocks.push(block);
        }
        doc
    }

    fn id(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    fn para(&mut self, rng: &mut Rng, most: u64) -> Para {
        Para(self.id(), rng.below(most))
    }

    fn block(&mut self, rng: &mut Rng) -> Block {
        match rng.below(10) {
            0 => {
                let rows = (0..2 + rng.pick(2))
                    .map(|_| {
                        let row = self.id();
                        (row, [vec![self.para(rng, 12)], vec![self.para(rng, 12)]])
                    })
                    .collect();
                Block::Table(rows)
            }
            1 => {
                let anchor = self.para(rng, 40);
                let inside = (0..1 + rng.pick(2)).map(|_| self.para(rng, 8)).collect();
                Block::Boxed(anchor, inside)
            }
            _ => Block::Para(self.para(rng, 60)),
        }
    }

    /// One random edit; `false` where it found nothing to do.
    fn edit(&mut self, rng: &mut Rng) -> bool {
        let n = self.blocks.len();
        let k = rng.pick(n);
        let kind = rng.below(8);
        let fresh = self.para(rng, 60);
        let fresh_small = Para(self.id(), rng.below(10));
        let row_id = self.id();
        match (kind, &mut self.blocks[k]) {
            (0, _) => self.blocks.insert(k, Block::Para(fresh)),
            (1, _) if n > 6 => {
                self.blocks.remove(k);
            }
            (2, Block::Para(p)) if p.1 > 4 => {
                let rest = p.1 / 2;
                p.1 -= rest;
                self.blocks.insert(k + 1, Block::Para(Para(fresh.0, rest)));
            }
            (3, Block::Para(p)) => p.1 = rng.below(60),
            (4, Block::Table(rows)) => {
                let r = rng.pick(rows.len());
                let cell = &mut rows[r].1[rng.pick(2)];
                if cell.len() > 1 && rng.below(2) == 0 {
                    cell.remove(rng.pick(cell.len()));
                } else {
                    cell.insert(rng.pick(cell.len() + 1), fresh_small);
                }
            }
            (5, Block::Table(rows)) => {
                if rows.len() > 1 && rng.below(2) == 0 {
                    rows.remove(rng.pick(rows.len()));
                } else {
                    let other = Para(fresh.0, 3);
                    rows.insert(rng.pick(rows.len() + 1), (row_id, [vec![fresh_small], vec![other]]));
                }
            }
            (6, Block::Boxed(_, inside)) => {
                if inside.len() > 1 && rng.below(2) == 0 {
                    inside.remove(rng.pick(inside.len()));
                } else {
                    inside.insert(rng.pick(inside.len() + 1), fresh_small);
                }
            }
            (7, Block::Boxed(anchor, _)) => anchor.1 = rng.below(40),
            _ => return false,
        }
        true
    }

    fn body(&self) -> String {
        let mut out = String::new();
        for (k, block) in self.blocks.iter().enumerate() {
            match block {
                Block::Para(p) => out.push_str(&paragraph(p, "")),
                Block::Table(rows) => {
                    out.push_str(r#"<w:tbl><w:tblPr><w:tblW w:w="4000" w:type="dxa"/><w:tblBorders><w:top w:val="single" w:sz="4" w:color="000000"/><w:bottom w:val="single" w:sz="4" w:color="000000"/><w:insideH w:val="single" w:sz="4" w:color="000000"/></w:tblBorders></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>"#);
                    for (id, cells) in rows {
                        out.push_str(&format!(r#"<w:tr w14:paraId="{id:08X}" w14:textId="77777777">"#));
                        for (c, cell) in cells.iter().enumerate() {
                            let shade = if c == 0 {
                                r#"<w:shd w:val="clear" w:color="auto" w:fill="EEDDCC"/>"#
                            } else {
                                ""
                            };
                            out.push_str(&format!(
                                r#"<w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/>{shade}</w:tcPr>"#
                            ));
                            for p in cell {
                                out.push_str(&paragraph(p, ""));
                            }
                            out.push_str("</w:tc>");
                        }
                        out.push_str("</w:tr>");
                    }
                    out.push_str("</w:tbl>");
                }
                Block::Boxed(anchor, inside) => {
                    let content: String = inside.iter().map(|p| paragraph(p, "")).collect();
                    let drawing = format!(
                        r#"<w:r><w:drawing><wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="{k}" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>1270000</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="1371600" cy="762000"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapSquare wrapText="bothSides"/><wp:docPr id="{}" name="d{k}"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1371600" cy="762000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="DDEEFF"/></a:solidFill></wps:spPr><wps:txbx><w:txbxContent>{content}</w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"#,
                        anchor.0
                    );
                    out.push_str(&paragraph(anchor, &drawing));
                }
            }
        }
        out.push_str(r#"<w:sectPr><w:pgSz w:w="5760" w:h="5040"/><w:pgMar w:top="720" w:right="540" w:bottom="720" w:left="540" w:header="300" w:footer="300" w:gutter="0"/></w:sectPr>"#);
        out
    }

    fn document(&self) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>{}</w:body></w:document>"#,
            self.body()
        )
    }

    fn package(&self) -> Arc<[u8]> {
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
        let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
        let mut w = Writer::new();
        for (name, data) in [
            ("[Content_Types].xml", types.to_owned()),
            ("_rels/.rels", rels.to_owned()),
            ("word/document.xml", self.document()),
        ] {
            let entry = NewEntry {
                name: name.into(),
                method: Method::Deflated,
                dos_time: 0,
                dos_date: 33,
                external_attr: 0,
                create_system: 3,
            };
            w.add(&entry, data.as_bytes()).unwrap();
        }
        Arc::from(w.finish(b"").unwrap())
    }

    /// Put this body as the document's next version.
    fn put(&self, s: &Session, d: &DocId) {
        let parts = vec![(
            "word/document.xml".to_owned(),
            Some(Arc::from(self.document().as_bytes())),
        )];
        s.update(d, parts, &CallContext::default()).unwrap();
    }
}

/// A client of the feed: each page's frame, order and groups, as it was sent them.
#[derive(Default)]
struct Client {
    token: Option<String>,
    frames: Vec<FeedFrame>,
    order: Vec<Vec<String>>,
    groups: Vec<HashMap<String, FeedGroup>>,
}

impl Client {
    fn apply(&mut self, changes: &Changes) {
        let pages = changes.pages.as_ref().unwrap();
        if changes.full {
            self.order.clear();
            self.groups.clear();
        }
        if let Some(list) = &pages.list {
            self.frames.clone_from(list);
            self.order.resize(list.len(), Vec::new());
            self.groups.resize(list.len(), HashMap::new());
        }
        for page in &pages.changed {
            let k = page.page - 1;
            let groups = &mut self.groups[k];
            for id in &page.removed {
                groups.remove(id);
            }
            let defs: HashMap<&str, &str> = page.defs.iter().map(|(id, d)| (id.as_str(), d.as_str())).collect();
            for g in &page.groups {
                groups.insert(
                    g.id.clone(),
                    FeedGroup {
                        id: g.id.clone(),
                        dy: g.dy,
                        content: g.svg.clone(),
                        defs: defs.get(g.id.as_str()).map_or_else(String::new, |d| (*d).to_owned()),
                        pictures: Vec::new(),
                    },
                );
            }
            for (id, dy) in &page.moved {
                groups.get_mut(id).unwrap().dy = *dy;
            }
            if let Some(order) = &page.order {
                self.order[k].clone_from(order);
            }
        }
        self.token = Some(changes.token.clone());
    }

    fn svg(&self, k: usize) -> String {
        feed_svg(&self.frames[k], self.order[k].iter().map(|id| &self.groups[k][id]))
    }
}

/// Every page of the head of `d` as the engine writes it for the feed.
fn expected(s: &Session, d: &DocId, layout: &EnginePages) -> Vec<String> {
    let head = s.snapshot(d).unwrap();
    let package = head.word().unwrap().package().clone();
    let laid = layout.laid(&package, &Never).unwrap();
    let mut engine = ooxml_word_pages::lock_engine(layout.engine());
    (0..laid.page_count())
        .map(|k| engine.feed_page(&laid, k).unwrap().0.svg())
        .collect()
}

fn changes(s: &Session, d: &DocId, since: Option<&str>, layout: &EnginePages) -> Changes {
    s.changes(d, since, true, layout, &CallContext::default()).unwrap()
}

/// The client caught up and checked page by page against the engine's own writing.
fn caught_up(s: &Session, d: &DocId, client: &mut Client, layout: &EnginePages, at: &str) -> Changes {
    let found = changes(s, d, client.token.as_deref(), layout);
    client.apply(&found);
    let pages = expected(s, d, layout);
    assert_eq!(client.frames.len(), pages.len(), "{at}");
    for (k, page) in pages.iter().enumerate() {
        assert_eq!(&client.svg(k), page, "page {}, {at}", k + 1);
    }
    found
}

fn opened(doc: &Doc) -> (Session, DocId, Arc<EnginePages>) {
    let s = Session::new(SessionConfig::default(), context(), ooxml_session_word::formats(None));
    let d = s.open(doc.package(), DocKind::Docx, "d.docx").unwrap();
    (s, d, layout())
}

fn replay(seed: u64, steps: usize) {
    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut doc = Doc::new(&mut rng);
    let (s, d, layout) = opened(&doc);
    let mut client = Client::default();
    let first = caught_up(&s, &d, &mut client, &layout, &format!("seed {seed}, first"));
    assert!(first.full);
    for step in 0..steps {
        let at = format!("seed {seed} step {step}");
        if step % 5 == 4 {
            let undone = call(&s, "undo", json!({"doc": d.to_string()}), &CallContext::default());
            assert!(undone.get("error").is_none(), "{undone}");
        } else {
            while !doc.edit(&mut rng) {}
            doc.put(&s, &d);
        }
        let found = caught_up(&s, &d, &mut client, &layout, &format!("{at}: {doc:?}"));
        assert!(!found.full, "{at}");
    }
}

#[test]
fn a_client_that_applies_the_feed_rebuilds_every_page() {
    for seed in 1..=3 {
        replay(seed, 10);
    }
}

#[test]
#[ignore = "many seeds: run by hand (cargo test -- --ignored)"]
fn a_client_that_applies_the_feed_rebuilds_every_page_many_seeds() {
    for seed in 1..=30 {
        replay(seed, 16);
    }
}

/// A document of `n` paragraphs, each of `words` words.
fn plain(n: u32, words: u64) -> Doc {
    Doc {
        blocks: (0..n).map(|k| Block::Para(Para(0x2000_0000 + k, words))).collect(),
        next: 0x3000_0000,
    }
}

#[test]
fn one_letter_sends_one_group_and_an_undo_only_its_pages() {
    let doc = plain(60, 30);
    let (s, d, layout) = opened(&doc);
    let mut client = Client::default();
    caught_up(&s, &d, &mut client, &layout, "first");
    // one letter: the paragraph's words one longer at most draws a letter more on one line
    let Block::Para(p) = &doc.blocks[30] else { panic!() };
    let edited = call(
        &s,
        "word_set_text",
        json!({"doc": d.to_string(), "target": format!("p:{:08X}", p.0), "text": format!("{}x", text(p))}),
        &CallContext::default(),
    );
    assert!(edited.get("error").is_none(), "{edited}");
    let letter = caught_up(&s, &d, &mut client, &layout, "one letter");
    let pages = letter.pages.as_ref().unwrap();
    assert_eq!(pages.changed.len(), 1, "{pages:?}");
    assert_eq!(pages.changed[0].groups.len(), 1);
    assert!(pages.changed[0].moved.is_empty() && pages.changed[0].removed.is_empty());
    assert!(pages.list.is_none());
    assert_eq!(letter.blocks.changed, vec![format!("p:{:08X}", p.0)]);
    assert_eq!(letter.parts, vec!["word/document.xml".to_owned()]);
    // its undo: only that page, that group
    let undone = call(&s, "undo", json!({"doc": d.to_string()}), &CallContext::default());
    assert!(undone.get("error").is_none(), "{undone}");
    let back = caught_up(&s, &d, &mut client, &layout, "undo");
    assert!(back.version > letter.version);
    let pages = back.pages.as_ref().unwrap();
    assert_eq!(pages.changed.len(), 1, "{pages:?}");
    assert_eq!(pages.changed[0].page, letter.pages.as_ref().unwrap().changed[0].page);
    assert_eq!(pages.changed[0].groups.len(), 1);
    // an insert at the start moves every page's lines; its undo sends those pages back, the
    // page it added gone from the list
    let mut grown = doc.clone();
    grown.blocks.insert(1, Block::Para(Para(0x3000_0001, 40)));
    grown.put(&s, &d);
    let inserted = caught_up(&s, &d, &mut client, &layout, "insert");
    let undone = call(&s, "undo", json!({"doc": d.to_string()}), &CallContext::default());
    assert!(undone.get("error").is_none(), "{undone}");
    let back = caught_up(&s, &d, &mut client, &layout, "undo of the insert");
    let pages_of = |c: &Changes| -> Vec<usize> { c.pages.as_ref().unwrap().changed.iter().map(|p| p.page).collect() };
    let count = back.pages.as_ref().unwrap().list.as_ref().map_or(usize::MAX, Vec::len);
    let expected: Vec<usize> = pages_of(&inserted).into_iter().filter(|p| *p <= count).collect();
    assert_eq!(pages_of(&back), expected);
}

#[test]
fn a_stale_since_gets_everything() {
    let doc = plain(20, 30);
    let (s, d, layout) = opened(&doc);
    let first = changes(&s, &d, None, &layout);
    let count = first.pages.as_ref().unwrap().changed.len();
    assert!(first.full && count > 1);
    for stale in [
        "0000000000000000-1".to_owned(),
        format!("{}-{}", s.nonce(), first.version + 5),
        "nonsense".to_owned(),
    ] {
        let found = changes(&s, &d, Some(&stale), &layout);
        assert!(found.full, "{stale}");
        let pages = found.pages.unwrap();
        assert_eq!(pages.changed.len(), count, "{stale}");
        assert!(pages.list.is_some());
    }
    let current = changes(&s, &d, Some(&first.token), &layout);
    assert!(!current.full && current.pages.unwrap().changed.is_empty());
}

#[test]
fn parts_and_blocks_are_answered_without_laying_out() {
    let doc = plain(8, 5);
    let (s, d, layout) = opened(&doc);
    assert!(doc.blocks.len() == 8);
    let first = s.changes(&d, None, false, &*layout, &CallContext::default()).unwrap();
    assert!(first.full && first.pages.is_none());
    assert!(first.parts.contains(&"word/document.xml".to_owned()));
    let mut next = doc.clone();
    next.blocks.remove(2);
    next.blocks.insert(0, Block::Para(Para(0x3000_0009, 4)));
    next.put(&s, &d);
    let found = s
        .changes(&d, Some(&first.token), false, &*layout, &CallContext::default())
        .unwrap();
    assert!(!found.full);
    assert_eq!(found.blocks.came, vec!["p:30000009".to_owned()]);
    assert_eq!(found.blocks.gone, vec!["p:20000002".to_owned()]);
    assert_eq!(found.parts, vec!["word/document.xml".to_owned()]);
    assert_eq!(
        ooxml_word_pages::lock_engine(layout.engine()).stats().full,
        0,
        "nothing laid out"
    );
}

#[test]
fn readers_during_six_writers_are_each_answered_from_one_version() {
    let doc = plain(40, 20);
    let (s, d, layout) = opened(&doc);
    let s = Arc::new(s);
    let stop = Arc::new(AtomicBool::new(false));
    let answers = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|scope| {
        for w in 0..6u32 {
            let (s, d, stop) = (Arc::clone(&s), d.clone(), Arc::clone(&stop));
            scope.spawn(move || {
                let mut n = 0;
                while !stop.load(Ordering::SeqCst) && n < 12 {
                    let target = format!("p:{:08X}", 0x2000_0000 + w * 6);
                    let text = WORDS[..1 + (n % 7)].join(" ");
                    let edited = call(
                        &s,
                        "word_set_text",
                        json!({"doc": d.to_string(), "target": target, "text": text}),
                        &CallContext::default(),
                    );
                    assert!(edited.get("error").is_none(), "{edited}");
                    n += 1;
                }
            });
        }
        for _ in 0..2 {
            let (s, d, layout, answers) = (Arc::clone(&s), d.clone(), Arc::clone(&layout), Arc::clone(&answers));
            scope.spawn(move || {
                let mut client = Client::default();
                let mut last = 0;
                for _ in 0..10 {
                    let found = changes(&s, &d, client.token.as_deref(), &layout);
                    assert!(found.version >= last, "an answer of an earlier version");
                    if let Some(head) = found.behind {
                        assert!(head > found.version);
                    }
                    last = found.version;
                    client.apply(&found);
                    answers.fetch_add(1, Ordering::SeqCst);
                }
            });
        }
    });
    stop.store(true, Ordering::SeqCst);
    assert!(answers.load(Ordering::SeqCst) >= 20);
    let mut client = Client::default();
    caught_up(&s, &d, &mut client, &layout, "after the writers");
    let again = caught_up(&s, &d, &mut client, &layout, "again");
    assert!(again.pages.unwrap().changed.is_empty());
}

/// A memory budget whose limit a test sets.
struct Limited {
    limit: usize,
    used: AtomicUsize,
}

impl CacheBudget for Limited {
    fn reserve(&self, bytes: usize) -> bool {
        let after = self.used.load(Ordering::SeqCst).saturating_add(bytes);
        let fits = after <= self.limit;
        if fits {
            self.used.store(after, Ordering::SeqCst);
        }
        fits
    }
    fn release(&self, bytes: usize) {
        self.used.fetch_sub(bytes, Ordering::SeqCst);
    }
    fn used(&self) -> usize {
        self.used.load(Ordering::SeqCst)
    }
}

#[test]
fn a_small_budget_is_held_and_a_page_let_go_is_written_again() {
    let mut doc = plain(50, 30);
    let budget = Arc::new(Limited {
        limit: 96 << 10,
        used: AtomicUsize::new(0),
    });
    let ports = Context {
        fonts: Arc::new(NoFonts),
        parts: Arc::new(NoParts),
        budget: Arc::clone(&budget) as Arc<dyn CacheBudget>,
        ..context()
    };
    let s = Session::new(SessionConfig::default(), ports, ooxml_session_word::formats(None));
    let d = s.open(doc.package(), DocKind::Docx, "d.docx").unwrap();
    let layout = layout();
    let mut client = Client::default();
    caught_up(&s, &d, &mut client, &layout, "first");
    assert!(budget.used() <= budget.limit);
    for step in 0..4 {
        let Block::Para(p) = &mut doc.blocks[5 + step * 10] else {
            panic!()
        };
        p.1 += 9;
        doc.put(&s, &d);
        let found = caught_up(&s, &d, &mut client, &layout, &format!("step {step}"));
        assert!(!found.full, "step {step}: the records were kept");
        assert!(budget.used() <= budget.limit, "{} of {}", budget.used(), budget.limit);
    }
    // every page again, from the records and pages written again where they were let go
    let mut fresh = Client::default();
    caught_up(&s, &d, &mut fresh, &layout, "a new client");
}
