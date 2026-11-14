//! The measure kernel: a paragraph's text and formats -> pieces (widths from docx2svg's own
//! advance and kern tables) -> lines (docx2svg's `linebreak.break_pieces`, greedy first fit,
//! exact) -> line heights (`lines.line_height`, `vertical.line_extent`, ...).
//!
//! Ported: `linebreak.pieces` (runs, tabs, breaks, soft and no-break hyphens, caps, `w:w`,
//! tracking, kerning, list labels with their alignment and suffix), `break_pieces` /
//! `break_line` (tab stops of every alignment, a stop past the edge, the justified squeeze
//! of mode 15, soft hyphens, emergency breaks), `lines.shares_of`, `lines.line_height` with
//! `vertical.line_extent`, `item_extent`, `tallest_natural`, `mixed_line_pitch`.
//! Not ported (the exporter refuses a document using them): automatic hyphenation, inline
//! pictures, picture bullets, note numbers, run borders, drawings beside text.

use crate::model::{Export, GeometryIn, MeasureIn};
use crate::units::{exact_div, floor, q, Px, Q};
use num_integer::Integer;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// A small multiplicative hasher (the standard SipHash is slow for these keys).
#[derive(Default)]
pub struct Fx(u64);
impl Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(b as u64)
        }
    }
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn write_u32(&mut self, i: u32) {
        self.write_u64(i as u64)
    }
    fn write_i64(&mut self, i: i64) {
        self.write_u64(i as u64)
    }
    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64)
    }
}
pub type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<Fx>>;

pub const GLYPH: u8 = 0;
pub const SPACE: u8 = 1;
pub const TAB: u8 = 2;
pub const BREAK: u8 = 3;
pub const SOFT: u8 = 4;

const DENSE: usize = 0x3000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FaceM {
    pub upm: i64,
    pub ascent: i64,
    pub descent: i64,
    pub gap: i64,
    pub script: [Option<i64>; 4],
}

pub struct FontT {
    pub upm: i64,
    pub metrics: Option<FaceM>,
    adv: Vec<i32>,
    adv_more: FxMap<u32, i64>,
    kern: FxMap<(u32, u32), i64>,
    kern_known: bool,
}

impl FontT {
    #[inline]
    pub fn advance(&self, c: char) -> i64 {
        let u = c as usize;
        let v = if u < DENSE { self.adv[u] as i64 } else { *self.adv_more.get(&(u as u32)).unwrap_or(&-1) };
        assert!(v >= 0, "no advance for {c:?}");
        v
    }
    #[inline]
    pub fn kern(&self, l: char, r: char) -> i64 {
        assert!(self.kern_known, "kerning not exported for this face");
        *self.kern.get(&(l as u32, r as u32)).unwrap_or(&0)
    }
}

#[derive(Clone, Copy)]
pub struct Fmt {
    pub font: i64,
    pub hp: i64,
    pub whp: i64,
    pub position: i64,
    pub border: Option<(i64, i64)>,
}

/// Everything measuring reads besides the paragraph: docx2svg's tables, as exported.
pub struct Tables {
    /// Widths are in 1/`s` layout units.
    pub s: i64,
    pub w_den: i64,
    pub px: Px,
    pub fonts: Vec<FontT>,
    pub fmts: Vec<Fmt>,
}

impl Tables {
    pub fn new(e: &Export) -> Tables {
        Tables::from_parts(&e.fonts, &e.formats, e.w_den, e.px_den, e.lu, e.dpi)
    }

