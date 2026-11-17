//! Owns: EMF's object records (pens, extended pens, brushes, pattern and monochrome brushes,
//! fonts, the slots of palettes and colour spaces; selection and deletion) and its bitmap,
//! gradient and text records.
//!
//! Must not: decode a bitmap itself (`dib` does).

use super::Reader;
use crate::bytes::{cp1252, strip, utf16le, Bytes};
use crate::dc::{self, colorref, Brush, BrushImage, Font, Object, Pen};
use crate::dib;
use crate::error::{Played, ReadError};
use crate::player::image::{Blend, Blit};
use crate::player::text::TextCall;
use ooxml_scene::Rgb;
use std::sync::Arc;

impl Reader<'_> {
    pub(super) fn select_or_delete(&mut self, rec: Bytes<'_>, delete: bool) -> Played {
        let index = rec.u32(8)?;
        if delete {
            self.p.delete(index);
        } else {
            self.p.select(index)?;
        }
        Ok(true)
    }

    pub(super) fn placeholder(&mut self, rec: Bytes<'_>) -> Played {
        let index = rec.u32(8)?;
        rec.u32(0)?;
        self.p.create(index, Object::Placeholder);
        Ok(true)
    }

    pub(super) fn create_pen(&mut self, rec: Bytes<'_>) -> Played {
        let (index, style, width, _wy, color) = (rec.u32(8)?, rec.u32(12)?, rec.i32(16)?, rec.i32(20)?, rec.u32(24)?);
        let width = i64::from(width).abs();
        let pen = Pen {
            style,
            width: width as f64,
            color: colorref(color),
            geometric: width > 1 || style & dc::PS_GEOMETRIC != 0,
            plain: true,
            user_style: Vec::new(),
            null: style & 0x0F == dc::PS_NULL,
        };
        self.p.create(index, Object::Pen(pen));
        Ok(true)
    }

    pub(super) fn ext_pen(&mut self, rec: Bytes<'_>) -> Played {
        let [index, ..] = rec.u32s::<5>(8)?;
        let [style, width, brush_style, color, _hatch, entries] = rec.u32s::<6>(28)?;
        let entries = (entries as usize).min(rec.len().saturating_sub(52) / 4).min(64);
        let user_style = rec.many(52, entries, 4, Bytes::u32)?;
        let geometric = style & dc::PS_GEOMETRIC != 0;
        let pen = Pen {
            style,
            width: if geometric { f64::from(width) } else { 0.0 },
            color: colorref(color),
            geometric,
            plain: false,
            user_style,
            null: style & 0x0F == dc::PS_NULL || brush_style == 1,
        };
        self.p.create(index, Object::Pen(pen));
        Ok(true)
    }

    pub(super) fn create_brush(&mut self, rec: Bytes<'_>) -> Played {
        let (index, style, color, hatch) = (rec.u32(8)?, rec.u32(12)?, rec.u32(16)?, rec.u32(20)?);
        let (brush, known) = match style {
            0 => (Brush::Solid(colorref(color)), true),
            1 => (Brush::Null, true),
            2 => (
                Brush::Hatch {
                    color: colorref(color),
                    hatch,
                },
                true,
            ),
            _ => (Brush::Solid(colorref(color)), false),
        };
        self.p.create(index, Object::Brush(brush));
        Ok(known)
    }

    fn brush_image(&mut self, rec: Bytes<'_>, palette: Option<[u8; 8]>) -> Result<(u32, Option<Brush>), ReadError> {
        let [index, _usage, off_bmi, cb_bmi, off_bits, cb_bits] = rec.u32s::<6>(8)?.map(|v| v as usize);
        let index = index as u32;
        let mut header = rec.slice(off_bmi, off_bmi.saturating_add(cb_bmi)).to_vec();
        let bits = rec.slice(off_bits, off_bits.saturating_add(cb_bits));
        if let Some(palette) = palette.filter(|_| header.len() >= 48) {
            header.splice(40..48, palette);
        }
        let image = dib::to_image(&header, bits).filter(|i| i.mime == "image/png");
        let Some(image) = image else {
            return Ok((index, None));
        };
        let head = Bytes(&header);
        let (w, h) = (head.i32(4)?, head.i32(8)?);
        let brush = Brush::Pattern(Arc::new(BrushImage {
            serial: self.p.serial(),
            png: image.data,
            width: i64::from(w).abs(),
            height: i64::from(h).abs(),
        }));
        Ok((index, Some(brush)))
    }

    pub(super) fn dib_brush(&mut self, rec: Bytes<'_>) -> Played {
        let (index, brush) = self.brush_image(rec, None)?;
        let found = brush.is_some();
        self.p.create(
            index,
            Object::Brush(brush.unwrap_or(Brush::Solid(Rgb(0x80, 0x80, 0x80)))),
        );
        Ok(found)
    }

    /// A monochrome pattern: its 0 bits in the text colour, 1 bits in the background's (as
    /// they stand when the brush is created).
    pub(super) fn mono_brush(&mut self, rec: Bytes<'_>) -> Played {
        let quad = |c: Rgb| [c.2, c.1, c.0, 0];
        let (t, b) = (quad(self.p.dc.text_color), quad(self.p.dc.bk_color));
        let palette = [t[0], t[1], t[2], t[3], b[0], b[1], b[2], b[3]];
        let (index, brush) = self.brush_image(rec, Some(palette))?;
        let found = brush.is_some();
        self.p.create(
            index,
            Object::Brush(brush.unwrap_or(Brush::Solid(Rgb(0x80, 0x80, 0x80)))),
        );
        Ok(found)
    }

    pub(super) fn create_font(&mut self, rec: Bytes<'_>) -> Played {
        let index = rec.u32(8)?;
        let [height, _width, escapement, _orientation, weight] = rec.i32s::<5>(12)?;
        let flags = rec.many(32, 8, 1, Bytes::u8)?;
        let face = utf16le(rec.slice(40, 104));
        let face = face.split('\0').next().unwrap_or("");
        let flag = |i: usize| flags.get(i).copied().unwrap_or(0);
        let font = Font {
            height: f64::from(height),
            escapement: f64::from(escapement),
            weight: i64::from(weight),
            italic: flag(0) != 0,
            underline: flag(1) != 0,
            strike: flag(2) != 0,
            face: strip(face).to_owned(),
            pitch_family: flag(7),
        };
        self.p.create(index, Object::Font(font));
        Ok(true)
    }

    fn bits<'r>(rec: Bytes<'r>, v: [usize; 4]) -> (&'r [u8], &'r [u8]) {
        let [off_bmi, cb_bmi, off_bits, cb_bits] = v;
        let len = rec.len();
        if cb_bmi == 0 || off_bmi.saturating_add(cb_bmi) > len || off_bits.saturating_add(cb_bits) > len {
            return (&[], &[]);
        }
        (
            rec.slice(off_bmi, off_bmi + cb_bmi),
            rec.slice(off_bits, off_bits + cb_bits),
        )
    }

    pub(super) fn stretchdibits(&mut self, rec: Bytes<'_>) -> Played {
        let [x, y, xs, ys, cxs, cys] = rec.i32s::<6>(24)?.map(f64::from);
        let [off_bmi, cb_bmi, off_bits, cb_bits, _usage, rop] = rec.u32s::<6>(48)?;
        let [cx, cy] = rec.i32s::<2>(72)?.map(f64::from);
        let (header, bits) = Self::bits(rec, [off_bmi, cb_bmi, off_bits, cb_bits].map(|v| v as usize));
        let blit = Blit {
            dest: (x, y, cx, cy),
            src: (xs, ys, cxs, cys),
        };
        self.p.image(blit, header, bits, rop, Blend::default())?;
        Ok(true)
    }

    pub(super) fn blt(&mut self, kind: u32, rec: Bytes<'_>) -> Played {
        let [x, y, cx, cy] = rec.i32s::<4>(24)?;
        let rop = rec.u32(40)?;
        let [xs, ys] = rec.i32s::<2>(44)?;
        let [_usage, off_bmi, cb_bmi, off_bits, cb_bits] = rec.u32s::<5>(80)?;
        let stretch = kind != 76;
        let [cxs, cys] = if stretch && rec.len() >= 108 {
            rec.i32s::<2>(100)?
        } else {
            [cx, cy]
        };
        let (header, bits) = Self::bits(rec, [off_bmi, cb_bmi, off_bits, cb_bits].map(|v| v as usize));
        let blit = Blit {
            dest: (f64::from(x), f64::from(y), f64::from(cx), f64::from(cy)),
            src: (f64::from(xs), f64::from(ys), f64::from(cxs), f64::from(cys)),
        };
        let (rop, blend) = match kind {
            114 => (
                dc::SRCCOPY,
                Blend {
                    alpha: Some(f64::from((rop >> 16) & 0xFF) / 255.0),
                    transparent: None,
                },
            ),
            116 => (
                dc::SRCCOPY,
                Blend {
                    alpha: None,
                    transparent: Some(rop),
                },
            ),
            _ => (rop, Blend::default()),
        };
        self.p.image(blit, header, bits, rop, blend)?;
        Ok(true)
    }

    pub(super) fn set_dibits(&mut self, rec: Bytes<'_>) -> Played {
        let [x, y, xs, ys, cxs, cys] = rec.i32s::<6>(24)?.map(f64::from);
        let offsets = rec.u32s::<4>(48)?;
        let (header, bits) = Self::bits(rec, offsets.map(|v| v as usize));
        let s = self.p.matrix().mean_scale();
        let scale = 1.0 / if s != 0.0 { s } else { 1.0 };
        let blit = Blit {
            dest: (x, y, cxs * scale, cys * scale),
            src: (xs, ys, cxs, cys),
        };
        self.p.image(blit, header, bits, dc::SRCCOPY, Blend::default())?;
        Ok(true)
    }

    pub(super) fn ext_text_out(&mut self, rec: Bytes<'_>, wide: bool) -> Played {
        let [x, y] = rec.i32s::<2>(36)?;
        let [count, off_string, options] = rec.u32s::<3>(44)?;
        let [l, t, r, b] = Self::box_at(rec, 56)?;
        let off_dx = rec.u32(72)? as usize;
        let count = (count as usize).min(self.p.limits.max_text_chars);
        if options & dc::ETO_GLYPH_INDEX != 0 {
            self.p.skip("EMR_EXTTEXTOUTW by glyph index (not drawn)");
            return Ok(true);
        }
        let off_string = off_string as usize;
        let raw = rec.slice(off_string, off_string.saturating_add(count * if wide { 2 } else { 1 }));
        let text = if wide { utf16le(raw) } else { cp1252(raw) };
        let mut dx = None;
        let pdy = options & dc::ETO_PDY != 0;
        if off_dx != 0 && off_dx < rec.len() {
            let n = (count * if pdy { 2 } else { 1 }).min((rec.len() - off_dx) / 4);
            let values = rec.many(off_dx, n, 4, Bytes::i32)?;
            let values: Vec<i64> = if pdy {
                values.iter().step_by(2).map(|&v| i64::from(v)).collect()
            } else {
                values.iter().map(|&v| i64::from(v)).collect()
            };
            dx = (values.len() >= count).then_some(values);
        }
        self.p.text(TextCall {
            x: f64::from(x),
            y: f64::from(y),
            text: &text,
            dx,
            options,
            rect: Some((l, t, r, b)),
        })?;
        Ok(true)
    }

    pub(super) fn gradient_fill(&mut self, rec: Bytes<'_>) -> Played {
        let (n_ver, n_tri, mode) = (rec.u32(24)? as usize, rec.u32(28)? as usize, rec.u32(32)?);
        let n_ver = n_ver.min(rec.len().saturating_sub(36) / 16);
        self.p.count_points(n_ver as i64)?;
        let mut vertices = Vec::with_capacity(n_ver);
        for k in 0..n_ver {
            let at = 36 + 16 * k;
            let (x, y) = (rec.i32(at)?, rec.i32(at + 4)?);
            let (r, g, b) = (rec.u16(at + 8)?, rec.u16(at + 10)?, rec.u16(at + 12)?);
            rec.u16(at + 14)?;
            vertices.push((
                f64::from(x),
                f64::from(y),
                Rgb((r >> 8) as u8, (g >> 8) as u8, (b >> 8) as u8),
            ));
        }
        let at = 36 + 16 * n_ver;
        match mode {
            0 | 1 => {
                let n = n_tri.min(rec.len().saturating_sub(at) / 8);
                for k in 0..n {
                    let (i, j) = (rec.u32(at + 8 * k)? as usize, rec.u32(at + 8 * k + 4)? as usize);
                    let (Some(&(x0, y0, c0)), Some(&(x1, y1, c1))) = (vertices.get(i), vertices.get(j)) else {
                        continue;
                    };
                    let forward = if mode == 0 { x0 <= x1 } else { y0 <= y1 };
                    let (a, b) = if forward { (c0, c1) } else { (c1, c0) };
                    let box_ = (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
                    self.p.gradient_rect(box_, a, b, mode == 1)?;
                }
                Ok(true)
            }
            2 => {
                let n = n_tri.min(rec.len().saturating_sub(at) / 12);
                for k in 0..n {
                    let o = at + 12 * k;
                    let ids = [rec.u32(o)? as usize, rec.u32(o + 4)? as usize, rec.u32(o + 8)? as usize];
                    let corners: Vec<_> = ids.iter().filter_map(|&i| vertices.get(i).copied()).collect();
                    if corners.len() < 3 {
                        continue;
                    }
                    let mean =
                        |ch: fn(&Rgb) -> u8| (corners.iter().map(|c| u32::from(ch(&c.2))).sum::<u32>() / 3) as u8;
                    let fill = Rgb(mean(|c| c.0), mean(|c| c.1), mean(|c| c.2));
                    let points: Vec<_> = corners.iter().map(|c| (c.0, c.1)).collect();
                    self.p.flat_polygon(&points, fill)?;
                }
                self.p.skip("EMR_GRADIENTFILL triangles (drawn in their mean colour)");
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}
