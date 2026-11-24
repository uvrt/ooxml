//! Owns: the glyph check docx2svg's tests scored the layout with (its `tools/glyphs.py`,
//! `read_render.py` and `read_wrap_side_probe.py` at 6e850e18), against `LaidPages`' paint:
//! every glyph Word drew (a recording's text objects, [`word_glyphs`]) matched to the
//! layout's ([`laid_glyphs`]) page by page in line order, and scored per glyph ([`compare`])
//! and per line of Word's ([`line_score`]).
//!
//! The tolerances are the deleted tests', unchanged. Word's pen positions come in two
//! precisions: the first glyph of each text object is printed to seven significant digits
//! and is held to half of that ([`exact_tolerance`]); inside an object Quartz encodes each
//! glyph's advance, so there the check is on the step from the glyph before, within half
//! of 1/1000 em per advance ([`STEP_EM`]). Baselines are whole device pixels and must be
//! equal; so must the face and the drawn size.
//!
//! Must not: lay out, or loosen a tolerance for a case: a case Word disagrees with is a
//! family's known difference, named there.

use ooxml_base::difflib::{opcodes, Tag};
use ooxml_word_layout::paint::{PagePaint, PaintedLine};
use serde_json::Value;
use std::collections::BTreeMap;

/// How far Quartz's encoding of one advance may be from Word's, in ems of the size it draws
/// at: half of the `TJ` grid (1/1000 em) and half of the precision `Tc` is printed to.
pub const STEP_EM: f64 = 0.0005 + 0.00005;

/// How far a text object's first glyph may be from the layout's in a line's score, device px.
pub const HALF_PIXEL: f64 = 0.5;

/// What Quartz's printing of a text object's pen x leaves uncertain: a single-precision
/// float in points (two units in its last place), scaled to device px, printed to seven
/// significant digits (half the last one).
pub fn exact_tolerance(x: f64) -> f64 {
    let quantum = 10f64.powf((x.abs().max(1.0)).log10().floor() - 6.0);
    let points = (x.abs() * 72.0 / 300.0).max(1.0);
    let ulp = 2f64.powf(points.log2().floor() - 23.0) * 300.0 / 72.0;
    quantum / 2.0 + 2.0 * ulp + 1e-6
}

/// One glyph drawn, Word's or the layout's.
#[derive(Debug, Clone)]
pub struct Glyph {
    /// 0-based page.
    pub page: usize,
    /// The character.
    pub ch: char,
    /// The pen x, device px.
    pub x: f64,
    /// The baseline it is drawn on, device px.
    pub y: f64,
    /// The face, as Word's `/BaseFont` names it or the document does.
    pub face: String,
    /// The drawn size, device px.
    pub size_px: f64,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// The line's baseline (for ordering; a script is drawn off it).
    pub line_y: f64,
    /// Word's side: the first glyph of a text object, whose x is exact.
    pub exact: bool,
    /// Word's side: which text object, and the glyph's index in it (spaces counted).
    pub run: usize,
    /// See `run`.
    pub index: usize,
}

/// `TimesNewRomanPS-BoldMT` -> `("TimesNewRoman", true, false)` (docx2svg's
/// `baselines._drawn_face`).
pub fn drawn_face(base: &str) -> (String, bool, bool) {
    let (family, style) = base.split_once('-').unwrap_or((base, ""));
    let mut family = family.to_owned();
    for suffix in ["PSMT", "MT", "PS"] {
        if family.ends_with(suffix) && family != suffix {
            family.truncate(family.len() - suffix.len());
            break;
        }
    }
    let style = style.strip_suffix("MT").unwrap_or(style);
    let bold = style.contains("Bold");
    let italic = style.contains("Italic") || style.contains("Oblique");
    let rest = style
        .replace("Bold", "")
        .replace("Italic", "")
        .replace("Oblique", "")
        .replace("Regular", "");
    (family + &rest, bold, italic)
}

/// Every non-space glyph of a recording's text objects: `[page, baseline, /BaseFont, drawn
/// size, text, x0 (1e-4 px), steps (1e-3 px)]` (docx2svg's `read_render.glyphs_of`).
pub fn word_glyphs(objects: &Value) -> Vec<Glyph> {
    let mut out = Vec::new();
    for (number, object) in objects.as_array().into_iter().flatten().enumerate() {
        let num = |k: usize| {
            object[k]
                .as_f64()
                .unwrap_or_else(|| panic!("text object field {k}: {object}"))
        };
        let (page, y, size) = (num(0) as usize, num(1), num(3));
        let (face, bold, italic) = drawn_face(object[2].as_str().unwrap_or_default());
        let mut x = num(5) / 10000.0;
        let mut xs = vec![x];
        for step in object[6].as_array().into_iter().flatten() {
            x += step.as_f64().unwrap_or_default() / 1000.0;
            xs.push(x);
        }
        let text = object[4].as_str().unwrap_or_default();
        for (k, (ch, gx)) in text.chars().zip(xs).enumerate() {
            if ch.is_whitespace() {
                continue;
            }
            out.push(Glyph {
                page,
                ch,
                x: gx,
                y,
                face: face.clone(),
                size_px: size,
                bold,
                italic,
                line_y: y,
                exact: k == 0,
                run: number + 1,
                index: k,
            });
        }
    }
    out
}