    /// Lane E: the tables from their parts (a commit may bring new faces, formats or
    /// characters: the tables are made again from the whole lists).
    pub fn from_parts(e_fonts: &[crate::model::FontIn], formats: &[(i64, i64, i64, i64, Option<(i64, i64)>)], w_den: i64, px_den: i64, lu: i64, dpi: i64) -> Tables {
        let mut s = w_den.lcm(&200);
        let mut fonts = Vec::new();
        for f in e_fonts {
            if f.upm > 0 {
                s = s.lcm(&(f.upm / f.upm.gcd(&2048)));
            }
            let mut adv = vec![-1i32; DENSE];
            let mut adv_more = FxMap::default();
            for (k, v) in &f.adv {
                let c = k.chars().next().unwrap() as usize;
                if c < DENSE {
                    adv[c] = *v as i32;
                } else {
                    adv_more.insert(c as u32, *v);
                }
            }
            let mut kern = FxMap::default();
            for (l, r, v) in &f.kern {
                kern.insert((l.chars().next().unwrap() as u32, r.chars().next().unwrap() as u32), *v);
            }
            let metrics = f.metrics.as_ref().map(|m| FaceM {
                upm: m[0].unwrap(),
                ascent: m[1].unwrap(),
                descent: m[2].unwrap(),
                gap: m[3].unwrap(),
                script: [m[4], m[5], m[6], m[7]],
            });
            let _ = (&f.face, f.bold, f.italic);
            fonts.push(FontT { upm: f.upm, metrics, adv, adv_more, kern, kern_known: f.kern_known });
        }
        let fmts = formats
            .iter()
            .map(|&(font, hp, whp, position, border)| Fmt { font, hp, whp, position, border })
            .collect();
        Tables { s, w_den, px: Px { den: px_den, lu, dpi }, fonts, fmts }
    }

