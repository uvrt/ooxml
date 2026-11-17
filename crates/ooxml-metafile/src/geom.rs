//! Owns: the figures GDI's and GDI+'s shapes are made of -- polylines, Bézier runs,
//! rectangles, ellipses, rounded rectangles and elliptical arcs as cubic Béziers -- built as
//! the reference builds them, point for point.
//!
//! Must not: paint, or know a record.

use crate::error::ReadError;
use crate::pymath;
use ooxml_scene::{Figure, Segment};
use std::f64::consts::PI;

/// The most Bézier segments one arc may be split into (a quarter turn each): an arc of more
/// turns is a hostile record, refused as unreadable.
const MAX_ARC_SEGMENTS: f64 = 1_000_000.0;

/// A polyline through `points`, closed or not; `None` for no points.
pub fn polyline(points: &[(f64, f64)], closed: bool) -> Option<Figure> {
    let (&(x, y), rest) = points.split_first()?;
    let mut fig = Figure::at(x, y);
    fig.segments = rest.iter().map(|&(x, y)| Segment::Line(x, y)).collect();
    fig.closed = closed;
    Some(fig)
}

/// Cubic Béziers from `points[0]`, three points a segment; `None` for no points.
pub fn beziers(points: &[(f64, f64)]) -> Option<Figure> {
    let (&(x, y), _) = points.split_first()?;
    let mut fig = Figure::at(x, y);
    let mut i = 1;
    while i + 2 < points.len() {
        if let (Some(a), Some(b), Some(c)) = (points.get(i), points.get(i + 1), points.get(i + 2)) {
            fig.segments.push(Segment::Cubic(a.0, a.1, b.0, b.1, c.0, c.1));
        }
        i += 3;
    }
    Some(fig)
}

/// The closed rectangle `l, t` to `r, b`, clockwise from the top left.
pub fn rect(l: f64, t: f64, r: f64, b: f64) -> Figure {
    Figure {
        start: (l, t),
        segments: vec![Segment::Line(r, t), Segment::Line(r, b), Segment::Line(l, b)],
        closed: true,
    }
}

/// Cubic Béziers along an ellipse from parametric angle `start` by `sweep` (radians;
/// positive turns clockwise on a y-down page), a quarter turn at most each.
pub fn arc_segments(cx: f64, cy: f64, rx: f64, ry: f64, start: f64, sweep: f64) -> Result<Vec<Segment>, ReadError> {
    if sweep == 0.0 {
        return Ok(Vec::new());
    }
    let count = pymath::ceil(sweep.abs() / (PI / 2.0) - 1e-9)?.max(1.0);
    if count > MAX_ARC_SEGMENTS {
        return Err(ReadError::TooMuchWork);
    }
    let step = sweep / count;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let mut out = Vec::new();
    let mut t = start;
    let mut n = 0.0;
    while n < count {
        let (c0, s0, c1, s1) = (t.cos(), t.sin(), (t + step).cos(), (t + step).sin());
        out.push(Segment::Cubic(
            cx + rx * (c0 - k * s0),
            cy + ry * (s0 + k * c0),
            cx + rx * (c1 + k * s1),
            cy + ry * (s1 - k * c1),
            cx + rx * c1,
            cy + ry * s1,
        ));
        t += step;
        n += 1.0;
    }
    Ok(out)
}

/// The closed ellipse in the box `l, t, r, b`, from its rightmost point.
pub fn ellipse(l: f64, t: f64, r: f64, b: f64) -> Figure {
    let (cx, cy, rx, ry) = ((l + r) / 2.0, (t + b) / 2.0, (r - l).abs() / 2.0, (b - t).abs() / 2.0);
    let mut fig = Figure::at(cx + rx, cy);
    // A full turn is four segments: never refused.
    fig.segments = arc_segments(cx, cy, rx, ry, 0.0, 2.0 * PI).unwrap_or_default();
    fig.closed = true;
    fig
}

/// The rounded rectangle in the box with corner ellipses `w` by `h`.
pub fn round_rect(l: f64, t: f64, r: f64, b: f64, w: f64, h: f64) -> Figure {
    let (l, r) = (l.min(r), l.max(r));
    let (t, b) = (t.min(b), t.max(b));
    let (rx, ry) = ((w.abs() / 2.0).min((r - l) / 2.0), (h.abs() / 2.0).min((b - t) / 2.0));
    if rx <= 0.0 || ry <= 0.0 {
        return rect(l, t, r, b);
    }
    let hp = PI / 2.0;
    let quarter = |cx: f64, cy: f64, start: f64| arc_segments(cx, cy, rx, ry, start, hp).unwrap_or_default();
    let mut fig = Figure::at(l + rx, t);
    fig.segments.push(Segment::Line(r - rx, t));
    fig.segments.extend(quarter(r - rx, t + ry, -hp));
    fig.segments.push(Segment::Line(r, b - ry));
    fig.segments.extend(quarter(r - rx, b - ry, 0.0));
    fig.segments.push(Segment::Line(l + rx, b));
    fig.segments.extend(quarter(l + rx, b - ry, hp));
    fig.segments.push(Segment::Line(l, t + ry));
    fig.segments.extend(quarter(l + rx, t + ry, PI));
    fig.closed = true;
    fig
}