/// The size a glyph of `half_points` is drawn at, whole device px (docx2svg's
/// `svg.font_size_px(.., "device")`: 300 dpi, rounded half up).
fn device_size(half_points: i64) -> f64 {
    // half_points / 2 * 300 / 72 = half_points * 25 / 12.
    (half_points * 25 * 2 + 12).div_euclid(24) as f64
}

fn px(value: ooxml_word_layout::Px) -> f64 {
    let q = value.value();
    *q.numer() as f64 / *q.denom() as f64
}

fn line_glyphs(page: usize, line: &PaintedLine, out: &mut Vec<Glyph>) {
    for span in &line.spans {
        for (ch, x) in span.chars.iter().zip(span.pens()) {
            if ch.is_whitespace() {
                continue;
            }
            out.push(Glyph {
                page,
                ch: *ch,
                x: px(x),
                y: span.y as f64,
                face: span.face.to_string(),
                size_px: device_size(span.half_points),
                bold: span.bold,
                italic: span.italic,
                line_y: line.baseline as f64,
                exact: true,
                run: 0,
                index: 0,
            });
        }
    }
}

/// Every non-space glyph the pages paint, their text boxes' included (docx2svg's
/// `glyphs.layout_glyphs` over `Page.text_lines`).
pub fn laid_glyphs(pages: &[std::sync::Arc<PagePaint>]) -> Vec<Glyph> {
    let mut out = Vec::new();
    for (page, paint) in pages.iter().enumerate() {
        for line in &paint.lines {
            line_glyphs(page, line, &mut out);
        }
        for text_box in &paint.text_boxes {
            for line in &text_box.lines {
                line_glyphs(page, line, &mut out);
            }
        }
    }
    out
}

/// What two glyphs must share to be matched: the character, a symbol face's private-use
/// code and the Unicode Word's `ToUnicode` gives it being the same glyph.
fn key(glyph: &Glyph) -> char {
    let face = norm_face(&glyph.face);
    if ('\u{f000}'..='\u{f0ff}').contains(&glyph.ch) || face == "symbol" || face == "wingdings" {
        '\u{f8ff}'
    } else {
        glyph.ch
    }
}

fn norm_face(face: &str) -> String {
    face.replace(' ', "").to_lowercase()
}

/// `round` as Python's: half to even.
fn round(v: f64) -> i64 {
    v.round_ties_even() as i64
}

/// One page's glyphs of both sides in line order, matched: `(layout's, Word's, pairs)`,
/// the pairs in order as indices into the two.
fn matched<'a>(
    model: &'a [Glyph],
    word: &'a [Glyph],
    page: usize,
) -> (Vec<&'a Glyph>, Vec<&'a Glyph>, Vec<(usize, usize)>) {
    let order = |g: &&Glyph| (round(g.y), ordered_float(g.x));
    let mut ours: Vec<&Glyph> = model.iter().filter(|g| g.page == page).collect();
    let mut theirs: Vec<&Glyph> = word.iter().filter(|g| g.page == page).collect();
    ours.sort_by_key(order);
    theirs.sort_by_key(order);
    let a: Vec<char> = ours.iter().map(|g| key(g)).collect();
    let b: Vec<char> = theirs.iter().map(|g| key(g)).collect();
    let mut pairs = Vec::new();
    for op in opcodes(&a, &b) {
        if op.tag == Tag::Equal {
            pairs.extend((0..op.i2 - op.i1).map(|k| (op.i1 + k, op.j1 + k)));
        }
    }
    (ours, theirs, pairs)
}

/// A total order on the x's (none is NaN).
fn ordered_float(x: f64) -> i64 {
    (x * 1e6).round() as i64
}

/// The glyph row docx2svg's recordings pin (`read_render.row`), and its problems.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Score {
    /// Word's glyphs.
    pub word: usize,
    /// The layout's.
    pub model: usize,
    /// Matched.
    pub matched: usize,
    /// Matched with x agreeing (an object's start exactly, an advance by its step).
    pub x_agree: usize,
    /// Object starts scored and agreeing.
    pub exact_scored: usize,
    /// See `exact_scored`.
    pub exact_agree: usize,
    /// Steps scored and agreeing.
    pub step_scored: usize,
    /// See `step_scored`.
    pub step_agree: usize,
    /// Matched on the same baseline.
    pub y_agree: usize,
    /// Matched in the same face, bold and italic.
    pub face_agree: usize,
    /// Matched at the same drawn size.
    pub size_agree: usize,
    /// Word's glyphs the layout does not draw.
    pub not_drawn: usize,
    /// The layout's glyphs Word does not draw.
    pub extra: usize,
    /// Each disagreeing glyph, described.
    pub problems: Vec<String>,
}