    /// `linebreak._units`: an advance at a size, in 1/`s` units, exact.
    #[inline]
    fn units(&self, adv: i64, upm: i64, hp: i64) -> i64 {
        exact_div(adv as i128 * hp as i128 * 2048 * self.s as i128, upm as i128)
    }
    /// An exported width (over `w_den`) in 1/`s` units.
    pub fn w(&self, v: i64) -> i64 {
        v * (self.s / self.w_den)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Piece {
    pub kind: u8,
    pub ch: char,
    pub width: i64,
    /// The width of the kern pair with the next piece (`linebreak._kern`).
    pub kern_next: i64,
    /// `(font, size)` pieces kern within; -1: kerning off.
    pub kkey: i64,
    pub source: i32,
    pub hyphen: i64,
    pub run: i32,
    /// Lane E: the half points the character is drawn at (0: not a drawn character).
    pub hp: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    pub start: usize,
    pub end: usize,
    pub forced: bool,
    pub emergency: bool,
    pub hyphenated: bool,
}

impl Line {
    fn new(start: usize, end: usize) -> Line {
        Line { start, end, forced: false, emergency: false, hyphenated: false }
    }
    pub fn flags(&self) -> u8 {
        self.forced as u8 | (self.emergency as u8) << 1 | (self.hyphenated as u8) << 2
    }
}

/// `linebreak.Geometry`, in 1/`s` units.
pub struct Geometry {
    first_start: i64,
    start: i64,
    right: i64,
    tabs: Vec<(i64, u8)>,
    default_tab: i64,
    hanging_stop: Option<i64>,
    squeeze: bool,
    tab_origin: i64,
}

// Tab alignments, ordered as their names compare (Python's `min` over `(stop, alignment)`).
fn align_code(a: &str) -> u8 {
    match a {
        "center" => 1,
        "decimal" => 2,
        "end" => 3,
        "left" => 4,
        "num" => 5,
        "right" => 6,
        "start" => 7,
        _ => 0,
    }
}
const LEFT: u8 = 4;

impl Geometry {
    pub fn new(t: &Tables, g: &GeometryIn) -> Geometry {
        Geometry {
            first_start: t.w(g.first_start),
            start: t.w(g.start),
            right: t.w(g.right),
            tabs: g.tabs.iter().map(|(p, a)| (t.w(*p), align_code(a))).collect(),
            default_tab: t.w(g.default_tab),
            hanging_stop: g.hanging_stop.map(|v| t.w(v)),
            squeeze: g.squeeze,
            tab_origin: t.w(g.tab_origin),
        }
    }

    /// `linebreak.next_tab`: the stop, its alignment, whether it is a default stop.
    fn next_tab(&self, x: i64, first: bool) -> (i64, u8, bool) {
        let origin = self.tab_origin;
        let mut best: Option<(i64, u8)> = None;
        for &(p, a) in &self.tabs {
            if p + origin > x {
                let c = (p + origin, a);
                if best.map_or(true, |b| c < b) {
                    best = Some(c);
                }
            }
        }
        if first {
            if let Some(h) = self.hanging_stop {
                if h > x {
                    let c = (h, LEFT);
                    if best.map_or(true, |b| c < b) {
                        best = Some(c);
                    }
                }
            }
        }
        if let Some((s, a)) = best {
            return (s, a, false);
        }
        let last = self.tabs.iter().map(|t| t.0).max().unwrap_or(0) + origin;
        let base = x.max(last);
        let step = self.default_tab;
        (((base - origin).div_floor(&step) + 1) * step + origin, LEFT, true)
    }
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\u{2002}' | '\u{2003}' | '\u{2005}' | '\u{200b}')
}

/// `linebreak.pieces` (with `label_pieces` first).
pub fn pieces(t: &Tables, m: &MeasureIn) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::with_capacity(256);
    if let Some(label) = &m.label {
        let mut lab = Vec::new();
        for (ch, f) in &label.chars {
            let f = t.fmts[*f];
            let font = &t.fonts[f.font as usize];
            let c = ch.chars().next().unwrap();
            lab.push(Piece {
                kind: GLYPH,
                ch: c,
                width: t.units(font.advance(c), font.upm, f.hp),
                kern_next: 0,
                kkey: -1,
                source: -1,
                hyphen: 0,
                run: -1,
                hp: f.hp,
            });
        }
        let total: i64 = lab.iter().map(|p| p.width).sum();
        let shift = match label.align.as_str() {
            "center" => exact_div(total as i128, 2),
            "right" => total,
            _ => 0,
        };
        if shift != 0 {
            out.push(Piece { kind: GLYPH, ch: '\0', width: -shift, kern_next: 0, kkey: -1, source: -1, hyphen: 0, run: -1, hp: 0 });
        }
        out.extend(lab);
        match label.suffix.as_str() {
            "tab" => out.push(Piece { kind: TAB, ch: '\t', width: 0, kern_next: 0, kkey: -1, source: -1, hyphen: 0, run: -1, hp: 0 }),
            "space" => {
                let f = t.fmts[label.space_fmt];
                let font = &t.fonts[f.font as usize];
                out.push(Piece {
                    kind: GLYPH,
                    ch: ' ',
                    width: t.units(font.advance(' '), font.upm, f.hp),
                    kern_next: 0,
                    kkey: -1,
                    source: -1,
                    hyphen: 0,
                    run: -1,
                    hp: 0,
                });
            }
            _ => {}
        }
    }
    let mut source: i32 = 0;
    for (ri, run) in m.runs.iter().enumerate() {
        let first_new = out.len();
        let n = run.text.chars().count() as i32;
        if run.vanish {
            source += n;
            continue;
        }
        let glyph = |c: char, kind: u8, index: i32, f: usize| -> Piece {
            let f = t.fmts[f];
            let drawn = if run.caps { c.to_uppercase().next().unwrap() } else { c };
            let font = &t.fonts[f.font as usize];
            let mut width = t.units(font.advance(drawn), font.upm, f.whp);
            if run.scale != 100 && run.scale != 0 {
                width = exact_div(width as i128 * run.scale as i128, 100);
            }
            if run.tracking != 0 {
                width += run.tracking * t.s;
            }
            let kkey = if run.kern != 0 && f.hp >= run.kern { f.font * 100_000 + f.whp } else { -1 };
            Piece { kind, ch: drawn, width, kern_next: 0, kkey, source: index, hyphen: 0, run: ri as i32, hp: f.hp }
        };
        let mark = |kind: &str, out: &mut Vec<Piece>| match kind {
            "tab" => out.push(Piece { kind: TAB, ch: '\t', width: 0, kern_next: 0, kkey: -1, source, hyphen: 0, run: -1, hp: 0 }),
            "br" | "textWrapping" | "page" | "column" | "cr" => {
                let ch = match kind {
                    "page" => '\u{c}',
                    "column" => '\u{b}',
                    _ => '\n',
                };
                out.push(Piece { kind: BREAK, ch, width: 0, kern_next: 0, kkey: -1, source, hyphen: 0, run: -1, hp: 0 })
            }
            "softHyphen" => {
                let h = glyph('-', GLYPH, source, run.hyphen_fmt);
                out.push(Piece { kind: SOFT, ch: '\u{ad}', width: 0, kern_next: 0, kkey: -1, source, hyphen: h.width, run: -1, hp: 0 })
            }
            "noBreakHyphen" => {
                let mut p = glyph('-', GLYPH, source, run.hyphen_fmt);
                p.ch = '\u{2011}';
                out.push(p)
            }
            other => panic!("mark {other} not ported"),
        };
        let uniform = run.fmts.len() == 1;
        let mut mi = 0;
        let marks = &run.marks;
        for (offset, c) in run.text.chars().enumerate() {
            while mi < marks.len() && marks[mi].0 == offset {
                mark(&marks[mi].1, &mut out);
                mi += 1;
            }
            let f = if uniform { run.fmts[0] } else { run.fmts[offset] };
            out.push(glyph(c, if is_space(c) { SPACE } else { GLYPH }, source + offset as i32, f));
        }
        while mi < marks.len() {
            assert!(marks[mi].0 == n as usize, "marks out of order");
            mark(&marks[mi].1, &mut out);
            mi += 1;
        }
        for p in &mut out[first_new..] {
            p.run = ri as i32;
        }
        source += n;
    }
    // kern pairs (`linebreak._kern(piece, next)`)
    for j in 0..out.len().saturating_sub(1) {
        let (a, b) = (out[j], out[j + 1]);
        if b.kkey >= 0 && a.kkey == b.kkey {
            let f = (b.kkey / 100_000) as usize;
            let hp = b.kkey % 100_000;
            let font = &t.fonts[f];
            let v = font.kern(a.ch, b.ch);
            if v != 0 {
                out[j].kern_next = t.units(v, font.upm, hp);
            }
        }
    }
    out
}

fn may_break(before: &Piece, after: &Piece) -> bool {
    if after.kind == SPACE {
        return false;
    }
    if before.kind == SPACE || after.kind == TAB || before.kind == SOFT {
        return true;
    }
    before.kind == GLYPH && matches!(before.ch, '-' | '\u{2013}' | '\u{2014}') && before.source >= 0
}

/// `linebreak.squeeze`: a quarter of a space, half up to the unit.
fn squeeze(s: i64, p: &Piece) -> i64 {
    (p.width + 2 * s).div_floor(&(4 * s)) * s
}

fn squeezes(over: i64, room: i64, spaces: i64, slack: i64, spaces_before: i64) -> bool {
    if over > room {
        return false;
    }
    spaces_before == 0 || 2 * over as i128 * spaces_before as i128 <= slack as i128 * spaces as i128
}

const INF: i64 = i64::MAX / 4;

/// `linebreak.break_line`.
pub fn break_line(s: i64, items: &[Piece], i: usize, g: &Geometry, first: bool) -> Line {
    let n = items.len();
    let mut x = if first { g.first_start } else { g.start };
    let start = i;
    let mut last_break: Option<usize> = None;
    let mut text_end = x;
    let mut limit = g.right;
    let (mut room, mut spaces, mut before_spaces) = (0i64, 0i64, 0i64);
    let mut before_x = x;
    let mut j = i;
    while j < n {
        let piece = &items[j];
        if j > start && may_break(&items[j - 1], piece) {
            let before = &items[j - 1];
            if before.kind != SOFT || x + before.hyphen <= g.right {
                last_break = Some(j - 1);
            }
        }
        if piece.kind == BREAK {
            return Line { start, end: j + 1, forced: true, emergency: false, hyphenated: false };
        }
        let x_new;
        if piece.kind == TAB {
            let (stop, align, default) = g.next_tab(x, first);
            if matches!(align, 6 | 1 | 2) {
                x_new = x;
                limit = if stop <= g.right { g.right } else if align != 1 { stop } else { 2 * stop - x };
            } else if stop > g.right && !default {
                x_new = stop;
                limit = INF;
            } else {
                x_new = stop;
                limit = g.right;
            }
            room = 0;
            spaces = 0;
            before_spaces = 0;
            before_x = x_new;
        } else {
            x_new = x + piece.width + piece.kern_next;
            if piece.kind == SPACE && g.squeeze {
                if j == start || items[j - 1].kind != SPACE {
                    before_x = x;
                    before_spaces = spaces;
                }
                room += squeeze(s, piece);
                spaces += 1;
            }
        }
        if piece.kind != SPACE
            && piece.kind != SOFT
            && x_new > limit
            && j > start
            && !(g.squeeze && squeezes(x_new - limit, room, spaces, limit - before_x, before_spaces))
        {
            return match last_break {
                Some(at) => Line { start, end: at + 1, forced: false, emergency: false, hyphenated: items[at].kind == SOFT },
                None => Line { start, end: j, forced: false, emergency: true, hyphenated: false },
            };
        }
        if piece.kind == GLYPH || piece.kind == TAB {
            text_end = x_new;
        }
        x = x_new;
        j += 1;
    }
    let _ = text_end;
    Line::new(start, n)
}

/// `linebreak.break_pieces`.
pub fn break_pieces(s: i64, items: &[Piece], g: &Geometry, columns: bool) -> Vec<Line> {
    let n = items.len();
    let mut lines: Vec<Line> = Vec::new();
    let mut i = 0;
    while i < n || lines.is_empty() {
        let line = break_line(s, items, i, g, lines.is_empty());
        lines.push(line);
        i = line.end;
        if i >= n {
            if line.forced {
                let c = items[n - 1].ch;
                if c == '\n' || (columns && c == '\u{b}') {
                    lines.push(Line::new(n, n));
                }
            }
            break;
        }
    }
    lines
}

/// One line's vertical facts, px over `px.den` (`lines.LineHeight`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineH {
    pub pitch: i64,
    pub above: i64,
    pub below: i64,
    pub rule: u8,
    pub line: i64,
    pub natural: i64,
}

