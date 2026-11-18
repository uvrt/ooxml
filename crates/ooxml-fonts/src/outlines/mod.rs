//! Owns: the box round a glyph's outline, exact -- the curves' extremes, not their control
//! points -- from a TrueType `glyf` outline (quadratic, composite glyphs followed) or a CFF
//! `CharStrings` Type 2 program (cubic, subroutines followed), optionally slanted
//! (`x + slant * y`): what a VML text path that fits its shape is stretched by. A port of
//! docx2svg's `outlines.py` (its boxes), number for number: a `glyf` header's box where the
//! outline is upright, floats where it is followed.
//!
//! What is not read is `None` (a CFF2 or CID-keyed table, a `seac` accent, Type 2
//! arithmetic, a composite placed by point numbers), as docx2svg's reader answers.
//!
//! Must not: draw or rasterise an outline, or hint it.

use crate::be::{i16_at, u16_at, u32_at};

mod cff;

/// A glyph's box, font units, y up: `[x min, y min, x max, y max]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InkBox {
    /// Integers, as a `glyf` header states them (an upright TrueType glyph).
    Units([i64; 4]),
    /// Floats, where the outline was followed (slanted, or CFF).
    Float([f64; 4]),
}

/// What a glyph's outline is: a box, nothing at all (a space), or not read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GlyphInk {
    /// Its box.
    Box(InkBox),
    /// An empty glyph.
    Empty,
}

/// The running box of segments, each in the slanted frame (`_Bounds`).
pub(crate) struct Bounds {
    slant: f64,
    found: Option<[f64; 4]>,
}

impl Bounds {
    pub(crate) fn new(slant: f64) -> Bounds {
        Bounds { slant, found: None }
    }

    pub(crate) fn point(&mut self, x: f64, y: f64) {
        self.slanted(x + self.slant * y, y);
    }

    fn slanted(&mut self, x: f64, y: f64) {
        self.found = Some(match self.found {
            None => [x, y, x, y],
            Some([a, b, c, d]) => [py_min(a, x), py_min(b, y), py_max(c, x), py_max(d, y)],
        });
    }

    fn turned(&self, p: (f64, f64)) -> [f64; 2] {
        [p.0 + self.slant * p.1, p.1]
    }

    /// A quadratic segment's extremes (its end `p2`; `p0` was added before).
    fn quad(&mut self, p0: (f64, f64), p1: (f64, f64), p2: (f64, f64)) {
        let (a, b, c) = (self.turned(p0), self.turned(p1), self.turned(p2));
        self.slanted(c[0], c[1]);
        for axis in 0..2 {
            let d = at(&a, axis) - 2.0 * at(&b, axis) + at(&c, axis);
            if d != 0.0 {
                let t = (at(&a, axis) - at(&b, axis)) / d;
                if 0.0 < t && t < 1.0 {
                    let w = [(1.0 - t).powf(2.0), 2.0 * t * (1.0 - t), t * t];
                    let along = |k: usize| 0.0 + w[0] * at(&a, k) + w[1] * at(&b, k) + w[2] * at(&c, k);
                    self.slanted(along(0), along(1));
                }
            }
        }
    }

    /// A cubic segment's extremes (its end `p3`; `p0` was added before).
    pub(crate) fn cubic(&mut self, p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), p3: (f64, f64)) {
        let (a, b, c, d) = (self.turned(p0), self.turned(p1), self.turned(p2), self.turned(p3));
        self.slanted(d[0], d[1]);
        for axis in 0..2 {
            let (pa, pb, pc, pd) = (at(&a, axis), at(&b, axis), at(&c, axis), at(&d, axis));
            let q = -pa + 3.0 * pb - 3.0 * pc + pd;
            let r = 2.0 * (pa - 2.0 * pb + pc);
            let u = pb - pa;
            let mut roots = Vec::new();
            if q.abs() < 1e-12 {
                if r != 0.0 {
                    roots.push(-u / r);
                }
            } else {
                let disc = r * r - 4.0 * q * u;
                if disc >= 0.0 {
                    let root = disc.powf(0.5);
                    roots.push((-r + root) / (2.0 * q));
                    roots.push((-r - root) / (2.0 * q));
                }
            }
            for t in roots {
                if 0.0 < t && t < 1.0 {
                    let m = 1.0 - t;
                    let w = [m.powf(3.0), 3.0 * t * m * m, 3.0 * t * t * m, t.powf(3.0)];
                    let along =
                        |k: usize| 0.0 + w[0] * at(&a, k) + w[1] * at(&b, k) + w[2] * at(&c, k) + w[3] * at(&d, k);
                    self.slanted(along(0), along(1));
                }
            }
        }
    }

    pub(crate) fn ink(&self) -> GlyphInk {
        self.found.map_or(GlyphInk::Empty, |b| GlyphInk::Box(InkBox::Float(b)))
    }
}

