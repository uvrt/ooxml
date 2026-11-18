//! The engine over a made-up face (every character 1000 units of 2048, `AV` kerned by
//! -100): advances in 1/4096 pt, greedy first fit at the edge, spaces past the edge,
//! emergency breaks, tabs, kerning charged to the left glyph, automatic hyphens, and the
//! memo.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::{Digest, DigestKey, Locale};
use ooxml_derive::CacheBudget;
use ooxml_text::{
    measure, Advance, CharFormat, FaceKey, Geometry, Glyphs, Hyphenation, Line, Measurer, Paragraph, PieceKind, Run,
    RunFormats, ScriptMetrics, TabAlign, Units, WordRules,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct Mono;

impl Glyphs for Mono {
    fn advance(&self, _: &FaceKey, ch: char) -> Option<Advance> {
        (ch != '\u{4e00}').then_some(Advance {
            advance: 1000,
            units_per_em: 2048,
        })
    }
    fn kern(&self, _: &FaceKey, left: char, right: char) -> Option<i32> {
        Some(if (left, right) == ('A', 'V') { -100 } else { 0 })
    }
    fn script_metrics(&self, _: &FaceKey) -> Option<ScriptMetrics> {
        None
    }
    fn epoch(&self) -> Digest {
        Digest::default()
    }
}

fn format() -> CharFormat {
    CharFormat {
        face: Some(FaceKey {
            family: "Mono".into(),
            bold: false,
            italic: false,
        }),
        half_points: 2,
        vertical_align: None,
    }
}

fn run(text: &str) -> Run {
    Run {
        text: text.into(),
        marks: Vec::new(),
        vanish: false,
        formats: RunFormats::Uniform(format()),
        hyphen_format: format(),
        tracking: 0,
        scale: 100,
        kern: 0,
        caps: false,
        note_number: false,
        lang: Some("en-US".into()),
        no_proof: false,
    }
}

fn paragraph(runs: Vec<Run>) -> Paragraph {
    Paragraph {
        runs,
        label: None,
        hyphenation: None,
    }
}

/// Every character is 1000 x 2 x 2048 / 2048 = 2000 units; a line `chars` characters wide.
fn geometry(chars: i64) -> Geometry {
    Geometry {
        first_start: Units::ZERO,
        start: Units::ZERO,
        right: Units::int(2000 * chars),
        tabs: Vec::new(),
        default_tab: Units::int(5000),
        hanging_stop: None,
        squeeze: false,
        tab_origin: Units::ZERO,
        hyphen_zone: None,
        hyphen_next: false,
        hyphen_gap: None,
        hyphen_limit: 0,
    }
}

fn spans(lines: &[Line]) -> Vec<(usize, usize)> {
    lines.iter().map(|l| (l.start, l.end)).collect()
}

#[test]
fn greedy_first_fit_at_the_edge_inclusive() {
    let m = measure(&Mono, &paragraph(vec![run("aaa bbb ccc")]), &geometry(7), false).unwrap();
    assert_eq!(m.pieces[0].width(), Units::int(2000));
    assert_eq!(m.pieces[3].kind, PieceKind::Space);
    // "aaa bbb" is exactly 7 characters: it fits, the space after it hangs.
    assert_eq!(spans(&m.lines), [(0, 8), (8, 11)]);
    let m = measure(&Mono, &paragraph(vec![run("abcdefghij")]), &geometry(4), false).unwrap();
    assert!(m.lines[0].emergency);
    assert_eq!(spans(&m.lines), [(0, 4), (4, 8), (8, 10)]);
}

#[test]
fn kerning_is_charged_to_the_left_glyph() {
    let mut r = run("AVA");
    r.kern = 2;
    let m = measure(&Mono, &paragraph(vec![r]), &geometry(10), false).unwrap();
    assert_eq!(m.pieces[0].kern_next(), Units::int(-200));
    assert_eq!(m.pieces[1].kern_next(), Units::ZERO);
    let unknown = measure(&Mono, &paragraph(vec![run("a\u{4e00}")]), &geometry(10), false);
    assert!(unknown.is_err());
}

#[test]
fn kerning_pairs_need_the_same_face_and_size_and_kerning_on() {
    // Two runs of the same face and size, both kerning: the pair across them kerns.
    let (mut a, mut v) = (run("A"), run("V"));
    (a.kern, v.kern) = (2, 2);
    let m = measure(&Mono, &paragraph(vec![a.clone(), v.clone()]), &geometry(10), false).unwrap();
    assert_eq!(m.pieces[0].kern_next(), Units::int(-200));
    // Kerning off on the right one, or only above its size, or another face: no pair.
    let mut off = v.clone();
    off.kern = 0;
    let mut above = v.clone();
    above.kern = 3;
    let mut other = v.clone();
    let mut bold = format();
    bold.face.as_mut().unwrap().bold = true;
    other.formats = RunFormats::Uniform(bold);
    for right in [off, above, other] {
        let m = measure(&Mono, &paragraph(vec![a.clone(), right]), &geometry(10), false).unwrap();
        assert_eq!(m.pieces[0].kern_next(), Units::ZERO);
    }
}

#[test]
fn pieces_are_small_and_share_their_run_s_format() {
    assert!(
        std::mem::size_of::<ooxml_text::Piece>() <= 104,
        "{}",
        std::mem::size_of::<ooxml_text::Piece>()
    );
    let m = measure(&Mono, &paragraph(vec![run("abc")]), &geometry(10), false).unwrap();
    let (first, last) = (m.pieces[0].format().unwrap(), m.pieces[2].format().unwrap());
    assert!(std::ptr::eq(first, last));
    assert_eq!(*first, format());
    assert_eq!(m.pieces.capacity(), m.pieces.len());
}

#[test]
fn tabs_jump_to_stops_and_right_stops_pull_back() {
    let mut r = run("ab");
    r.marks = vec![(1, ooxml_text::Mark::Tab)];
    let mut g = geometry(10);
    let m = measure(&Mono, &paragraph(vec![r.clone()]), &g, false).unwrap();
    assert_eq!(m.pieces[1].kind, PieceKind::Tab);
    assert_eq!(m.lines.len(), 1);
    g.tabs = vec![(Units::int(19_000), TabAlign::Right)];
    let m = measure(&Mono, &paragraph(vec![r]), &g, false).unwrap();
    assert_eq!(spans(&m.lines), [(0, 3)]);
}

#[test]
fn automatic_hyphens_break_long_words() {
    let mut p = paragraph(vec![run("extraordinary hyphenation")]);
    p.hyphenation = Some(Hyphenation {
        mode15: true,
        no_caps: false,
    });
    let mut g = geometry(18);
    let rules = WordRules { compatibility_mode: 15 };
    let fields = rules.hyphenation(None, 0, true);
    g.hyphen_gap = fields.gap;
    let m = measure(&Mono, &p, &g, false).unwrap();
    assert!(m.pieces.iter().any(|piece| piece.auto));
    assert!(m.lines[0].hyphenated, "{:?}", m.lines);
}

struct Budget(AtomicUsize);

impl CacheBudget for Budget {
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

#[test]
fn measures_are_memoised_by_content() {
    let budget = Arc::new(Budget(AtomicUsize::new(0)));
    let mut measurer = Measurer::new(budget.clone(), DigestKey::new(1, 2));
    let locale = Locale::default();
    let p = paragraph(vec![run("aaa bbb ccc")]);
    let first = measurer.measure(&Mono, &p, &geometry(7), false, &locale).unwrap();
    let again = measurer
        .measure(&Mono, &p.clone(), &geometry(7), false, &locale)
        .unwrap();
    assert!(Arc::ptr_eq(&first, &again));
    let narrower = measurer.measure(&Mono, &p, &geometry(5), false, &locale).unwrap();
    assert_ne!(first.lines, narrower.lines);
    let stats = measurer.stats();
    assert_eq!((stats.hits, stats.misses, stats.entries), (1, 2, 2));
    assert!(budget.used() > 0);
    drop(measurer);
    assert_eq!(budget.used(), 0);
}
