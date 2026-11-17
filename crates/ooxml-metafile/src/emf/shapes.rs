//! Owns: EMF's drawing records -- poly-lines, -gons and -Béziers (32- and 16-bit points),
//! poly-polygons, `POLYDRAW`, boxes (rectangle, ellipse, rounded rectangle, with
//! `PS_INSIDEFRAME`), arcs, `ANGLEARC` and `SETPIXELV` -- and the arc of a box shared with
//! WMF ([`draw_arc`]).
//!
//! Must not: keep state of its own.

use super::Reader;
use crate::bytes::Bytes;
use crate::dc;
use crate::error::{Played, ReadError, RecordError};
use crate::geom;
use crate::player::Player;
use crate::pymath::radians;
use ooxml_scene::{Figure, Segment};
use std::f64::consts::PI;

/// What `ARC`, `ARCTO`, `CHORD` and `PIE` draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArcKind {
    Arc,
    ArcTo,
    Chord,
    Pie,
}

impl Reader<'_> {
    /// Points from `start`, their count at `count_at`, 16- or 32-bit, counted against the
    /// cap (a count past the record is cut to what it holds).
    fn points(
        &mut self,
        rec: Bytes<'_>,
        count_at: usize,
        start: usize,
        small: bool,
    ) -> Result<Vec<(f64, f64)>, RecordError> {
        let count = i64::from(rec.u32(count_at)?);
        let size = if small { 4 } else { 8 };
        let fits = (rec.len() as i64 - start as i64).div_euclid(size);
        let count = count.min(fits);
        self.p.count_points(count)?;
        let n = usize::try_from(count).unwrap_or(0);
        Ok(read_points(rec, start, n, small)?)
    }

    fn simple(&mut self, rec: Bytes<'_>) -> Result<Vec<(f64, f64)>, RecordError> {
        let small = matches!(rec.u32(0)?, 85..=89 | 92);
        self.points(rec, 24, 28, small)
    }

    pub(super) fn polybezier(&mut self, rec: Bytes<'_>) -> Played {
        let points = self.simple(rec)?;
        let fig = geom::beziers(&points);
        self.draw_open(fig)
    }

    pub(super) fn polyline(&mut self, rec: Bytes<'_>) -> Played {
        let points = self.simple(rec)?;
        let fig = geom::polyline(&points, false);
        self.draw_open(fig)
    }

    fn draw_open(&mut self, fig: Option<Figure>) -> Played {
        if self.p.in_path() {
            self.p.add_to_path(fig);
        } else {
            self.p.paint(fig.into_iter().collect(), false, true, None, None)?;
        }
        Ok(true)
    }

    pub(super) fn polygon(&mut self, rec: Bytes<'_>) -> Played {
        let points = self.simple(rec)?;
        self.p.shape(vec![geom::polyline(&points, true)], true)?;
        Ok(true)
    }

    pub(super) fn poly_to(&mut self, rec: Bytes<'_>, bezier: bool) -> Played {
        let points = self.simple(rec)?;
        self.p.poly_to(&points, bezier)?;
        Ok(true)
    }

    fn poly_poly(&mut self, rec: Bytes<'_>) -> Result<Vec<Vec<(f64, f64)>>, RecordError> {
        let small = matches!(rec.u32(0)?, 90 | 91);
        let (polys, total) = (rec.u32(24)? as usize, rec.u32(28)? as i64);
        let polys = polys.min(rec.len().saturating_sub(32) / 4);
        let counts = rec.many(32, polys, 4, Bytes::u32)?;
        let start = 32 + 4 * polys;
        let size = if small { 4 } else { 8 };
        let sum: i64 = counts.iter().map(|&c| i64::from(c)).sum();
        let total = total.min(sum).min((rec.len() as i64 - start as i64).div_euclid(size));
        self.p.count_points(total)?;
        let points = read_points(rec, start, usize::try_from(total).unwrap_or(0), small)?;
        let mut out = Vec::new();
        let mut k = 0usize;
        for n in counts {
            let end = k.saturating_add(n as usize);
            let chunk = points.get(k.min(points.len())..end.min(points.len())).unwrap_or(&[]);
            if !chunk.is_empty() {
                out.push(chunk.to_vec());
            }
            k = end;
        }
        Ok(out)
    }

    pub(super) fn polypolyline(&mut self, rec: Bytes<'_>) -> Played {
        let figures: Vec<_> = self.poly_poly(rec)?.iter().map(|p| geom::polyline(p, false)).collect();
        if self.p.in_path() {
            figures.into_iter().for_each(|f| self.p.add_to_path(f));
        } else {
            self.p
                .paint(figures.into_iter().flatten().collect(), false, true, None, None)?;
        }
        Ok(true)
    }

    pub(super) fn polypolygon(&mut self, rec: Bytes<'_>) -> Played {
        let figures = self.poly_poly(rec)?.iter().map(|p| geom::polyline(p, true)).collect();
        self.p.shape(figures, true)?;
        Ok(true)
    }

    pub(super) fn polydraw(&mut self, rec: Bytes<'_>) -> Played {
        let small = rec.u32(0)? == 92;
        let points = self.points(rec, 24, 28, small)?;
        let from = 28 + points.len() * if small { 4 } else { 8 };
        let types = rec.slice(from, from + points.len());
        let mut k = 0;
        while k < points.len() {
            let kind = types.get(k).map_or(2, |t| t & 0x06);
            let (x, y) = points.get(k).copied().unwrap_or_default();
            match kind {
                6 => {
                    self.p.move_to(x, y);
                    k += 1;
                }
                2 => {
                    self.p.line_to(x, y)?;
                    k += 1;
                }
                4 if k + 2 < points.len() => {
                    self.p.poly_to(points.get(k..k + 3).unwrap_or(&[]), true)?;
                    k += 3;
                }
                _ => {
                    k += 1;
                    continue;
                }
            }
            if types.get(k - 1).is_some_and(|t| t & 0x01 != 0) && self.p.in_path() {
                self.p.close_figure();
            }
        }
        Ok(true)
    }

    pub(super) fn set_pixel(&mut self, rec: Bytes<'_>) -> Played {
        let (x, y, color) = (rec.i32(8)?, rec.i32(12)?, rec.u32(16)?);
        self.p.pixel(f64::from(x), f64::from(y), color)?;
        Ok(true)
    }

    /// `PS_INSIDEFRAME`: a geometric pen's outline drawn inside the box.
    fn inside(&self, [l, t, r, b]: [f64; 4]) -> [f64; 4] {
        let pen = &self.p.dc.pen;
        if pen.kind() == dc::PS_INSIDEFRAME && pen.geometric && pen.width > 1.0 {
            let half = pen.width / 2.0;
            let (l, r) = if l <= r {
                (l + half, r - half)
            } else {
                (l - half, r + half)
            };
            let (t, b) = if t <= b {
                (t + half, b - half)
            } else {
                (t - half, b + half)
            };
            return [l, t, r, b];
        }
        [l, t, r, b]
    }

    pub(super) fn box_shape(&mut self, kind: u32, rec: Bytes<'_>) -> Played {
        let [l, t, r, b] = self.inside(Self::box_at(rec, 8)?);
        let fig = match kind {
            42 => geom::ellipse(l, t, r, b),
            43 => geom::rect(l, t, r, b),
            _ => {
                let (w, h) = (rec.i32(24)?, rec.i32(28)?);
                geom::round_rect(l, t, r, b, f64::from(w), f64::from(h))
            }
        };
        self.p.shape(vec![Some(fig)], true)?;
        Ok(true)
    }

    pub(super) fn arc(&mut self, kind: u32, rec: Bytes<'_>) -> Played {
        let rect = Self::box_at(rec, 8)?;
        let [xs, ys, xe, ye] = Self::box_at(rec, 24)?;
        let kind = match kind {
            45 => ArcKind::Arc,
            46 => ArcKind::Chord,
            47 => ArcKind::Pie,
            _ => ArcKind::ArcTo,
        };
        draw_arc(&mut self.p, kind, rect, [xs, ys, xe, ye])?;
        Ok(true)
    }

    pub(super) fn angle_arc(&mut self, rec: Bytes<'_>) -> Played {
        let (x, y, radius) = (f64::from(rec.i32(8)?), f64::from(rec.i32(12)?), f64::from(rec.u32(16)?));
        let (start, sweep) = (f64::from(rec.f32(20)?), f64::from(rec.f32(24)?));
        if !start.is_finite() || !sweep.is_finite() {
            return Ok(false);
        }
        let a0 = -radians(start);
        let delta = -radians(sweep);
        let (sx, sy) = (x + radius * a0.cos(), y + radius * a0.sin());
        let segments = geom::arc_segments(x, y, radius, radius, a0, delta)?;
        let end = segments.last().map(Segment::end);
        arc_to(&mut self.p, (sx, sy), segments)?;
        if let Some(end) = end {
            self.p.dc.position = end;
        }
        Ok(true)
    }
}