fn at(p: &[f64; 2], axis: usize) -> f64 {
    p.get(axis).copied().unwrap_or(0.0)
}

/// Python's `min`: the first of equals (`-0.0` and `0.0` kept as met).
fn py_min(a: f64, b: f64) -> f64 {
    if b < a {
        b
    } else {
        a
    }
}

/// Python's `max`: the first of equals.
fn py_max(a: f64, b: f64) -> f64 {
    if b > a {
        b
    } else {
        a
    }
}

// -- TrueType ------------------------------------------------------------------------------

/// The outline tables of a TrueType face.
pub(crate) struct Glyf<'a> {
    /// `glyf`.
    pub glyf: &'a [u8],
    /// `loca`.
    pub loca: &'a [u8],
    /// `head.indexToLocFormat` is 1.
    pub long_offsets: bool,
}

/// An affine matrix `(a, b, c, d, e, f)`.
type Matrix = [f64; 6];

const UNIT_MATRIX: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

impl Glyf<'_> {
    /// Where glyph `glyph`'s data lies in `glyf`, from `loca`; `None` past its end.
    pub fn range(&self, glyph: u32) -> Option<(usize, usize)> {
        let size = if self.long_offsets { 4 } else { 2 };
        let index = usize::try_from(glyph).ok()?;
        if (index + 2) * size > self.loca.len() {
            return None;
        }
        if self.long_offsets {
            let start = u32_at(self.loca, index * 4)?;
            let end = u32_at(self.loca, index * 4 + 4)?;
            Some((usize::try_from(start).ok()?, usize::try_from(end).ok()?))
        } else {
            let start = u16_at(self.loca, index * 2)?;
            let end = u16_at(self.loca, index * 2 + 2)?;
            Some((2 * usize::from(start), 2 * usize::from(end)))
        }
    }

    /// The box of an upright glyph: its header's.
    pub fn header_box(&self, glyph: u32) -> Option<GlyphInk> {
        let (start, end) = self.range(glyph)?;
        if start == end {
            return Some(GlyphInk::Empty);
        }
        if start + 10 <= end && end <= self.glyf.len() {
            let v = |k: usize| i16_at(self.glyf, start + 2 + 2 * k).map(i64::from);
            return Some(GlyphInk::Box(InkBox::Units([v(0)?, v(1)?, v(2)?, v(3)?])));
        }
        None
    }

    /// The exact box of glyph `glyph`'s outline, slanted by `slant` (`glyf_box`).
    pub fn outline_box(&self, glyph: u32, slant: f64) -> Option<GlyphInk> {
        let mut bounds = Bounds::new(slant);
        self.outline(glyph, UNIT_MATRIX, &mut bounds, 0)?.then(|| bounds.ink())
    }

    /// Glyph `glyph`'s outline under `matrix` into `bounds`: `Some(false)` where it cannot
    /// be followed, `None` where its data is cut short.
    fn outline(&self, glyph: u32, matrix: Matrix, bounds: &mut Bounds, depth: usize) -> Option<bool> {
        let Some((start, end)) = self.range(glyph).filter(|_| depth <= 8) else {
            return Some(false);
        };
        if end <= start {
            return Some(true);
        }
        if end > self.glyf.len() || end - start < 10 {
            return Some(false);
        }
        let contours = i16_at(self.glyf, start)?;
        let mut at = start + 10;
        if contours < 0 {
            return self.composite(at, end, matrix, bounds, depth);
        }
        let contours = usize::try_from(contours).ok()?;
        let ends: Vec<usize> = (0..contours)
            .map(|k| u16_at(self.glyf, at + 2 * k).map(usize::from))
            .collect::<Option<_>>()?;
        at += 2 * contours;
        let count = ends.last().map_or(0, |e| e + 1);
        at += 2 + usize::from(u16_at(self.glyf, at)?);
        let mut flags: Vec<u8> = Vec::new();
        while flags.len() < count {
            let flag = *self.glyf.get(at)?;
            at += 1;
            let mut repeat = 1;
            if flag & 8 != 0 {
                repeat += usize::from(*self.glyf.get(at)?);
                at += 1;
            }
            flags.extend(std::iter::repeat_n(flag, repeat));
        }
        let mut coordinates: Vec<Vec<i64>> = Vec::new();
        for (short, same) in [(2_u8, 16_u8), (4, 32)] {
            let mut value = 0_i64;
            let mut values = Vec::with_capacity(count);
            for &flag in flags.iter().take(count) {
                if flag & short != 0 {
                    let delta = i64::from(*self.glyf.get(at)?);
                    at += 1;
                    value += if flag & same != 0 { delta } else { -delta };
                } else if flag & same == 0 {
                    value += i64::from(i16_at(self.glyf, at)?);
                    at += 2;
                }
                values.push(value);
            }
            coordinates.push(values);
        }
        let [a, b, c, d, e, f] = matrix;
        let xs = coordinates.first()?;
        let ys = coordinates.get(1)?;
        #[allow(clippy::cast_precision_loss)]
        let points: Vec<(f64, f64, bool)> = xs
            .iter()
            .zip(ys)
            .zip(&flags)
            .map(|((&x, &y), &flag)| {
                let (x, y) = (x as f64, y as f64);
                (a * x + c * y + e, b * x + d * y + f, flag & 1 != 0)
            })
            .collect();
        let mut first = 0;
        for last in ends {
            contour(points.get(first..=last).unwrap_or_default(), bounds);
            first = last + 1;
        }
        Some(true)
    }

    fn composite(&self, mut at: usize, end: usize, matrix: Matrix, bounds: &mut Bounds, depth: usize) -> Option<bool> {
        loop {
            if at + 4 > end {
                return Some(false);
            }
            let flags = u16_at(self.glyf, at)?;
            let component = u16_at(self.glyf, at + 2)?;
            at += 4;
            let (dx, dy) = if flags & 1 != 0 {
                let v = (i16_at(self.glyf, at)?, i16_at(self.glyf, at + 2)?);
                at += 4;
                (f64::from(v.0), f64::from(v.1))
            } else {
                let byte = |k: usize| self.glyf.get(k).map(|b| f64::from(i8::from_be_bytes([*b])));
                let v = (byte(at)?, byte(at + 1)?);
                at += 2;
                v
            };
            if flags & 2 == 0 {
                return Some(false);
            }
            let scale = |k: usize| i16_at(self.glyf, k).map(|v| f64::from(v) / 16384.0);
            let (mut a, mut b, mut c, mut d) = (1.0, 0.0, 0.0, 1.0);
            if flags & 8 != 0 {
                a = scale(at)?;
                d = a;
                at += 2;
            } else if flags & 0x40 != 0 {
                a = scale(at)?;
                d = scale(at + 2)?;
                at += 4;
            } else if flags & 0x80 != 0 {
                a = scale(at)?;
                b = scale(at + 2)?;
                c = scale(at + 4)?;
                d = scale(at + 6)?;
                at += 8;
            }
            let [pa, pb, pc, pd, pe, pf] = matrix;
            let inner = [
                pa * a + pc * b,
                pb * a + pd * b,
                pa * c + pc * d,
                pb * c + pd * d,
                pa * dx + pc * dy + pe,
                pb * dx + pd * dy + pf,
            ];
            if !self.outline(u32::from(component), inner, bounds, depth + 1)? {
                return Some(false);
            }
            if flags & 0x20 == 0 {
                return Some(true);
            }
        }
    }
}

