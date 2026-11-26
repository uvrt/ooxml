//! The layout box by box (phase 3 identity design, section 5 item 4): every page of every
//! document of the oracle's trees (`corpus/` and `fixtures/`) laid out and painted with the
//! open faces the repository ships, and written down per page -- its frame, each line with
//! its spans (glyph runs), each rule, picture, drawing slot and text box, each with the
//! name it is drawn under (`data-docx-path`, and a line's `data-docx-id`), each box's exact
//! geometry and paint as a digest, and the page's SVG as a digest -- against a golden
//! recorded before block identity replaced the layout's paths.
//!
//! The golden (`tests/data/laid-digest.txt.gz`) names the layout it was recorded from on its
//! first line. Bless it again, only for a change of layout that is meant, with
//! `LAID_DIGEST_BLESS=1 cargo test -p ooxml-word-pages --test laid_digest`, and say why in
//! the commit.
//!
//! A digest is over the fields a box had when the golden was recorded, named one by one:
//! a field added to a box later (its block's identity) is not in it, so the golden holds
//! across such a change and still sees every name and every length.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ooxml_derive::CacheBudget;
use ooxml_fonts::{Folder, Location, WordFontOptions};
use ooxml_word_layout::paint::drawings::DrawingSlot;
use ooxml_word_layout::paint::text_box::TextBoxPaint;
use ooxml_word_layout::paint::{Drawn, PagePaint, PaintedLine, Rule, Span};
use ooxml_word_model::model::Graphic;
use ooxml_word_pages::{DiskFonts, EngineBudgets, EngineOptions, FontSetup, PageEngine};
use std::fmt::Write as _;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
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

/// Items recorded, and those of them anchored (named at the edge through the version's
/// names): `[all, anchored]` of a kind.
struct Counted([AtomicUsize; 2]);

impl Counted {
    const fn new() -> Counted {
        Counted([AtomicUsize::new(0), AtomicUsize::new(0)])
    }

    fn add(&self, anchored: bool) {
        self.0[0].fetch_add(1, Ordering::Relaxed);
        if anchored {
            self.0[1].fetch_add(1, Ordering::Relaxed);
        }
    }

    fn count(&self, all: usize, anchored: usize) {
        self.0[0].fetch_add(all, Ordering::Relaxed);
        self.0[1].fetch_add(anchored, Ordering::Relaxed);
    }

    /// `(anchored, all)`.
    fn get(&self) -> (usize, usize) {
        (self.0[1].load(Ordering::Relaxed), self.0[0].load(Ordering::Relaxed))
    }
}

/// Body and story lines (their paragraph's key beside their path).
static LINES: Counted = Counted::new();
/// Their spans (a run, a label or a line number).
static SPANS: Counted = Counted::new();
/// Rules (paragraph, cell and table rules, decorations, column separators).
static RULES: Counted = Counted::new();
/// Text boxes' lines, named below a drawing that is anchored.
static BOXED: Counted = Counted::new();
/// Lines and rules, and those of them of a separator the layout makes (Word's own, where
/// the notes part names none): no block of the version, so not anchored.
static MADE: Counted = Counted::new();
/// The body blocks the pages' facts say each page draws something of (`Page.blocks`),
/// anchored beside their paths.
static BLOCKS: Counted = Counted::new();

/// Whether `path` is a separator paragraph the layout makes (`notes::default_separator`).
fn made_separator(path: &str) -> bool {
    matches!(
        path,
        "w:footnotes/separator"
            | "w:footnotes/continuationSeparator"
            | "w:endnotes/separator"
            | "w:endnotes/continuationSeparator"
    )
}

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/laid-digest.txt.gz");

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

/// The first 8 hex digits of the SHA-1 of `text`: a box's digest.
fn digest(text: &str) -> String {
    hex(text, 4)
}