/// Points at `start`: `count` pairs of 16- or 32-bit integers.
pub(crate) fn read_points(
    rec: Bytes<'_>,
    start: usize,
    count: usize,
    small: bool,
) -> Result<Vec<(f64, f64)>, ReadError> {
    if small {
        rec.many(start, count, 4, |b, at| {
            Ok((f64::from(b.i16(at)?), f64::from(b.i16(at + 2)?)))
        })
    } else {
        rec.many(start, count, 8, |b, at| {
            Ok((f64::from(b.i32(at)?), f64::from(b.i32(at + 4)?)))
        })
    }
}

/// A line to `start` and the arc's segments from the current position: added to the open
/// path, or stroked.
fn arc_to(p: &mut Player<'_>, start: (f64, f64), segments: Vec<Segment>) -> Result<(), RecordError> {
    if p.in_path() {
        p.line_to(start.0, start.1)?;
        let m = p.matrix();
        if let Some(fig) = p.path_figure() {
            fig.segments.extend(segments.iter().map(|s| s.mapped(&m)));
        }
    } else {
        let mut fig = Figure::at(p.dc.position.0, p.dc.position.1);
        fig.segments.push(Segment::Line(start.0, start.1));
        fig.segments.extend(segments);
        p.paint(vec![fig], false, true, None, None)?;
    }
    Ok(())
}

