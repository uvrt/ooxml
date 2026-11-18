//! Owns: a CFF table's charstrings read for their outlines' boxes (name-keyed Type 2 fonts
//! only: `CharStrings`, global and local subroutines) and a Type 2 interpreter that only
//! follows the pen (Adobe's Technical Notes 5176 and 5177), as docx2svg's `outlines.CFF`.
//!
//! Must not: hint, draw, or follow Type 2 arithmetic and storage.

use super::{Bounds, GlyphInk};
use crate::be::{i16_at, u16_at, u32_at};

/// `data[start..end]`, cut short as a Python slice is.
fn cut(data: &[u8], start: usize, end: usize) -> &[u8] {
    let start = start.min(data.len());
    data.get(start..end.clamp(start, data.len())).unwrap_or_default()
}

/// A CFF INDEX at `at`: its items, and the offset after it.
fn index(data: &[u8], at: usize) -> Option<(Vec<&[u8]>, usize)> {
    let count = usize::from(u16_at(data, at)?);
    if count == 0 {
        return Some((Vec::new(), at + 2));
    }
    let size = usize::from(*data.get(at + 2)?);
    let offsets: Vec<usize> = (0..=count)
        .map(|i| {
            let from = at + 3 + i * size;
            cut(data, from, from + size)
                .iter()
                .fold(0_usize, |v, b| (v << 8) | usize::from(*b))
        })
        .collect();
    let base = at + 2 + (count + 1) * size;
    let items = offsets
        .windows(2)
        .map(|w| match w {
            [a, b] => cut(data, base + a, base + b),
            _ => &[][..],
        })
        .collect();
    Some((items, base + offsets.last()?))
}

/// A CFF DICT: operator (`12 x` as `1200 + x`) to its operands; `None` for a reserved byte.
fn dict(data: &[u8]) -> Option<std::collections::HashMap<u16, Vec<i64>>> {
    let mut out = std::collections::HashMap::new();
    let mut operands = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let b0 = *data.get(at)?;
        match b0 {
            0..=21 => {
                if b0 == 12 {
                    out.insert(1200 + u16::from(*data.get(at + 1)?), std::mem::take(&mut operands));
                    at += 2;
                } else {
                    out.insert(u16::from(b0), std::mem::take(&mut operands));
                    at += 1;
                }
            }
            28 => {
                operands.push(i64::from(i16_at(data, at + 1)?));
                at += 3;
            }
            29 => {
                operands.push(i64::from(crate::be::u32_at(data, at + 1)?.cast_signed()));
                at += 5;
            }
            30 => {
                at += 1;
                while let Some(&byte) = data.get(at) {
                    at += 1;
                    if byte & 0x0F == 0x0F || byte >> 4 == 0x0F {
                        break;
                    }
                }
                operands.push(0);
            }
            32..=246 => {
                operands.push(i64::from(b0) - 139);
                at += 1;
            }
            247..=250 => {
                operands.push((i64::from(b0) - 247) * 256 + i64::from(*data.get(at + 1)?) + 108);
                at += 2;
            }
            251..=254 => {
                operands.push(-(i64::from(b0) - 251) * 256 - i64::from(*data.get(at + 1)?) - 108);
                at += 2;
            }
            _ => return None,
        }
    }
    Some(out)
}

fn bias(count: usize) -> i64 {
    if count < 1240 {
        107
    } else if count < 33900 {
        1131
    } else {
        32768
    }
}

/// A `CFF ` table's charstrings, for their outlines' boxes (name-keyed fonts only).
pub(crate) struct Cff<'a> {
    charstrings: Vec<&'a [u8]>,
    global_subrs: Vec<&'a [u8]>,
    subrs: Vec<&'a [u8]>,
}