/// Match `model` against `word` page by page and score every matched glyph
/// (docx2svg's `glyphs.compare`).
pub fn compare(model: &[Glyph], word: &[Glyph]) -> Score {
    let mut score = Score {
        word: word.len(),
        model: model.len(),
        ..Score::default()
    };
    let pages: std::collections::BTreeSet<usize> = model.iter().chain(word).map(|g| g.page).collect();
    for page in pages {
        let (ours, theirs, pairs) = matched(model, word, page);
        let mut previous: Option<(&Glyph, &Glyph)> = None;
        for &(i, j) in &pairs {
            let (a, b) = (ours[i], theirs[j]);
            score.matched += 1;
            let x_ok = if b.exact {
                let ok = (a.x - b.x).abs() <= exact_tolerance(b.x);
                score.exact_scored += 1;
                score.exact_agree += usize::from(ok);
                ok
            } else if let Some((pa, pb)) = previous.filter(|(_, pb)| pb.run == b.run) {
                let advances = (b.index - pb.index) as f64;
                let error = ((a.x - pa.x) - (b.x - pb.x)).abs();
                let ok = error <= advances * STEP_EM * b.size_px + 2e-3;
                score.step_scored += 1;
                score.step_agree += usize::from(ok);
                ok
            } else {
                true
            };
            score.x_agree += usize::from(x_ok);
            previous = Some((a, b));
            let y_ok = round(a.y) == round(b.y);
            let face_ok = (norm_face(&a.face), a.bold, a.italic) == (norm_face(&b.face), b.bold, b.italic);
            let size_ok = round(a.size_px) == round(b.size_px);
            score.y_agree += usize::from(y_ok);
            score.face_agree += usize::from(face_ok);
            score.size_agree += usize::from(size_ok);
            if !(x_ok && y_ok && face_ok && size_ok) {
                score.problems.push(format!(
                    "p{} {:?}: layout x={:.4} y={} {} {}{}{}; Word x={:.4} y={} {} {}{}{}{} dx={:+.4}",
                    page + 1,
                    a.ch,
                    a.x,
                    a.y,
                    a.face,
                    a.size_px,
                    if a.bold { " b" } else { "" },
                    if a.italic { " i" } else { "" },
                    b.x,
                    b.y,
                    b.face,
                    b.size_px,
                    if b.bold { " b" } else { "" },
                    if b.italic { " i" } else { "" },
                    if b.exact { "" } else { " (in TJ)" },
                    a.x - b.x
                ));
            }
        }
        score.extra += ours.len() - pairs.len();
        score.not_drawn += theirs.len() - pairs.len();
    }
    score
}

/// A line of Word's: its page (0-based) and baseline, device px.
pub type LineKey = (usize, i64);

/// Every line of Word's and its problems, empty where it agrees (docx2svg's
/// `read_wrap_side_probe.line_score`): every glyph on it matched to the layout's on the same
/// rounded baseline, each text object's first glyph within [`HALF_PIXEL`], and every glyph
/// inside an object stepping from the one before as the layout's does ([`STEP_EM`]).
pub fn line_score(model: &[Glyph], word: &[Glyph]) -> BTreeMap<LineKey, Vec<String>> {
    let mut out: BTreeMap<LineKey, Vec<String>> = BTreeMap::new();
    let pages: std::collections::BTreeSet<usize> = word.iter().map(|g| g.page).collect();
    let line = |g: &Glyph| (g.page, round(if g.line_y != 0.0 { g.line_y } else { g.y }));
    for page in pages {
        let (ours, theirs, pairs) = matched(model, word, page);
        for g in &theirs {
            out.entry(line(g)).or_default();
        }
        let mut seen = vec![false; theirs.len()];
        // The glyph before restarts at each matching block (docx2svg's loop does).
        let mut previous: Option<(usize, usize)> = None;
        for &(i, j) in &pairs {
            let (a, b) = (ours[i], theirs[j]);
            seen[j] = true;
            let contiguous = previous.filter(|&(pi, pj)| pi + 1 == i && pj + 1 == j);
            let ok = if b.exact {
                (a.x - b.x).abs() <= HALF_PIXEL
            } else if let Some((pi, pj)) = contiguous.filter(|&(_, pj)| theirs[pj].run == b.run) {
                let (pa, pb) = (ours[pi], theirs[pj]);
                let advances = (b.index - pb.index) as f64;
                let error = ((a.x - pa.x) - (b.x - pb.x)).abs();
                error <= advances * STEP_EM * b.size_px + 2e-3
            } else {
                true
            };
            previous = Some((i, j));
            if !ok || round(a.y) != round(b.y) {
                out.entry(line(b)).or_default().push(format!(
                    "{:?} Word x={:.2} y={}, layout x={:.2} y={}",
                    b.ch, b.x, b.y, a.x, a.y
                ));
            }
        }
        for (j, g) in theirs.iter().enumerate() {
            if !seen[j] {
                out.entry(line(g))
                    .or_default()
                    .push(format!("{:?} at x={:.2} not drawn by the layout", g.ch, g.x));
            }
        }
    }
    out
}