/// `ARC`, `ARCTO`, `CHORD` and `PIE`: the ellipse of the box from the ray through the start
/// point to the ray through the end point, counter-clockwise unless `SETARCDIRECTION` says
/// otherwise.
pub(crate) fn draw_arc(p: &mut Player<'_>, kind: ArcKind, rect: [f64; 4], ends: [f64; 4]) -> Result<(), RecordError> {
    let [l, t, r, b] = rect;
    let [xs, ys, xe, ye] = ends;
    let (cx, cy, rx, ry) = ((l + r) / 2.0, (t + b) / 2.0, (r - l).abs() / 2.0, (b - t).abs() / 2.0);
    if rx == 0.0 || ry == 0.0 {
        return Ok(());
    }
    let a0 = ((ys - cy) / ry).atan2((xs - cx) / rx);
    let a1 = ((ye - cy) / ry).atan2((xe - cx) / rx);
    let turn = 2.0 * PI;
    let sweep = if p.dc.arc_direction == dc::AD_CLOCKWISE {
        Some(py_mod(a1 - a0, turn)).filter(|v| *v != 0.0).unwrap_or(turn)
    } else {
        -Some(py_mod(a0 - a1, turn)).filter(|v| *v != 0.0).unwrap_or(turn)
    };
    let start = (cx + rx * a0.cos(), cy + ry * a0.sin());
    let segments = geom::arc_segments(cx, cy, rx, ry, a0, sweep)?;
    let end = segments.last().map_or(start, Segment::end);
    if kind == ArcKind::ArcTo {
        arc_to(p, start, segments)?;
        p.dc.position = end;
        return Ok(());
    }
    let mut fig = Figure::at(start.0, start.1);
    fig.segments = segments;
    if kind == ArcKind::Pie {
        fig.segments.push(Segment::Line(cx, cy));
    }
    fig.closed = matches!(kind, ArcKind::Pie | ArcKind::Chord);
    if p.in_path() {
        p.add_to_path(Some(fig));
    } else {
        p.paint(vec![fig], kind != ArcKind::Arc, true, None, None)?;
    }
    Ok(())
}

/// The reference's float modulo: the result has the divisor's sign.
fn py_mod(a: f64, b: f64) -> f64 {
    let m = a % b;
    if m != 0.0 && (m < 0.0) != (b < 0.0) {
        m + b
    } else {
        m
    }
}
