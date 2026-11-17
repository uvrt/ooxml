//! Owns: the current position and the path bracket -- moves, lines and poly-segments that
//! either draw or add to the open path, `BEGINPATH` to `ENDPATH`, closing a figure, and
//! filling or stroking the closed bracket (a widened path filled as the pen's stroke in the
//! brush's colour).
//!
//! A shape inside a path bracket is added to the path, not drawn (Windows); a bracket is
//! kept in device coordinates, as GDI keeps it.
//!
//! Must not: read records.

use super::Player;
use crate::dc::Brush;
use crate::error::Halt;
use ooxml_scene::{Figure, Matrix, Segment};

impl Player<'_> {
    pub fn in_path(&self) -> bool {
        self.path.is_some()
    }

    /// The open figure of the bracket, started at the current position if needed.
    pub fn path_figure(&mut self) -> Option<&mut Figure> {
        let needs_new = self.path.as_ref().is_some_and(|p| p.last().is_none_or(|f| f.closed));
        if needs_new {
            let (x, y) = self.matrix().apply(self.dc.position.0, self.dc.position.1);
            if let Some(path) = self.path.as_mut() {
                path.push(Figure::at(x, y));
            }
        }
        self.path.as_mut().and_then(|p| p.last_mut())
    }

    /// Adds a logical figure to the bracket, mapped to device space.
    pub fn add_to_path(&mut self, figure: Option<Figure>) {
        let m = self.matrix();
        if let (Some(fig), Some(path)) = (figure, self.path.as_mut()) {
            path.push(fig.mapped(&m));
        }
    }

    pub fn move_to(&mut self, x: f64, y: f64) {
        self.dc.position = (x, y);
        let (dx, dy) = self.matrix().apply(x, y);
        if let Some(path) = self.path.as_mut() {
            match path.last_mut() {
                Some(last) if last.segments.is_empty() && !last.closed => *last = Figure::at(dx, dy),
                _ => path.push(Figure::at(dx, dy)),
            }
        }
    }

    pub fn line_to(&mut self, x: f64, y: f64) -> Result<(), Halt> {
        let (x0, y0) = self.dc.position;
        self.dc.position = (x, y);
        if self.in_path() {
            let (dx, dy) = self.matrix().apply(x, y);
            if let Some(fig) = self.path_figure() {
                fig.segments.push(Segment::Line(dx, dy));
            }
            Ok(())
        } else {
            self.segment(x0, y0, x, y)
        }
    }

    /// `POLYLINETO` and `POLYBEZIERTO`: from the current position on.
    pub fn poly_to(&mut self, points: &[(f64, f64)], bezier: bool) -> Result<(), Halt> {
        if points.is_empty() {
            return Ok(());
        }
        self.count_points(i64::try_from(points.len()).unwrap_or(i64::MAX))?;
        let start = self.dc.position;
        let points = if bezier {
            points.get(..points.len() / 3 * 3).unwrap_or(&[])
        } else {
            points
        };
        let Some(&last) = points.last() else {
            return Ok(());
        };
        self.dc.position = last;
        let segments = |pts: &[(f64, f64)]| -> Vec<Segment> {
            if bezier {
                pts.as_chunks::<3>()
                    .0
                    .iter()
                    .map(|[a, b, e]| Segment::Cubic(a.0, a.1, b.0, b.1, e.0, e.1))
                    .collect()
            } else {
                pts.iter().map(|&(x, y)| Segment::Line(x, y)).collect()
            }
        };
        if self.in_path() {
            let m = self.matrix();
            let mapped: Vec<(f64, f64)> = points.iter().map(|&(x, y)| m.apply(x, y)).collect();
            let new = segments(&mapped);
            if let Some(fig) = self.path_figure() {
                fig.segments.extend(new);
            }
            return Ok(());
        }
        let mut fig = Figure::at(start.0, start.1);
        fig.segments = segments(points);
        self.paint(vec![fig], false, true, None, None)
    }

    /// A drawing call outside a bracket paints; inside one it adds its figures.
    pub fn shape(&mut self, figures: Vec<Option<Figure>>, fill: bool) -> Result<(), Halt> {
        if self.in_path() {
            for fig in figures {
                self.add_to_path(fig);
            }
            return Ok(());
        }
        self.paint(figures.into_iter().flatten().collect(), fill, true, None, None)
    }

    pub fn begin_path(&mut self) -> Result<(), Halt> {
        self.flush()?;
        self.path = Some(Vec::new());
        self.path_done = None;
        self.path_widened = false;
        Ok(())
    }

    pub fn end_path(&mut self) {
        if let Some(path) = self.path.take() {
            self.path_done = Some(path);
        }
    }

    pub fn abort_path(&mut self) {
        self.path = None;
        self.path_done = None;
    }

    pub fn close_figure(&mut self) {
        let inverse = self.matrix().invert();
        let Some(fig) = self.path.as_mut().and_then(|p| p.last_mut()) else {
            return;
        };
        if fig.closed {
            return;
        }
        fig.closed = true;
        let start = fig.start;
        if let Some(inv) = inverse {
            self.dc.position = inv.apply(start.0, start.1);
        }
    }

    pub fn use_path(&mut self, fill: bool, stroke: bool) -> Result<(), Halt> {
        let Some(mut figures) = self.path_done.take().filter(|f| !f.is_empty()) else {
            return Ok(());
        };
        if fill && !stroke && self.path_widened {
            // FILLPATH after WIDENPATH fills the pen's outline with the brush: drawn as
            // the pen's stroke in the brush's colour.
            return self.paint_widened(figures);
        }
        if fill {
            figures.iter_mut().for_each(|f| f.closed = true);
        }
        let device = self.matrix();
        self.paint(figures, fill, stroke, Some(Matrix::IDENTITY), Some(device))
    }

    fn paint_widened(&mut self, figures: Vec<Figure>) -> Result<(), Halt> {
        let Brush::Solid(color) = self.dc.brush else {
            return Ok(());
        };
        let saved = self.dc.pen.clone();
        self.dc.pen.color = color;
        self.dc.pen.style &= !0x0F;
        let device = self.matrix();
        self.paint(figures, false, true, Some(Matrix::IDENTITY), Some(device))?;
        self.dc.pen = saved;
        Ok(())
    }
}