/// The first `2 * bytes` hex digits of the SHA-1 of `text`.
fn hex(text: &str, bytes: usize) -> String {
    ooxml_base::sha1::digest(text.as_bytes())
        .iter()
        .take(bytes)
        .fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

/// A name as one word of a record (`-` none; spaces never occur in a path or id).
fn word(name: Option<&str>) -> String {
    match name {
        Some(n) if !n.is_empty() => n.replace(' ', "\\s"),
        _ => "-".to_owned(),
    }
}

fn span(out: &mut String, s: &Span) {
    let Span {
        path,
        chars,
        xs,
        y,
        end,
        face,
        bold,
        italic,
        half_points,
        color,
        underline,
        strike,
        double_strike,
        highlight,
        shading,
        kind,
        line_baseline,
        vertical_align,
        effects,
        ..
    } = s;
    let fields = format!(
        "{chars:?}|{xs:?}|{y}|{end:?}|{face:?}|{bold}|{italic}|{half_points}|{color:?}|{underline:?}|{strike}|\
         {double_strike}|{highlight:?}|{shading:?}|{kind:?}|{line_baseline}|{vertical_align:?}|{effects:?}"
    );
    let _ = writeln!(out, "  span {} {}", word(Some(path)), digest(&fields));
}

fn line(out: &mut String, l: &PaintedLine) {
    let PaintedLine {
        path,
        paragraph_id,
        number,
        baseline,
        top,
        pitch,
        column,
        band,
        spans,
        story,
        ..
    } = l;
    let fields = format!("{number}|{baseline}|{top:?}|{pitch:?}|{column:?}|{band:?}|{story:?}");
    let _ = writeln!(
        out,
        " line {} {} {}",
        word(Some(path)),
        word(paragraph_id.as_deref()),
        digest(&fields)
    );
    for s in spans {
        span(out, s);
    }
}

fn rule(out: &mut String, r: &Rule) {
    let Rule {
        kind,
        path,
        x,
        y,
        width,
        height,
        color,
        style,
        ..
    } = r;
    let fields = format!("{x:?}|{y:?}|{width:?}|{height:?}|{color:?}|{style:?}");
    let _ = writeln!(out, " rule {kind} {} {}", word(Some(path)), digest(&fields));
}

fn picture(out: &mut String, d: &Drawn) {
    let Drawn {
        path,
        relationship,
        part,
        x,
        y,
        width,
        height,
        crop,
        ..
    } = d;
    let fields = format!("{relationship:?}|{part:?}|{x:?}|{y:?}|{width:?}|{height:?}|{crop:?}");
    let _ = writeln!(out, " picture {} {}", word(Some(path)), digest(&fields));
}

/// What a drawing shows, without its text box's blocks (their lines are the text box's
/// paint) or the element it was read from (its paint is the page's SVG).
fn graphic(g: &Graphic) -> String {
    let Graphic {
        kind,
        offset,
        extent,
        rotation,
        flip_h,
        flip_v,
        relationship,
        text,
        body,
        children,
        child_offset,
        child_extent,
        unsupported,
        uri,
        path,
        source,
        ..
    } = g;
    let children: Vec<String> = children.iter().map(|c| graphic(c)).collect();
    format!(
        "{kind:?}|{offset:?}|{extent:?}|{rotation}|{flip_h}|{flip_v}|{relationship:?}|{}|{body:?}|{children:?}|\
         {child_offset:?}|{child_extent:?}|{unsupported:?}|{uri:?}|{path:?}|{}",
        text.len(),
        source.is_some()
    )
}

fn slot(out: &mut String, indent: &str, s: &DrawingSlot) {
    let DrawingSlot {
        path,
        paragraph_path,
        run,
        index,
        inline,
        x,
        y,
        width,
        height,
        layer,
        order,
        story,
        graphic: shown,
        no_overlap,
        ..
    } = s;
    let shown = shown.as_deref().map(graphic);
    let fields = format!(
        "{run}|{index}|{inline}|{x:?}|{y:?}|{width:?}|{height:?}|{layer:?}|{order:?}|{story:?}|{shown:?}|{no_overlap}"
    );
    let _ = writeln!(
        out,
        "{indent}drawing {} {} {}",
        word(Some(path)),
        word(Some(paragraph_path)),
        digest(&fields)
    );
}

fn text_box(out: &mut String, t: &TextBoxPaint) {
    let _ = writeln!(out, " text-box {}", t.slot);
    for l in &t.lines {
        out.push(' ');
        line(out, l);
    }
    for r in &t.rules {
        out.push(' ');
        rule(out, r);
    }
    for d in &t.pictures {
        out.push(' ');
        picture(out, d);
    }
    for s in &t.drawings {
        slot(out, "  ", s);
    }
    for u in &t.unpainted {
        let _ = writeln!(out, "  unpainted {}", word(Some(u)));
    }
}

/// Page `k`'s record: its frame and every box, then its SVG.
fn page(out: &mut String, k: usize, paint: &PagePaint, svg: &str) {
    let g = &paint.geometry;
    let drawn: Vec<String> = g
        .lines
        .iter()
        .map(|l| {
            format!(
                "{:?}|{:?}|{:?}|{}|{:?}|{:?}|{}|{:?}|{:?}|{:?}|{}",
                l.source, l.block, l.body, l.number, l.top, l.pitch, l.baseline, l.band, l.column, l.left, l.white_text
            )
        })
        .collect();
    let frame = format!("{}|{:?}|{:?}|{drawn:?}", g.page, g.info, g.frame);
    let _ = writeln!(out, "page {} {} svg {}", k + 1, hex(&frame, 8), hex(svg, 8));
    for l in &paint.lines {
        LINES.add(l.anchor.is_some());
        MADE.add(l.anchor.is_none() && made_separator(&l.path));
        for s in &l.spans {
            SPANS.add(s.anchor.is_some());
        }
        line(out, l);
    }
    for r in &paint.rules {
        RULES.add(r.anchor.is_some());
        MADE.add(r.anchor.is_none() && made_separator(r.path.split("/w:r[").next().unwrap_or_default()));
        rule(out, r);
    }
    for t in &paint.text_boxes {
        let anchored = paint.drawings.get(t.slot).is_some_and(|d| d.anchor.is_some());
        for _ in &t.lines {
            BOXED.add(anchored);
        }
    }
    for d in &paint.pictures {
        picture(out, d);
    }
    for s in &paint.drawings {
        slot(out, " ", s);
    }
    for t in &paint.text_boxes {
        text_box(out, t);
    }
    for u in &paint.unpainted {
        let _ = writeln!(out, " unpainted {}", word(Some(u)));
    }
    for u in &paint.unplaced {
        let _ = writeln!(out, " unplaced {}", word(Some(u)));
    }
}

fn docx_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            docx_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "docx") {
            out.push(path);
        }
    }
}

