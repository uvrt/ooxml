//! Owns: EMF's record names and the dispatch of each record type to its handler, and the
//! handlers for state: mapping, transforms, modes, colours, the current position, saved
//! states, paths and clipping.
//!
//! Must not: draw shapes itself (`shapes`), or create objects (`objects`).

use super::Reader;
use crate::bytes::Bytes;
use crate::dc::{self, colorref};
use crate::error::{Played, ReadError};
use crate::geom;
use ooxml_scene::Matrix;

/// MS-EMF's record names by type (empty where a type has none).
const NAMES: [&str; 123] = [
    "",
    "HEADER",
    "POLYBEZIER",
    "POLYGON",
    "POLYLINE",
    "POLYBEZIERTO",
    "POLYLINETO",
    "POLYPOLYLINE",
    "POLYPOLYGON",
    "SETWINDOWEXTEX",
    "SETWINDOWORGEX",
    "SETVIEWPORTEXTEX",
    "SETVIEWPORTORGEX",
    "SETBRUSHORGEX",
    "EOF",
    "SETPIXELV",
    "SETMAPPERFLAGS",
    "SETMAPMODE",
    "SETBKMODE",
    "SETPOLYFILLMODE",
    "SETROP2",
    "SETSTRETCHBLTMODE",
    "SETTEXTALIGN",
    "SETCOLORADJUSTMENT",
    "SETTEXTCOLOR",
    "SETBKCOLOR",
    "OFFSETCLIPRGN",
    "MOVETOEX",
    "SETMETARGN",
    "EXCLUDECLIPRECT",
    "INTERSECTCLIPRECT",
    "SCALEVIEWPORTEXTEX",
    "SCALEWINDOWEXTEX",
    "SAVEDC",
    "RESTOREDC",
    "SETWORLDTRANSFORM",
    "MODIFYWORLDTRANSFORM",
    "SELECTOBJECT",
    "CREATEPEN",
    "CREATEBRUSHINDIRECT",
    "DELETEOBJECT",
    "ANGLEARC",
    "ELLIPSE",
    "RECTANGLE",
    "ROUNDRECT",
    "ARC",
    "CHORD",
    "PIE",
    "SELECTPALETTE",
    "CREATEPALETTE",
    "SETPALETTEENTRIES",
    "RESIZEPALETTE",
    "REALIZEPALETTE",
    "EXTFLOODFILL",
    "LINETO",
    "ARCTO",
    "POLYDRAW",
    "SETARCDIRECTION",
    "SETMITERLIMIT",
    "BEGINPATH",
    "ENDPATH",
    "CLOSEFIGURE",
    "FILLPATH",
    "STROKEANDFILLPATH",
    "STROKEPATH",
    "FLATTENPATH",
    "WIDENPATH",
    "SELECTCLIPPATH",
    "ABORTPATH",
    "",
    "COMMENT",
    "FILLRGN",
    "FRAMERGN",
    "INVERTRGN",
    "PAINTRGN",
    "EXTSELECTCLIPRGN",
    "BITBLT",
    "STRETCHBLT",
    "MASKBLT",
    "PLGBLT",
    "SETDIBITSTODEVICE",
    "STRETCHDIBITS",
    "EXTCREATEFONTINDIRECTW",
    "EXTTEXTOUTA",
    "EXTTEXTOUTW",
    "POLYBEZIER16",
    "POLYGON16",
    "POLYLINE16",
    "POLYBEZIERTO16",
    "POLYLINETO16",
    "POLYPOLYLINE16",
    "POLYPOLYGON16",
    "POLYDRAW16",
    "CREATEMONOBRUSH",
    "CREATEDIBPATTERNBRUSHPT",
    "EXTCREATEPEN",
    "POLYTEXTOUTA",
    "POLYTEXTOUTW",
    "SETICMMODE",
    "CREATECOLORSPACE",
    "SETCOLORSPACE",
    "DELETECOLORSPACE",
    "GLSRECORD",
    "GLSBOUNDEDRECORD",
    "PIXELFORMAT",
    "DRAWESCAPE",
    "EXTESCAPE",
    "",
    "SMALLTEXTOUT",
    "FORCEUFIMAPPING",
    "NAMEDESCAPE",
    "COLORCORRECTPALETTE",
    "SETICMPROFILEA",
    "SETICMPROFILEW",
    "ALPHABLEND",
    "SETLAYOUT",
    "TRANSPARENTBLT",
    "",
    "GRADIENTFILL",
    "SETLINKEDUFIS",
    "SETTEXTJUSTIFICATION",
    "COLORMATCHTOTARGETW",
    "CREATECOLORSPACEW",
];

/// A record type's name (MS-EMF), or its number in hexadecimal.
pub fn name(kind: u32) -> String {
    match NAMES.get(kind as usize) {
        Some(n) if !n.is_empty() => (*n).to_owned(),
        _ => format!("0x{kind:X}"),
    }
}