impl<'a> Cff<'a> {
    /// The table read, `None` where it is not a name-keyed Type 2 CFF this reader follows.
    pub fn new(table: &'a [u8]) -> Option<Cff<'a>> {
        let at = usize::from(*table.get(2)?);
        let (_, at) = index(table, at)?;
        let (tops, at) = index(table, at)?;
        let (_, at) = index(table, at)?;
        let (global_subrs, _) = index(table, at)?;
        let top = dict(tops.first()?)?;
        let kind = match top.get(&1206) {
            Some(v) => *v.first()?,
            None => 2,
        };
        if top.contains_key(&1230) || kind != 2 {
            return None;
        }
        let charstrings_at = usize::try_from(*top.get(&17)?.first()?).ok()?;
        let (charstrings, _) = index(table, charstrings_at)?;
        let mut subrs = Vec::new();
        if let Some(private) = top.get(&18) {
            let [size, offset] = private.as_slice() else {
                return None;
            };
            let (size, offset) = (usize::try_from(*size).ok()?, usize::try_from(*offset).ok()?);
            let private = dict(cut(table, offset, offset + size))?;
            if let Some(local) = private.get(&19) {
                let at = offset.checked_add_signed(isize::try_from(*local.first()?).ok()?)?;
                subrs = index(table, at)?.0;
            }
        }
        Some(Cff {
            charstrings,
            global_subrs,
            subrs,
        })
    }

    /// As [`Glyf::outline_box`], for charstring `glyph`.
    pub fn outline_box(&self, glyph: u32, slant: f64) -> Option<GlyphInk> {
        let code = self.charstrings.get(usize::try_from(glyph).ok()?)?;
        let mut run = Type2 {
            cff: self,
            bounds: Bounds::new(slant),
            stack: Vec::new(),
            stems: 0,
            width_done: false,
            x: 0.0,
            y: 0.0,
        };
        match run.run(code, 0)? {
            Some(true) => Some(run.bounds.ink()),
            _ => None,
        }
    }
}

/// A Type 2 charstring interpreter that only follows the pen (Technical Note 5177).
struct Type2<'c, 'a> {
    cff: &'c Cff<'a>,
    bounds: Bounds,
    stack: Vec<f64>,
    stems: usize,
    width_done: bool,
    x: f64,
    y: f64,
}

/// What following a program came to: `Some(true)` at `endchar`, `Some(false)` where it
/// cannot be followed, `None` at a subroutine's `return` (or its end).
type Ran = Option<bool>;

impl Type2<'_, '_> {
    fn width(&mut self, even: bool) {
        if !self.width_done {
            self.width_done = true;
            if (self.stack.len() % 2 == 1) == even && !self.stack.is_empty() {
                self.stack.remove(0);
            }
        }
    }

    fn arg(&self, k: usize) -> Option<f64> {
        self.stack.get(k).copied()
    }

    fn last(&self, k: usize) -> Option<f64> {
        self.stack.len().checked_sub(k).and_then(|i| self.stack.get(i)).copied()
    }

    fn move_to(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
        self.bounds.point(self.x, self.y);
    }

    fn curve(&mut self, v: [f64; 6]) {
        let [a, b, c, d, e, f] = v;
        let p0 = (self.x, self.y);
        let p1 = (p0.0 + a, p0.1 + b);
        let p2 = (p1.0 + c, p1.1 + d);
        let p3 = (p2.0 + e, p2.1 + f);
        self.bounds.cubic(p0, p1, p2, p3);
        (self.x, self.y) = p3;
    }

    fn six(&self, from: usize) -> Option<[f64; 6]> {
        Some([
            self.arg(from)?,
            self.arg(from + 1)?,
            self.arg(from + 2)?,
            self.arg(from + 3)?,
            self.arg(from + 4)?,
            self.arg(from + 5)?,
        ])
    }

    /// Follow `code`; the outer `None` is an error (docx2svg's caught exceptions).
    fn run(&mut self, code: &[u8], depth: usize) -> Option<Ran> {
        if depth > 10 {
            return Some(Some(false));
        }
        let mut at = 0;
        while at < code.len() {
            let b0 = *code.get(at)?;
            at += 1;
            if b0 >= 32 || b0 == 28 {
                let value = number(b0, code, &mut at)?;
                self.stack.push(value);
                continue;
            }
            // Every operator followed counts against the glyph's budget: subroutines that
            // call one another over and over are refused, not followed for ever.
            if !self.bounds.follow(1) {
                return Some(Some(false));
            }
            if let Step::Done(ran) = self.operator(b0, code, &mut at, depth)? {
                return Some(ran);
            }
        }
        Some(None)
    }