/// Document `file`'s record (named `name`) and its page count.
fn document(name: &str, file: &Path) -> (String, usize) {
    let mut out = String::new();
    let bytes = std::fs::read(file).unwrap();
    // one engine per document: what a document laid out first is laid out from
    let mut engine = engine();
    match engine.lay_out_bytes(Arc::from(bytes)) {
        Err(e) => {
            let _ = writeln!(out, "document {name} error {}", word(Some(&e.to_string())));
            (out, 0)
        }
        Ok(Err(fallback)) => {
            let _ = writeln!(
                out,
                "document {name} fallback {} {}",
                fallback.reason.replace(' ', "-"),
                word(Some(&fallback.at))
            );
            (out, 0)
        }
        Ok(Ok(version)) => {
            let count = version.page_count();
            let _ = writeln!(out, "document {name} pages {count}");
            for k in 0..count {
                let paint = engine.paint(&version, k).unwrap();
                let (svg, _) = engine.page_svg(&version, k).unwrap();
                page(&mut out, k, &paint, &svg);
            }
            // the facts name each page's blocks by anchor (asserted against their paths in a
            // debug build); not recorded: the golden is the paint's
            for facts in &version.facts().pages {
                BLOCKS.count(facts.blocks.len(), facts.anchors.len());
            }
            (out, count)
        }
    }
}

