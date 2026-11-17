//! Owns: a text's ink in one face -- where each character is set (its advances, kerned as
//! Word kerns a watermark's letters: `GPOS`'s `kern` pairs, else the legacy `kern` table's)
//! and the box round the outlines set there, optionally slanted ([`Face::ink`],
//! [`TextInk`]): what a VML text path that fits its shape is stretched by. docx2svg's
//! `Face.pair_kern`, `Face.pens`, `Face.glyph_box` and `Face.ink`.
//!
//! Must not: shape text (no ligatures, no other feature), or choose the face.

use crate::be::u16_at;
use crate::outlines::{Cff, Glyf, GlyphInk, InkBox};
use crate::sfnt::Face;

/// A text's outlines set in one face: what a text path that fits its shape is stretched by.
#[derive(Debug, Clone, PartialEq)]
pub struct TextInk {
    /// The face's units per em.
    pub units_per_em: u16,
    /// The box round the outlines, font units, y up from the baseline.
    pub ink: InkBox,
    /// Where each character is set from pen 0, font units.
    pub pens: Vec<i64>,
    /// The slant the outlines were followed at (`x + slant * y`), 0 for upright.
    pub slant: f64,
}

impl Face {
    /// The kerning between two characters a watermark's letters are set with, font units:
    /// the `GPOS` `kern` feature's pair adjustment where the face has one, else its legacy
    /// `kern` table's.
    pub fn pair_kern(&self, left: char, right: char) -> i64 {
        let Some(gpos) = self.gpos_kerning() else {
            return i64::from(self.kern(left, right));
        };
        match (self.glyph(left), self.glyph(right)) {
            (Some(a), Some(b)) => match (u16::try_from(a), u16::try_from(b)) {
                (Ok(a), Ok(b)) => gpos.pair(a, b),
                _ => 0,
            },
            _ => 0,
        }
    }

    /// Where each character of `text` is set from pen 0, font units: at its advances, with
    /// [`Face::pair_kern`] between neighbours when `kerned`; `None` for a character the face
    /// lacks.
    pub fn pens(&self, text: &str, kerned: bool) -> Option<Vec<i64>> {
        let chars: Vec<char> = text.chars().collect();
        let mut pens = Vec::with_capacity(chars.len());
        let mut pen = 0_i64;
        for (i, &ch) in chars.iter().enumerate() {
            let advance = self.advance(ch)?;
            self.glyph(ch)?;
            pens.push(pen);
            let kern = match chars.get(i + 1) {
                Some(&next) if kerned => self.pair_kern(ch, next),
                _ => 0,
            };
            pen += i64::from(advance) + kern;
        }
        Some(pens)
    }

    /// The box round glyph `glyph`'s outline, font units, y up, slanted by `slant`: an
    /// upright `glyf` glyph's is its header's; `None` for an outline not read (or a face
    /// with neither `glyf` nor `CFF `).
    pub fn glyph_ink(&self, glyph: u32, slant: f64) -> Option<GlyphInk> {
        if let (Some(head), Some(loca), Some(glyf)) = (self.table(b"head"), self.table(b"loca"), self.table(b"glyf")) {
            let outlines = Glyf {
                glyf,
                loca,
                long_offsets: u16_at(head, 50)? == 1,
            };
            return if slant == 0.0 {
                outlines.header_box(glyph)
            } else {
                outlines.outline_box(glyph, slant)
            };
        }
        Cff::new(self.table(b"CFF ")?)?.outline_box(glyph, slant)
    }

    /// `text`'s outlines set at [`Face::pens`] (kerned), each glyph's box slanted by
    /// `slant`; `None` for a character the face lacks, an outline not read, or no outline
    /// at all.
    pub fn ink(&self, text: &str, slant: f64) -> Option<TextInk> {
        let pens = self.pens(text, true)?;
        let mut found: Option<InkBox> = None;
        for (ch, pen) in text.chars().zip(&pens) {
            let glyph = self.glyph(ch)?;
            let GlyphInk::Box(b) = self.glyph_ink(glyph, slant)? else {
                continue;
            };
            let moved = match b {
                InkBox::Units([x0, y0, x1, y1]) => InkBox::Units([pen + x0, y0, pen + x1, y1]),
                #[allow(clippy::cast_precision_loss)]
                InkBox::Float([x0, y0, x1, y1]) => InkBox::Float([*pen as f64 + x0, y0, *pen as f64 + x1, y1]),
            };
            found = Some(match (found, moved) {
                (None, m) => m,
                (Some(InkBox::Units(a)), InkBox::Units(m)) => {
                    InkBox::Units([a[0].min(m[0]), a[1].min(m[1]), a[2].max(m[2]), a[3].max(m[3])])
                }
                (Some(a), m) => {
                    let (a, m) = (floats(a), floats(m));
                    let lo = |x: f64, y: f64| if y < x { y } else { x };
                    let hi = |x: f64, y: f64| if y > x { y } else { x };
                    InkBox::Float([lo(a[0], m[0]), lo(a[1], m[1]), hi(a[2], m[2]), hi(a[3], m[3])])
                }
            });
        }
        let ink = found?;
        let f = floats(ink);
        (f[2] > f[0] && f[3] > f[1]).then(|| TextInk {
            units_per_em: self.units_per_em(),
            ink,
            pens,
            slant,
        })
    }
}

/// A box's values as floats.
#[allow(clippy::cast_precision_loss)]
pub fn floats(b: InkBox) -> [f64; 4] {
    match b {
        InkBox::Units(v) => v.map(|x| x as f64),
        InkBox::Float(v) => v,
    }
}