/// A point of a contour: x, y, on the curve.
type Point = (f64, f64, bool);

/// One contour's segments into `bounds` (`_contour`).
fn contour(points: &[Point], bounds: &mut Bounds) {
    let Some(&first_point) = points.first() else {
        return;
    };
    let on = points.iter().position(|p| p.2);
    let (start, rest): ((f64, f64), Vec<Point>) = match on {
        None => {
            let second = points.get(1 % points.len()).copied().unwrap_or(first_point);
            (
                ((first_point.0 + second.0) / 2.0, (first_point.1 + second.1) / 2.0),
                points.to_vec(),
            )
        }
        Some(on) => {
            let start = points.get(on).map_or((0.0, 0.0), |p| (p.0, p.1));
            let mut rest: Vec<_> = points.get(on + 1..).unwrap_or_default().to_vec();
            rest.extend_from_slice(points.get(..on).unwrap_or_default());
            (start, rest)
        }
    };
    bounds.point(start.0, start.1);
    let mut current = start;
    let mut control: Option<(f64, f64)> = None;
    for (x, y, on_curve) in rest.into_iter().chain(std::iter::once((start.0, start.1, true))) {
        if on_curve {
            match control {
                None => bounds.point(x, y),
                Some(ctrl) => bounds.quad(current, ctrl, (x, y)),
            }
            current = (x, y);
            control = None;
        } else if let Some(ctrl) = control {
            let middle = ((ctrl.0 + x) / 2.0, (ctrl.1 + y) / 2.0);
            bounds.quad(current, ctrl, middle);
            current = middle;
            control = Some((x, y));
        } else {
            control = Some((x, y));
        }
    }
}

pub(crate) use cff::Cff;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quadratic_bulge_is_its_extreme_not_its_control_point() {
        let mut bounds = Bounds::new(0.0);
        bounds.point(0.0, 0.0);
        bounds.quad((0.0, 0.0), (50.0, 100.0), (100.0, 0.0));
        assert_eq!(bounds.found, Some([0.0, 0.0, 100.0, 50.0]));
    }

    #[test]
    fn a_slant_moves_x_by_y() {
        let mut bounds = Bounds::new(0.5);
        bounds.point(0.0, 10.0);
        bounds.point(10.0, 0.0);
        assert_eq!(bounds.found, Some([5.0, 0.0, 10.0, 10.0]));
    }
}