/// Every document's record, and how many documents and pages it holds: the documents
/// shared out among a few threads, their records joined in name order.
fn record() -> (String, usize, usize) {
    let oracle = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ooxml-oracle");
    let mut files = Vec::new();
    for tree in ["corpus", "fixtures"] {
        docx_files(&oracle.join(tree), &mut files);
    }
    files.sort();
    let named: Vec<(String, &PathBuf)> = files
        .iter()
        .map(|f| (f.strip_prefix(&oracle).unwrap().to_string_lossy().replace('\\', "/"), f))
        .collect();
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 6));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut done: Vec<(usize, String, usize)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((name, file)) = named.get(k) else { break };
                        let (text, pages) = document(name, file);
                        mine.push((k, text, pages));
                    }
                    mine
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().unwrap()).collect()
    });
    done.sort_by_key(|(k, _, _)| *k);
    let pages = done.iter().map(|(_, _, p)| p).sum();
    let out = done.into_iter().map(|(_, text, _)| text).collect();
    (out, files.len(), pages)
}

/// The golden's records, past its first line (what it was recorded from).
fn golden() -> Option<(String, String)> {
    let bytes = std::fs::read(GOLDEN).ok()?;
    let mut text = String::new();
    flate2::read::GzDecoder::new(&bytes[..])
        .read_to_string(&mut text)
        .ok()?;
    let (source, records) = text.split_once('\n')?;
    Some((source.to_owned(), records.to_owned()))
}

fn bless(records: &str) {
    let head = std::process::Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map_or_else(|| "unknown".to_owned(), |s| s.trim().to_owned());
    let source = format!(
        "# laid-digest: layout at {head}; bless: LAID_DIGEST_BLESS=1 cargo test -p ooxml-word-pages --test laid_digest"
    );
    let file = std::fs::File::create(GOLDEN).unwrap();
    let mut gz = flate2::write::GzEncoder::new(file, flate2::Compression::best());
    gz.write_all(format!("{source}\n{records}").as_bytes()).unwrap();
    gz.finish().unwrap();
}

#[test]
fn every_page_of_every_oracle_document_is_laid_out_box_by_box_as_recorded() {
    let (records, documents, pages) = record();
    assert!(
        documents > 250 && pages > documents,
        "{documents} documents, {pages} pages"
    );
    // the SVG and the facts name every anchored item through the version's names,
    // asserting its old path in a debug build
    for (what, counted) in [
        ("lines", &LINES),
        ("spans", &SPANS),
        ("rules", &RULES),
        ("text-box lines", &BOXED),
        ("page blocks", &BLOCKS),
    ] {
        let (anchored, all) = counted.get();
        eprintln!("{anchored} of {all} {what} anchored");
    }
    // every line and rule is anchored but a separator's the layout makes (of no block);
    // every span, text-box line and page block is
    let (made, _) = MADE.get();
    let ((lines_anchored, lines), (rules_anchored, rules)) = (LINES.get(), RULES.get());
    eprintln!("{made} lines and rules of separators the layout makes");
    assert_eq!(
        lines_anchored + rules_anchored + made,
        lines + rules,
        "lines and rules anchored"
    );
    for (what, counted) in [("spans", &SPANS), ("text-box lines", &BOXED), ("page blocks", &BLOCKS)] {
        let (anchored, all) = counted.get();
        assert_eq!(anchored, all, "{what} anchored");
    }
    if std::env::var_os("LAID_DIGEST_BLESS").is_some() {
        bless(&records);
        return;
    }
    let (source, golden) = golden().expect("the golden: bless it with LAID_DIGEST_BLESS=1");
    if golden == records {
        return;
    }
    // the first document that differs, line by line
    let (mut want, mut got) = (golden.lines(), records.lines());
    let mut document = "";
    for n in 1.. {
        match (want.next(), got.next()) {
            (Some(a), Some(b)) if a == b => {
                if a.starts_with("document ") {
                    document = a;
                }
            }
            (a, b) => panic!(
                "the layout differs from the golden ({source}) at record {n}, in {document}:\n  golden: {a:?}\n  now:    {b:?}"
            ),
        }
    }
}