    fn operator(&mut self, b0: u8, code: &[u8], at: &mut usize, depth: usize) -> Option<Step> {
        match b0 {
            1 | 3 | 18 | 23 | 19 | 20 => {
                self.width(true);
                self.stems += self.stack.len() / 2;
                self.stack.clear();
                if matches!(b0, 19 | 20) {
                    *at += self.stems.div_ceil(8);
                }
            }
            21 | 22 | 4 => self.moves(b0)?,
            5..=7 => self.lines(b0),
            8 | 24 | 25 => self.curves(b0)?,
            26 | 27 | 30 | 31 => self.axis_curves(b0)?,
            10 | 29 => return self.call(b0, depth),
            11 => return Some(Step::Done(None)),
            14 => {
                self.width(true);
                return Some(Step::Done(Some(self.stack.is_empty())));
            }
            12 => {
                let b1 = *code.get(*at)?;
                *at += 1;
                return self.flex(b1);
            }
            _ => return Some(Step::Done(Some(false))),
        }
        Some(Step::Go)
    }

    fn moves(&mut self, b0: u8) -> Option<()> {
        if b0 == 21 {
            self.width(true);
            let (dx, dy) = (self.last(2)?, self.last(1)?);
            self.move_to(dx, dy);
        } else {
            self.width(false);
            let v = self.last(1)?;
            if b0 == 22 {
                self.move_to(v, 0.0);
            } else {
                self.move_to(0.0, v);
            }
        }
        self.stack.clear();
        Some(())
    }

    fn lines(&mut self, b0: u8) {
        let values = std::mem::take(&mut self.stack);
        if b0 == 5 {
            for [dx, dy] in values.as_chunks::<2>().0 {
                self.move_to(*dx, *dy);
            }
            return;
        }
        let mut horizontal = b0 == 6;
        for value in values {
            if horizontal {
                self.move_to(value, 0.0);
            } else {
                self.move_to(0.0, value);
            }
            horizontal = !horizontal;
        }
    }

    /// `rrcurveto`, `rcurveline`, `rlinecurve`.
    fn curves(&mut self, b0: u8) -> Option<()> {
        let len = self.stack.len();
        match b0 {
            8 => {
                for k in 0..len.saturating_sub(5).div_ceil(6) {
                    let v = self.six(6 * k)?;
                    self.curve(v);
                }
            }
            24 => {
                for k in 0..len.checked_sub(2)? / 6 {
                    let v = self.six(6 * k)?;
                    self.curve(v);
                }
                let (dx, dy) = (self.last(2)?, self.last(1)?);
                self.move_to(dx, dy);
            }
            _ => {
                let lines = len.checked_sub(6)?;
                let mut k = 0;
                while k < lines {
                    let (dx, dy) = (self.arg(k)?, self.arg(k + 1)?);
                    self.move_to(dx, dy);
                    k += 2;
                }
                let v = self.six(lines)?;
                self.curve(v);
            }
        }
        self.stack.clear();
        Some(())
    }

    /// `vvcurveto`, `hhcurveto`, `vhcurveto`, `hvcurveto`.
    fn axis_curves(&mut self, b0: u8) -> Option<()> {
        let mut args: std::collections::VecDeque<f64> = std::mem::take(&mut self.stack).into();
        if matches!(b0, 26 | 27) {
            let mut first = if args.len() % 4 == 1 { args.pop_front()? } else { 0.0 };
            while args.len() >= 4 {
                let (a, b, c, d) = (
                    args.pop_front()?,
                    args.pop_front()?,
                    args.pop_front()?,
                    args.pop_front()?,
                );
                if b0 == 26 {
                    self.curve([first, a, b, c, 0.0, d]);
                } else {
                    self.curve([a, first, b, c, d, 0.0]);
                }
                first = 0.0;
            }
            return Some(());
        }
        let mut horizontal = b0 == 31;
        while args.len() >= 4 {
            let (a, b, c, d) = (
                args.pop_front()?,
                args.pop_front()?,
                args.pop_front()?,
                args.pop_front()?,
            );
            let last = if args.len() == 1 { args.pop_front()? } else { 0.0 };
            if horizontal {
                self.curve([a, 0.0, b, c, last, d]);
            } else {
                self.curve([0.0, a, b, c, d, last]);
            }
            horizontal = !horizontal;
        }
        Some(())
    }

