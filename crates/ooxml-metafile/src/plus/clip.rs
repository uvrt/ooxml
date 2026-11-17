//! Owns: the EMF+ clip -- `Replace`, `Intersect`, `Union`, `Xor`, `Exclude` and
//! `Complement` of device-space figures with the clip in force (as clip paths chained or
//! unioned; a combination of an unknown clip that is not one plain union is named as not
//! drawn), `ResetClip` and `OffsetClip`.
//!
//! Must not: draw.

use super::reader::Data;
use super::{Clip, Plus};
use crate::error::{Halt, Played};
use ooxml_scene::{Def, Figure, Matrix, Path, PathData, ResourceKind};

const C_REPLACE: u32 = 0;
const C_INTERSECT: u32 = 1;
const C_UNION: u32 = 2;
const C_XOR: u32 = 3;
const C_EXCLUDE: u32 = 4;
const C_COMPLEMENT: u32 = 5;

type Parts = Vec<(Vec<Figure>, bool)>;

impl Plus<'_, '_> {
    fn everything(&self) -> Figure {
        self.p.everything()
    }

    /// A clip path of the union of `parts` (device figures and their rule), inside `parent`.
    fn clip_of(&mut self, parts: Parts, parent: Option<&Clip>) -> Result<Clip, Halt> {
        let id = self.p.new_id(ResourceKind::Clip);
        let mut paths: Vec<Path> = parts
            .iter()
            .map(|(figs, even_odd)| Path {
                data: if figs.is_empty() {
                    PathData::new(vec![empty_figure()])
                } else {
                    PathData::new(figs.clone())
                },
                clip_even_odd: *even_odd,
                ..Path::default()
            })
            .collect();
        if paths.is_empty() {
            paths.push(Path {
                data: PathData::new(vec![empty_figure()]),
                ..Path::default()
            });
        }
        self.p.define(Def::ClipPath {
            id,
            parent: parent.map(|c| c.id),
            paths,
        })?;
        Ok(Clip {
            id,
            parts: if parent.is_some() { None } else { Some(parts) },
        })
    }

    /// Combines the clip with device-space figures; `false` for a combination not drawn.
    pub fn combine(&mut self, figures: Vec<Figure>, mode: u32, even_odd: bool) -> Result<bool, Halt> {
        let current = self.pp.state.clip.clone();
        let outside = |me: &Self, figs: Vec<Figure>| -> (Vec<Figure>, bool) {
            let mut all = vec![me.everything()];
            all.extend(figs);
            (all, true)
        };
        let new = (figures, even_odd);
        let clip = match mode {
            C_REPLACE => {
                let base = self.pp.state.base_clip.clone();
                self.clip_of(vec![new], base.as_ref())?
            }
            C_INTERSECT => self.clip_of(vec![new], current.as_ref())?,
            C_EXCLUDE => {
                let part = outside(self, new.0);
                self.clip_of(vec![part], current.as_ref())?
            }
            C_UNION => match current {
                None => return Ok(true),
                Some(Clip {
                    parts: Some(mut parts), ..
                }) => {
                    parts.push(new);
                    self.clip_of(parts, None)?
                }
                Some(_) => return Ok(false),
            },
            C_XOR => match single(&current) {
                Some(None) => {
                    let part = outside(self, new.0);
                    self.clip_of(vec![part], None)?
                }
                Some(Some(mut figs)) => {
                    figs.extend(new.0);
                    self.clip_of(vec![(figs, true)], None)?
                }
                None => return Ok(false),
            },
            C_COMPLEMENT => match single(&current) {
                Some(None) => self.clip_of(vec![(Vec::new(), false)], None)?,
                Some(Some(figs)) => {
                    let part = outside(self, figs);
                    let outer = self.clip_of(vec![part], None)?;
                    self.clip_of(vec![new], Some(&outer))?
                }
                None => return Ok(false),
            },
            _ => return Ok(false),
        };
        self.pp.state.clip = Some(clip);
        Ok(true)
    }

    pub(super) fn offset_clip(&mut self, d: &mut Data<'_>) -> Played {
        let [dx, dy] = d.floats_n::<2>()?;
        let Some(clip) = self.pp.state.clip.clone() else {
            return Ok(true);
        };
        let Some(parts) = clip.parts else {
            return Ok(false);
        };
        let m = self.matrix();
        let (ox, oy) = m.apply(dx, dy);
        let (zx, zy) = m.apply(0.0, 0.0);
        let shift = Matrix::translate(ox - zx, oy - zy);
        let parts = parts
            .into_iter()
            .map(|(figs, eo)| (figs.iter().map(|f| f.mapped(&shift)).collect(), eo))
            .collect();
        self.pp.state.clip = Some(self.clip_of(parts, None)?);
        Ok(true)
    }
}

/// `M0 0Z`: a clip of nothing.
fn empty_figure() -> Figure {
    Figure {
        closed: true,
        ..Figure::at(0.0, 0.0)
    }
}

/// For no clip `Some(None)`; for a clip of one plain part `Some(Some(its figures))`; else
/// `None`.
fn single(current: &Option<Clip>) -> Option<Option<Vec<Figure>>> {
    match current {
        None => Some(None),
        Some(Clip { parts: Some(parts), .. }) if parts.len() == 1 => parts.first().map(|(f, _)| Some(f.clone())),
        Some(_) => None,
    }
}
