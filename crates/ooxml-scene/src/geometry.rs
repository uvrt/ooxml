//! Owns: the scene's geometry -- affine matrices ([`Matrix`]: compose, apply, invert, mean
//! scale) and paths ([`PathData`]: figures of line and cubic segments, open or closed).
//!
//! Must not: know where a path came from or how it is painted.

/// An affine map `x' = a x + c y + e`, `y' = b x + d y + f` (SVG's `matrix(a b c d e f)`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    /// `a`.
    pub a: f64,
    /// `b`.
    pub b: f64,
    /// `c`.
    pub c: f64,
    /// `d`.
    pub d: f64,
    /// `e`.
    pub e: f64,
    /// `f`.
    pub f: f64,
}

impl Matrix {
    /// The identity.
    pub const IDENTITY: Matrix = Matrix::new(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);

    /// The matrix of its six values, in SVG's order.
    pub const fn new(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> Matrix {
        Matrix { a, b, c, d, e, f }
    }

    /// A translation.
    pub const fn translate(x: f64, y: f64) -> Matrix {
        Matrix::new(1.0, 0.0, 0.0, 1.0, x, y)
    }

    /// A scale about the origin.
    pub const fn scale(x: f64, y: f64) -> Matrix {
        Matrix::new(x, 0.0, 0.0, y, 0.0, 0.0)
    }

    /// The six values, in SVG's order.
    pub fn values(&self) -> [f64; 6] {
        [self.a, self.b, self.c, self.d, self.e, self.f]
    }

    /// Whether this is the identity (each value equal to the identity's, `-0` to `0`).
    pub fn is_identity(&self) -> bool {
        *self == Matrix::IDENTITY
    }

    /// Whether every value is finite.
    pub fn is_finite(&self) -> bool {
        self.values().iter().all(|v| v.is_finite())
    }

    /// The map that applies `self` and then `then`.
    pub fn then(&self, then: &Matrix) -> Matrix {
        let (a1, b1, c1, d1, e1, f1) = (self.a, self.b, self.c, self.d, self.e, self.f);
        let (a2, b2, c2, d2, e2, f2) = (then.a, then.b, then.c, then.d, then.e, then.f);
        Matrix::new(
            a1 * a2 + b1 * c2,
            a1 * b2 + b1 * d2,
            c1 * a2 + d1 * c2,
            c1 * b2 + d1 * d2,
            e1 * a2 + f1 * c2 + e2,
            e1 * b2 + f1 * d2 + f2,
        )
    }

    /// The image of a point.
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    /// The inverse, or `None` when the determinant is zero or not finite.
    pub fn invert(&self) -> Option<Matrix> {
        let Matrix { a, b, c, d, e, f } = *self;
        let det = a * d - b * c;
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        Some(Matrix::new(
            d / det,
            -b / det,
            -c / det,
            a / det,
            (c * f - d * e) / det,
            (b * e - a * f) / det,
        ))
    }

    /// How much the matrix scales a length on average: the square root of its area.
    pub fn mean_scale(&self) -> f64 {
        (self.a * self.d - self.b * self.c).abs().sqrt()
    }
}

/// One segment of a figure, ending at its last point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    /// A straight line to `(x, y)`.
    Line(f64, f64),
    /// A cubic Bézier: two control points, then the end point.
    Cubic(f64, f64, f64, f64, f64, f64),
}

impl Segment {
    /// The end point.
    pub fn end(&self) -> (f64, f64) {
        match *self {
            Segment::Line(x, y) | Segment::Cubic(_, _, _, _, x, y) => (x, y),
        }
    }

    /// The segment's points mapped by `m`.
    pub fn mapped(&self, m: &Matrix) -> Segment {
        match *self {
            Segment::Line(x, y) => {
                let (x, y) = m.apply(x, y);
                Segment::Line(x, y)
            }
            Segment::Cubic(x1, y1, x2, y2, x, y) => {
                let (x1, y1) = m.apply(x1, y1);
                let (x2, y2) = m.apply(x2, y2);
                let (x, y) = m.apply(x, y);
                Segment::Cubic(x1, y1, x2, y2, x, y)
            }
        }
    }
}

/// One subpath: a start point, its segments, and whether it is closed.
#[derive(Debug, Clone, PartialEq)]
pub struct Figure {
    /// Where it starts.
    pub start: (f64, f64),
    /// Its segments, in order.
    pub segments: Vec<Segment>,
    /// Whether it is closed (`Z`).
    pub closed: bool,
}

impl Figure {
    /// An empty, open figure at `(x, y)`.
    pub fn at(x: f64, y: f64) -> Figure {
        Figure {
            start: (x, y),
            segments: Vec::new(),
            closed: false,
        }
    }

    /// Where it ends: its last segment's end, or its start.
    pub fn end(&self) -> (f64, f64) {
        self.segments.last().map_or(self.start, Segment::end)
    }

    /// The figure mapped by `m`.
    pub fn mapped(&self, m: &Matrix) -> Figure {
        let (x, y) = m.apply(self.start.0, self.start.1);
        Figure {
            start: (x, y),
            segments: self.segments.iter().map(|s| s.mapped(m)).collect(),
            closed: self.closed,
        }
    }
}

/// A path: its figures, in order (SVG's `d`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PathData {
    /// The figures.
    pub figures: Vec<Figure>,
}

impl PathData {
    /// A path of these figures.
    pub fn new(figures: Vec<Figure>) -> PathData {
        PathData { figures }
    }

    /// Whether it has no figure.
    pub fn is_empty(&self) -> bool {
        self.figures.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrices_compose_apply_and_invert() {
        let m = Matrix::new(2.0, 0.0, 0.0, 4.0, 5.0, 7.0);
        assert_eq!(m.apply(1.0, 1.0), (7.0, 11.0));
        let back = m.invert().unwrap();
        assert!(m.then(&back).is_identity());
        assert_eq!(m.mean_scale(), 8f64.sqrt());
        assert!(Matrix::scale(0.0, 1.0).invert().is_none());
    }
}
