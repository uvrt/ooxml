//! Owns: GDI's clip region as chained clip paths in device space -- copy, intersect and
//! exclude combine; a union with no clip is no clip; the rest are named as not drawn.
//!
//! Must not: draw.

use super::Player;
use crate::dc::{RGN_AND, RGN_COPY, RGN_DIFF, RGN_OR};
use crate::error::Halt;
use crate::geom;
use ooxml_scene::{Def, Figure, Path, PathData, ResourceId, ResourceKind};

impl Player<'_> {
    /// A clip path of `data` (even-odd or not) inside `parent`.
    pub fn clip_of(&mut self, data: PathData, even_odd: bool, parent: Option<ResourceId>) -> Result<ResourceId, Halt> {
        let id = self.new_id(ResourceKind::Clip);
        self.define(Def::ClipPath {
            id,
            parent,
            paths: vec![Path {
                data,
                clip_even_odd: even_odd,
                ..Path::default()
            }],
        })?;
        Ok(id)
    }

    /// A rectangle far larger than the canvas: "everything", to cut a hole out of.
    pub fn everything(&self) -> Figure {
        let (x, y, w, h) = self.canvas;
        let big = if h > w { h } else { w };
        let margin = 4.0 * if 1.0 > big { 1.0 } else { big } + 100_000.0;
        geom::rect(x - margin, y - margin, x + w + margin, y + h + margin)
    }

    /// Combines device-space figures with the clip; no figures with `RGN_COPY` resets it.
    /// `false` for a combination not drawn (OR, XOR).
    pub fn set_clip(&mut self, figures: Option<Vec<Figure>>, mode: u32, even_odd: bool) -> Result<bool, Halt> {
        self.flush()?;
        let current = self.dc.clip;
        let figures = figures.filter(|f| !f.is_empty());
        if mode == RGN_COPY {
            self.dc.clip = match figures {
                None => None,
                Some(figures) => Some(self.clip_of(PathData::new(figures), even_odd, None)?),
            };
            return Ok(true);
        }
        let Some(figures) = figures else {
            if mode == RGN_AND {
                let empty = Figure {
                    closed: true,
                    ..Figure::at(0.0, 0.0)
                };
                self.dc.clip = Some(self.clip_of(PathData::new(vec![empty]), false, None)?);
            }
            return Ok(mode == RGN_AND || mode == RGN_DIFF || mode == RGN_OR);
        };
        match mode {
            RGN_AND => {
                self.dc.clip = Some(self.clip_of(PathData::new(figures), even_odd, current)?);
                Ok(true)
            }
            RGN_DIFF => {
                let mut all = vec![self.everything()];
                all.extend(figures);
                self.dc.clip = Some(self.clip_of(PathData::new(all), true, current)?);
                Ok(true)
            }
            RGN_OR if current.is_none() => Ok(true),
            _ => Ok(false),
        }
    }

    pub fn clip_rect(&mut self, l: f64, t: f64, r: f64, b: f64, exclude: bool) -> Result<(), Halt> {
        let fig = geom::rect(l, t, r, b).mapped(&self.matrix());
        self.set_clip(Some(vec![fig]), if exclude { RGN_DIFF } else { RGN_AND }, false)?;
        Ok(())
    }
}
