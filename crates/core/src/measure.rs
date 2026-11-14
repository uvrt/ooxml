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

use crate::model::Export;
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
    pub fn units(&self, adv: i64, upm: i64, hp: i64) -> i64 {
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

/// Line heights over the document's tables (`lines.line_height`, `vertical.line_extent`).
pub struct Heights<'a> {
    pub t: &'a Tables,
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
    pub fn line_height(&self, text: &[usize], label: &[usize], rule: u8, line: i64) -> Option<LineH> {
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

pub fn height_key(f: &Fmt) -> (i64, i64, i64, Option<(i64, i64)>) {
    (f.font, f.hp, f.position, f.border)
}

pub fn floor_q(x: Q) -> i128 {
    floor(x)
}