impl LineH {
    /// `paginate.multiple_extra` taken off: what of the line must fit above the margin.
    pub fn fit(&self) -> i64 {
        if self.rule == 0 && self.line > 240 {
            self.above + self.below
        } else {
            self.pitch
        }
    }
}

#[derive(Clone, Copy)]
struct Extent {
    above: Q,
    below: Q,
    hp: i64,
    face: Option<FaceM>,
    label: bool,
    moved: bool,
}

/// A face as `line_extent` returns it: a real one, or the synthetic face of a mixed line
/// (`unitsPerEm` 1, its extents fractions of an em).
#[derive(Clone, Copy)]
enum Face {
    Real(FaceM),
    Synthetic(Q, Q),
}

impl Face {
    fn extent_sum(&self) -> (Q, Q, Q, Q) {
        // (ascent, descent, gap, upm) as rationals
        match *self {
            Face::Real(f) => (q(f.ascent as i128, 1), q(f.descent as i128, 1), q(f.gap as i128, 1), q(f.upm as i128, 1)),
            Face::Synthetic(a, d) => (a, d, Q::from_integer(0), Q::from_integer(1)),
        }
    }
}

struct Heights<'a> {
    t: &'a Tables,
}

impl<'a> Heights<'a> {
    fn em(&self, upm: Q, hp: i64) -> Q {
        q(hp as i128, 2) * self.t.px.px_per_pt() / upm
    }
    fn natural(&self, face: &Face, hp: i64) -> Q {
        let (a, d, g, upm) = face.extent_sum();
        self.t.px.quantise((a + d + g) * self.em(upm, hp))
    }
    fn face_extent(&self, f: &FaceM, hp: i64) -> (Q, Q) {
        let em = self.em(q(f.upm as i128, 1), hp);
        (q((f.ascent + f.gap) as i128, 1) * em, q(f.descent as i128, 1) * em)
    }
    fn item_extent(&self, fmt: &Fmt, f: &FaceM) -> (Q, Q) {
        let (mut above, mut below) = self.face_extent(f, fmt.hp);
        let shift = q(self.t.px.twips(fmt.position * 10) as i128, self.t.px.den as i128);
        above += shift;
        below -= shift;
        assert!(fmt.border.is_none(), "run borders not ported");
        (above, below)
    }
    fn metrics(&self, fmt: &Fmt) -> Option<FaceM> {
        if fmt.font < 0 {
            None
        } else {
            self.t.fonts[fmt.font as usize].metrics
        }
    }
    /// `vertical.line_extent`.
    fn line_extent(&self, text: &[usize], label: &[usize]) -> Option<(Face, i64, bool)> {
        let mut ext: Vec<Extent> = Vec::new();
        for (k, is_label) in text.iter().map(|k| (k, false)).chain(label.iter().map(|k| (k, true))) {
            let fmt = &self.t.fmts[*k];
            let Some(face) = self.metrics(fmt) else { continue };
            let (above, below) = self.item_extent(fmt, &face);
            let moved = (above, below) != self.face_extent(&face, fmt.hp);
            ext.push(Extent { above, below: if is_label { Q::from_integer(0) } else { below }, hp: fmt.hp, face: Some(face), label: is_label, moved });
        }
        if ext.is_empty() {
            return None;
        }
        let mut top = &ext[0];
        for e in &ext {
            if e.above > top.above {
                top = e;
            }
        }
        let above = top.above;
        let below = ext.iter().map(|e| e.below).max().unwrap();
        let hp = top.hp;
        let first = (ext[0].face, ext[0].hp);
        let distinct_one = ext.iter().all(|e| (e.face, e.hp) == first);
        if distinct_one && !ext.iter().all(|e| e.label) && !ext.iter().any(|e| e.moved) {
            return Some((Face::Real(first.0.unwrap()), first.1, false));
        }
        let em = q(hp as i128, 2) * self.t.px.px_per_pt();
        Some((Face::Synthetic(above / em, below / em), hp, true))
    }
    fn tallest_natural(&self, items: &[usize]) -> Option<Q> {
        items
            .iter()
            .filter_map(|k| {
                let fmt = &self.t.fmts[*k];
                self.metrics(fmt).map(|f| self.natural(&Face::Real(f), fmt.hp))
            })
            .max()
    }
    fn pitch_px(&self, face: &Face, hp: i64, rule: u8, line: i64) -> Q {
        let natural = self.natural(face, hp);
        match rule {
            0 => self.t.px.quantise(natural * q(line as i128, 240)),
            1 => q(self.t.px.twips(line) as i128, self.t.px.den as i128),
            _ => natural.max(q(self.t.px.twips(line) as i128, self.t.px.den as i128)),
        }
    }
    /// `lines._line_height`.
    fn line_height(&self, text: &[usize], label: &[usize], rule: u8, line: i64) -> Option<LineH> {
        let combined = self.line_extent(text, label)?;
        let text_only = if label.is_empty() { None } else { self.line_extent(text, &[]) };
        let text_natural = self.tallest_natural(text);
        let (face, hp, _) = combined;
        let pitch = if rule != 0 || line == 240 || (text_only.is_none() && text_natural.is_none()) {
            self.pitch_px(&face, hp, rule, line)
        } else {
            let tn = match text_natural {
                Some(v) => v,
                None => {
                    let (f, h, _) = text_only.unwrap();
                    self.natural(&f, h)
                }
            };
            let everything = self.natural(&face, hp);
            self.t.px.quantise(everything + tn * (q(line as i128, 240) - Q::from_integer(1)))
        };
        let (a, d, g, upm) = face.extent_sum();
        let em = self.em(upm, hp);
        let px = &self.t.px;
        Some(LineH {
            pitch: px.of(pitch),
            above: px.of((a + g) * em),
            below: px.of(d * em),
            rule,
            line,
            natural: px.of(self.natural(&face, hp)),
        })
    }
}