    /// `callsubr`, `callgsubr`.
    fn call(&mut self, b0: u8, depth: usize) -> Option<Step> {
        let subrs = if b0 == 10 {
            &self.cff.subrs
        } else {
            &self.cff.global_subrs
        };
        #[allow(clippy::cast_possible_truncation)]
        let index = self.stack.pop()?.trunc() as i64 + bias(subrs.len());
        let length = i64::try_from(subrs.len()).ok()?;
        let index = if index < 0 { index + length } else { index };
        let code = *subrs.get(usize::try_from(index).ok()?)?;
        Some(match self.run(code, depth + 1)? {
            Some(done) => Step::Done(Some(done)),
            None => Step::Go,
        })
    }

    /// The flex operators (`12 34` to `12 37`); any other escaped operator is not followed.
    fn flex(&mut self, b1: u8) -> Option<Step> {
        let s: Vec<f64> = self.stack.clone();
        let g = |k: usize| s.get(k).copied();
        match b1 {
            35 => {
                let (first, second) = (self.six(0)?, self.six(6)?);
                self.curve(first);
                self.curve(second);
            }
            34 => {
                let (dx1, dx2, dy2, dx3, dx4, dx5, dx6) = (g(0)?, g(1)?, g(2)?, g(3)?, g(4)?, g(5)?, g(6)?);
                self.curve([dx1, 0.0, dx2, dy2, dx3, 0.0]);
                self.curve([dx4, 0.0, dx5, -dy2, dx6, 0.0]);
            }
            36 => {
                let (dx1, dy1, dx2, dy2, dx3) = (g(0)?, g(1)?, g(2)?, g(3)?, g(4)?);
                let (dx4, dx5, dy5, dx6) = (g(5)?, g(6)?, g(7)?, g(8)?);
                self.curve([dx1, dy1, dx2, dy2, dx3, 0.0]);
                self.curve([dx4, 0.0, dx5, dy5, dx6, -(dy1 + dy2 + dy5)]);
            }
            37 => {
                g(10)?;
                let sum = |from: usize| s.iter().skip(from).step_by(2).take(5).fold(0.0, |t, v| t + v);
                let (dx, dy) = (sum(0), sum(1));
                self.curve(self.six(0)?);
                let (a, b, c, d, e) = (g(6)?, g(7)?, g(8)?, g(9)?, g(10)?);
                if dx.abs() > dy.abs() {
                    self.curve([a, b, c, d, e, -dy]);
                } else {
                    self.curve([a, b, c, d, -dx, e]);
                }
            }
            _ => return Some(Step::Done(Some(false))),
        }
        self.stack.clear();
        Some(Step::Go)
    }
}

/// What an operator leaves: go on, or stop with what the program came to.
enum Step {
    Go,
    Done(Ran),
}

/// A Type 2 number operand starting with `b0`, the reading moved past it.
fn number(b0: u8, code: &[u8], at: &mut usize) -> Option<f64> {
    Some(match b0 {
        28 => {
            *at += 2;
            f64::from(i16_at(code, *at - 2)?)
        }
        32..=246 => f64::from(b0) - 139.0,
        247..=250 => {
            *at += 1;
            (f64::from(b0) - 247.0) * 256.0 + f64::from(*code.get(*at - 1)?) + 108.0
        }
        251..=254 => {
            *at += 1;
            -(f64::from(b0) - 251.0) * 256.0 - f64::from(*code.get(*at - 1)?) - 108.0
        }
        _ => {
            *at += 4;
            f64::from(u32_at(code, *at - 4)?.cast_signed()) / 65536.0
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cff_dict_reads_its_operands() {
        let found = dict(&[139, 140, 17, 28, 1, 0, 12, 6]).unwrap_or_default();
        assert_eq!(found.get(&17), Some(&vec![0, 1]));
        assert_eq!(found.get(&1206), Some(&vec![256]));
        assert!(dict(&[255]).is_none());
    }
}