impl Reader<'_> {
    /// The record's handler's result, or `None` when no handler reads its type.
    pub(super) fn handle(&mut self, kind: u32, rec: Bytes<'_>) -> Option<Played> {
        Some(match kind {
            2 | 85 => self.polybezier(rec),
            3 | 86 => self.polygon(rec),
            4 | 87 => self.polyline(rec),
            5 | 88 => self.poly_to(rec, true),
            6 | 89 => self.poly_to(rec, false),
            7 | 90 => self.polypolyline(rec),
            8 | 91 => self.polypolygon(rec),
            56 | 92 => self.polydraw(rec),
            9 | 11 => self.extent(rec, kind == 11),
            10 | 12 => self.origin(rec, kind == 12),
            31 | 32 => self.scale_extent(rec, kind == 31),
            15 => self.set_pixel(rec),
            17 => self.map_mode(rec),
            18..=20 | 22 | 24 | 25 | 57 | 58 | 115 => self.mode(kind, rec),
            27 | 54 => self.move_or_line(rec, kind == 54),
            29 | 30 => self.clip_rect(rec, kind == 29),
            33 => {
                self.p.save_dc();
                Ok(true)
            }
            34 => self.restore_dc(rec),
            35 | 36 => self.world(rec, kind == 36),
            37 | 40 => self.select_or_delete(rec, kind == 40),
            38 => self.create_pen(rec),
            39 => self.create_brush(rec),
            41 => self.angle_arc(rec),
            42..=44 => self.box_shape(kind, rec),
            45..=47 | 55 => self.arc(kind, rec),
            49 | 99 | 122 => self.placeholder(rec),
            59..=64 | 66 | 68 => self.path_op(kind),
            67 => self.select_clip_path(rec),
            70 => self.comment(rec),
            71 | 74 => self.region_paint(rec, kind == 71),
            75 => self.ext_select_clip_rgn(rec),
            76 | 77 | 114 | 116 => self.blt(kind, rec),
            80 => self.set_dibits(rec),
            81 => self.stretchdibits(rec),
            82 => self.create_font(rec),
            83 | 84 => self.ext_text_out(rec, kind == 84),
            93 => self.mono_brush(rec),
            94 => self.dib_brush(rec),
            95 => self.ext_pen(rec),
            118 => self.gradient_fill(rec),
            _ => return None,
        })
    }

    fn pair(rec: Bytes<'_>) -> Result<(f64, f64), ReadError> {
        Ok((f64::from(rec.i32(8)?), f64::from(rec.i32(12)?)))
    }

    fn extent(&mut self, rec: Bytes<'_>, viewport: bool) -> Played {
        let (x, y) = Self::pair(rec)?;
        if x != 0.0 && y != 0.0 {
            if viewport {
                self.p.dc.viewport_ext = (x, y);
            } else {
                self.p.dc.window_ext = (x, y);
            }
        }
        Ok(true)
    }

    fn origin(&mut self, rec: Bytes<'_>, viewport: bool) -> Played {
        let xy = Self::pair(rec)?;
        if viewport {
            self.p.dc.viewport_org = xy;
        } else {
            self.p.dc.window_org = xy;
        }
        Ok(true)
    }

    fn scale_extent(&mut self, rec: Bytes<'_>, viewport: bool) -> Played {
        let ext = if viewport {
            self.p.dc.viewport_ext
        } else {
            self.p.dc.window_ext
        };
        let (xn, xd, yn, yd) = (rec.i32(8)?, rec.i32(12)?, rec.i32(16)?, rec.i32(20)?);
        if xd == 0 || yd == 0 || xn == 0 || yn == 0 {
            return Ok(true);
        }
        let scaled = (
            ext.0 * f64::from(xn) / f64::from(xd),
            ext.1 * f64::from(yn) / f64::from(yd),
        );
        if viewport {
            self.p.dc.viewport_ext = scaled;
        } else {
            self.p.dc.window_ext = scaled;
        }
        Ok(true)
    }

    fn map_mode(&mut self, rec: Bytes<'_>) -> Played {
        let mode = rec.u32(8)?;
        if !(1..=8).contains(&mode) {
            return Ok(false);
        }
        self.p.dc.map_mode = mode;
        Ok(true)
    }

    fn world(&mut self, rec: Bytes<'_>, modify: bool) -> Played {
        let v = rec.many(8, 6, 4, Bytes::f32)?;
        let m = Matrix::from_slice(&v);
        if modify {
            let mode = rec.u32(32)?;
            return Ok(self.p.modify_world(m, mode));
        }
        Ok(self.p.set_world(m))
    }

    fn mode(&mut self, kind: u32, rec: Bytes<'_>) -> Played {
        let value = rec.u32(8)?;
        let dc = &mut self.p.dc;
        match kind {
            18 => dc.bk_mode = value,
            19 => dc.fill_mode = if value == 2 { dc::WINDING } else { dc::ALTERNATE },
            20 => {
                self.p.flush()?;
                self.p.dc.rop2 = value;
                if ![1, 11, 13, 16].contains(&value) {
                    self.p.skip(format!("EMR_SETROP2 mode {value} (drawn as R2_COPYPEN)"));
                }
            }
            22 => dc.text_align = value,
            24 => dc.text_color = colorref(value),
            25 => dc.bk_color = colorref(value),
            57 => {
                dc.arc_direction = if value == 2 {
                    dc::AD_CLOCKWISE
                } else {
                    dc::AD_COUNTERCLOCKWISE
                }
            }
            58 => {
                // Some writers store a float.
                let v = if value > 0x3F00_0000 {
                    f64::from(rec.f32(8)?)
                } else {
                    f64::from(value)
                };
                if v.is_finite() && v > 0.0 {
                    dc.miter_limit = v;
                }
            }
            _ => return Ok(value == 0), // SETLAYOUT: only left to right is drawn
        }
        Ok(true)
    }

    fn move_or_line(&mut self, rec: Bytes<'_>, line: bool) -> Played {
        let (x, y) = Self::pair(rec)?;
        if line {
            self.p.line_to(x, y)?;
        } else {
            self.p.move_to(x, y);
        }
        Ok(true)
    }

    fn restore_dc(&mut self, rec: Bytes<'_>) -> Played {
        self.p.restore_dc(i64::from(rec.i32(8)?))?;
        Ok(true)
    }

    fn path_op(&mut self, kind: u32) -> Played {
        match kind {
            59 => self.p.begin_path()?,
            60 => self.p.end_path(),
            61 => self.p.close_figure(),
            62 => self.p.use_path(true, false)?,
            63 => self.p.use_path(true, true)?,
            64 => self.p.use_path(false, true)?,
            66 => self.p.path_widened = true,
            _ => self.p.abort_path(),
        }
        Ok(true)
    }

    fn select_clip_path(&mut self, rec: Bytes<'_>) -> Played {
        let mode = rec.u32(8)?;
        let Some(mut figures) = self.p.path_done.take().filter(|f| !f.is_empty()) else {
            return Ok(true);
        };
        figures.iter_mut().for_each(|f| f.closed = true);
        let even_odd = self.p.dc.fill_mode == dc::ALTERNATE;
        Ok(self.p.set_clip(Some(figures), mode, even_odd)?)
    }

    fn clip_rect(&mut self, rec: Bytes<'_>, exclude: bool) -> Played {
        let [l, t, r, b] = Self::box_at(rec, 8)?;
        self.p.clip_rect(l, t, r, b, exclude)?;
        Ok(true)
    }

    /// The four integers at `at` (left, top, right, bottom).
    pub(super) fn box_at(rec: Bytes<'_>, at: usize) -> Result<[f64; 4], ReadError> {
        Ok([rec.i32(at)?, rec.i32(at + 4)?, rec.i32(at + 8)?, rec.i32(at + 12)?].map(f64::from))
    }

    /// A region's rectangles at `at` (its `RGNDATA`), counted against the point cap.
    pub(super) fn region_rects(
        &mut self,
        rec: Bytes<'_>,
        at: usize,
    ) -> Result<Vec<(i32, i32, i32, i32)>, crate::error::RecordError> {
        if rec.len() < at + 32 {
            return Ok(Vec::new());
        }
        let count = (rec.u32(at + 8)? as usize).min((rec.len() - at - 32) / 16);
        self.p.count_points(4 * count as i64)?;
        (0..count)
            .map(|k| {
                let o = at + 32 + 16 * k;
                Ok((rec.i32(o)?, rec.i32(o + 4)?, rec.i32(o + 8)?, rec.i32(o + 12)?))
            })
            .collect()
    }

    fn ext_select_clip_rgn(&mut self, rec: Bytes<'_>) -> Played {
        let (size, mode) = (rec.u32(8)?, rec.u32(12)?);
        let rects = if size != 0 {
            self.region_rects(rec, 16)?
        } else {
            Vec::new()
        };
        let figures: Vec<_> = rects
            .iter()
            .map(|&(l, t, r, b)| geom::rect(f64::from(l), f64::from(t), f64::from(r), f64::from(b)))
            .collect();
        Ok(self.p.set_clip(Some(figures), mode, false)?)
    }

    fn region_paint(&mut self, rec: Bytes<'_>, fill_rgn: bool) -> Played {
        if fill_rgn {
            let index = rec.u32(28)?;
            let _size = rec.u32(24)?;
            let Some(dc::Object::Brush(brush)) = self.p.object(index) else {
                return Ok(false);
            };
            let rects = self.region_rects(rec, 32)?;
            self.p.device_rects(&rects, &brush)?;
            return Ok(true);
        }
        let rects = self.region_rects(rec, 28)?;
        let brush = self.p.dc.brush.clone();
        self.p.device_rects(&rects, &brush)?;
        Ok(true)
    }
}

trait FromSlice {
    fn from_slice(v: &[f32]) -> Matrix;
}

impl FromSlice for Matrix {
    fn from_slice(v: &[f32]) -> Matrix {
        let g = |i: usize| f64::from(v.get(i).copied().unwrap_or(0.0));
        Matrix::new(g(0), g(1), g(2), g(3), g(4), g(5))
    }
}