pub struct Measured {
    pub pieces: Vec<Piece>,
    pub lines: Vec<Line>,
    pub heights: Vec<LineH>,
}

/// `lines.shares_of` -> per line the formats that set its height, then `line_height`.
pub fn measure(t: &Tables, m: &MeasureIn) -> Measured {
    let items = pieces(t, m);
    let g = Geometry::new(t, &m.geometry);
    let lines = break_pieces(t.s, &items, &g, m.columns);
    // which line each character is on
    let total: usize = m.runs.iter().map(|r| r.text.chars().count()).sum();
    let mut line_of: Vec<i32> = vec![-1; total + 1]; // a mark at the end carries the index after the last character
    for (number, line) in lines.iter().enumerate() {
        for p in &items[line.start..line.end] {
            if p.source >= 0 && p.kind != TAB && p.kind != BREAK {
                let slot = &mut line_of[p.source as usize];
                if *slot < 0 {
                    *slot = number as i32;
                }
            }
        }
    }
    // per line: formats of its non-space characters (deduplicated by what sets a height), or none
    let mut per_line: Vec<Vec<usize>> = vec![Vec::new(); lines.len()];
    let mut any_text: Vec<bool> = vec![false; lines.len()];
    let mut current = 0usize;
    let mut index = 0usize;
    for run in &m.runs {
        let n = run.text.chars().count();
        if !run.vanish {
            let uniform = run.fmts.len() == 1;
            for (offset, c) in run.text.chars().enumerate() {
                if line_of[index + offset] >= 0 {
                    current = line_of[index + offset] as usize;
                }
                if c != ' ' {
                    any_text[current] = true;
                    let f = if uniform { run.fmts[0] } else { run.fmts[offset] };
                    let key = height_key(&t.fmts[f]);
                    if !per_line[current].iter().any(|k| height_key(&t.fmts[*k]) == key) {
                        per_line[current].push(f);
                    }
                }
            }
        }
        index += n;
    }
    let h = Heights { t };
    let mut label: Vec<usize> = Vec::new();
    for f in &m.label_height_fmts {
        if !label.iter().any(|k| height_key(&t.fmts[*k]) == height_key(&t.fmts[*f])) {
            label.push(*f);
        }
    }
    let mut cache: Vec<(Vec<usize>, bool, LineH)> = Vec::new();
    let mut heights = Vec::with_capacity(lines.len());
    for number in 0..lines.len() {
        let text: Vec<usize> = if any_text[number] { per_line[number].clone() } else { vec![m.mark_fmt] };
        let first = number == 0;
        if let Some((_, _, v)) = cache.iter().find(|(k, f, _)| k == &text && (*f == first || label.is_empty())) {
            heights.push(*v);
            continue;
        }
        let v = h.line_height(&text, if first { &label } else { &[] }, m.rule, m.line).expect("no face metrics");
        cache.push((text, first, v));
        heights.push(v);
    }
    Measured { pieces: items, lines, heights }
}

fn height_key(f: &Fmt) -> (i64, i64, i64, Option<(i64, i64)>) {
    (f.font, f.hp, f.position, f.border)
}

pub fn floor_q(x: Q) -> i128 {
    floor(x)
}
