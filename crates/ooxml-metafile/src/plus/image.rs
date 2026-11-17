//! Owns: EMF+ images -- bitmaps (PNG, JPEG, GIF and BMP passed through or decoded; raw pixel
//! formats to PNG, with a transparent column and row after them for what Windows samples
//! past the image), metafiles drawn by the same players (nested at most twice), and drawing
//! one through a source rectangle mapped onto a parallelogram.
//!
//! Windows' slide show centres source pixel i on i, not i + 1/2, and samples transparency
//! past the image: the bitmap is drawn half a pixel up and left, padded, clipped.
//!
//! Must not: decode a DIB itself (`dib` does).

use super::objects::Object;
use super::reader::Data;
use super::{Plus, U_DISPLAY, U_PIXEL, U_WORLD};
use crate::bytes::Bytes;
use crate::error::{Played, RecordError};
use crate::geom;
use crate::player::transform_of;
use crate::pymath::round_even;
use crate::{dib, png, Drawing};
use ooxml_scene::{Bounds, Group, GroupTransform, ImageData, Matrix, Node, Path, PathData, ResourceKind, Viewport};
use std::sync::Arc;

/// What an image holds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ImageKind {
    /// A bitmap of `mime`, and its padded PNG where one could be made.
    Bitmap {
        data: Vec<u8>,
        mime: String,
        padded: Option<Vec<u8>>,
    },
    /// A metafile this player drew.
    Svg(Box<Drawing>),
}

/// An image object.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Image {
    /// Tells two images apart (the drawing keeps one texture pattern per image).
    pub serial: u64,
    pub kind: ImageKind,
    pub width: f64,
    pub height: f64,
}

/// `EmfPlusImage`: `None` for one that cannot be drawn.
pub(crate) fn parse_image(pp: &mut Plus<'_, '_>, d: &mut Data<'_>) -> Result<Option<Arc<Image>>, RecordError> {
    let [_version, kind] = d.u32s::<2>()?;
    let made = match kind {
        1 => {
            let [w, h, stride, fmt, data_type] = d.u32s::<5>()?;
            let (w, h, stride) = (w as i32, h as i32, stride as i32);
            if w <= 0 || h <= 0 {
                return Ok(None);
            }
            pp.p.pixels += i64::from(w) * i64::from(h);
            if pp.p.pixels > i64::try_from(pp.p.limits.max_pixels).unwrap_or(i64::MAX) {
                pp.p.skip("bitmaps beyond the pixel cap");
                return Ok(None);
            }
            let payload = d.b.get(d.at..).unwrap_or(&[]);
            if data_type == 1 {
                compressed(pp, payload, w, h)
            } else {
                pixels(pp, payload, (w, h, stride), fmt)
            }
        }
        2 => {
            let [_mtype, size] = d.u32s::<2>()?;
            let payload = d.bytes((size as usize).min(d.left()))?;
            metafile_image(pp, payload)
        }
        _ => None,
    };
    Ok(made.map(|(kind, width, height)| {
        Arc::new(Image {
            serial: pp.p.serial(),
            kind,
            width,
            height,
        })
    }))
}

type Made = Option<(ImageKind, f64, f64)>;

fn bitmap(data: Vec<u8>, mime: &str, w: i32, h: i32, padded: Option<Vec<u8>>) -> Made {
    Some((
        ImageKind::Bitmap {
            data,
            mime: mime.to_owned(),
            padded,
        },
        f64::from(w),
        f64::from(h),
    ))
}

fn compressed(pp: &mut Plus<'_, '_>, payload: &[u8], w: i32, h: i32) -> Made {
    for (magic, mime) in [
        (&b"\x89PNG\r\n\x1a\n"[..], "image/png"),
        (b"\xff\xd8", "image/jpeg"),
        (b"GIF8", "image/gif"),
    ] {
        if payload.starts_with(magic) {
            return bitmap(payload.to_vec(), mime, w, h, pad_png(payload));
        }
    }
    if payload.starts_with(b"BM") && payload.len() > 14 {
        if let Some(image) =
            dib::split_packed(payload.get(14..).unwrap_or(&[])).and_then(|(hd, bits)| dib::to_image(hd, bits))
        {
            let padded = pad_png(&image.data);
            return bitmap(image.data, image.mime, w, h, padded);
        }
    }
    pp.p.skip("EmfPlus compressed bitmaps other than PNG, JPEG, GIF or BMP (not drawn)");
    None
}

/// A pixel format's kind and bits a pixel.
fn format(fmt: u32) -> Option<(&'static str, usize)> {
    Some(match fmt {
        0x0003_0101 => ("i1", 1),
        0x0003_0402 => ("i4", 4),
        0x0003_0803 => ("i8", 8),
        0x0010_1004 => ("g16", 16),
        0x0002_1005 => ("555", 16),
        0x0002_1006 => ("565", 16),
        0x0006_1007 => ("1555", 16),
        0x0002_1808 => ("24", 24),
        0x0002_2009 => ("32", 32),
        0x0026_200A => ("argb", 32),
        0x000E_200B => ("pargb", 32),
        _ => return None,
    })
}

fn pixels(pp: &mut Plus<'_, '_>, payload: &[u8], size: (i32, i32, i32), fmt: u32) -> Made {
    let (w, h, stride) = size;
    let Some((kind, bits)) = format(fmt) else {
        pp.p.skip(format!("EmfPlus bitmaps in pixel format 0x{fmt:08X} (not drawn)"));
        return None;
    };
    let b = Bytes(payload);
    let mut palette: Vec<[u8; 4]> = Vec::new();
    let mut at = 0usize;
    if kind.starts_with('i') {
        let n = b.u32(4).ok()? as usize;
        b.u32(0).ok()?;
        let n = n.min(256).min(payload.len().saturating_sub(8) / 4);
        palette = (0..n)
            .filter_map(|i| b.u32(8 + 4 * i).ok())
            .map(|v| [(v >> 16) as u8, (v >> 8) as u8, v as u8, (v >> 24) as u8])
            .collect();
        at = 8 + 4 * n;
    }
    let (wu, hu) = (w as usize, h as usize);
    let row_bytes = (wu * bits).div_ceil(8);
    let step = if stride != 0 {
        stride.unsigned_abs() as usize
    } else {
        (row_bytes + 3) & !3
    };
    if step < row_bytes || at + step * (hu - 1) + row_bytes > payload.len() {
        pp.p.skip("an EmfPlus bitmap shorter than its size (not drawn)");
        return None;
    }
    let mut rgba = vec![0u8; wu * hu * 4];
    let mut alpha = false;
    for y in 0..hu {
        let source = if stride >= 0 { y } else { hu - 1 - y };
        let row = payload
            .get(at + source * step..at + source * step + row_bytes)
            .unwrap_or(&[]);
        for x in 0..wu {
            let px = pixel(kind, bits, row, x, &palette);
            alpha |= px[3] != 255;
            if let Some(slot) = rgba.get_mut((y * wu + x) * 4..(y * wu + x) * 4 + 4) {
                slot.copy_from_slice(&px);
            }
        }
    }
    let png = png::encode(w as u32, h as u32, &rgba, alpha);
    bitmap(png, "image/png", w, h, Some(padded(wu, hu, &rgba)))
}

fn pixel(kind: &str, bits: usize, row: &[u8], x: usize, palette: &[[u8; 4]]) -> [u8; 4] {
    let at = |i: usize| row.get(i).copied().unwrap_or(0);
    match kind {
        "argb" | "pargb" | "32" => {
            let (b, g, r, a) = (at(4 * x), at(4 * x + 1), at(4 * x + 2), at(4 * x + 3));
            if kind == "32" {
                return [r, g, b, 255];
            }
            if kind == "pargb" && a != 0 {
                let un = |v: u8| round_even(f64::from(v) * 255.0 / f64::from(a)).min(255.0) as u8;
                return [un(r), un(g), un(b), a];
            }
            [r, g, b, a]
        }
        "24" => [at(3 * x + 2), at(3 * x + 1), at(3 * x), 255],
        "555" | "565" | "1555" | "g16" => {
            let v = u32::from(at(2 * x)) | (u32::from(at(2 * x + 1)) << 8);
            let scale = |field: u32, max: u32| (field * 255 / max) as u8;
            match kind {
                "565" => [scale(v >> 11, 31), scale((v >> 5) & 63, 63), scale(v & 31, 31), 255],
                "g16" => {
                    let g = (v >> 8) as u8;
                    [g, g, g, 255]
                }
                _ => {
                    let a = if kind == "555" || v & 0x8000 != 0 { 255 } else { 0 };
                    [
                        scale((v >> 10) & 31, 31),
                        scale((v >> 5) & 31, 31),
                        scale(v & 31, 31),
                        a,
                    ]
                }
            }
        }
        _ => {
            let index = if bits == 8 {
                usize::from(at(x))
            } else {
                let per = 8 / bits;
                let byte = at(x / per);
                let shift = 8 - bits * (x % per + 1);
                usize::from((byte >> shift) & ((1u8 << bits) - 1))
            };
            palette.get(index).copied().unwrap_or([0, 0, 0, 255])
        }
    }
}

/// A PNG of the pixels with a transparent column and row after them.
fn padded(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
    let row = 4 * width;
    let mut out = Vec::with_capacity((width + 1) * (height + 1) * 4);
    for y in 0..height {
        out.extend_from_slice(rgba.get(y * row..(y + 1) * row).unwrap_or(&[]));
        out.extend_from_slice(&[0; 4]);
    }
    out.extend(std::iter::repeat_n(0u8, 4 * (width + 1)));
    png::encode(width as u32 + 1, height as u32 + 1, &out, true)
}

/// [`padded`] of an 8-bit RGB or RGBA PNG, `None` for any other.
fn pad_png(data: &[u8]) -> Option<Vec<u8>> {
    let (w, h, rgba) = png::decode_rgba(data, 4_000_000)?;
    Some(padded(w as usize, h as usize, &rgba))
}

fn metafile_image(pp: &mut Plus<'_, '_>, payload: &[u8]) -> Made {
    if pp.pp.nesting >= pp.p.limits.max_nesting {
        pp.p.skip("EmfPlus metafile images nested too deep (not drawn)");
        return None;
    }
    let drawing = crate::draw_nested(payload, pp.p.faces, pp.p.limits, pp.pp.nesting + 1).filter(|d| d.drawn > 0);
    let Some(drawing) = drawing else {
        pp.p.skip("an EmfPlus metafile image that draws nothing");
        return None;
    };
    for (name, count) in &drawing.skipped {
        for _ in 0..*count {
            pp.p.skip(name.clone());
        }
    }
    let (w, h) = (drawing.width, drawing.height);
    Some((ImageKind::Svg(Box::new(drawing)), w, h))
}

/// An image drawn at `x, y, width, height` (nearest-neighbour with interpolation mode 5);
/// `padded` draws a bitmap's padded PNG where it has one.
pub(crate) fn image_markup(image: &Image, at: (f64, f64, f64, f64), interpolation: u32, padded: bool) -> Node {
    let (x, y, w, h) = at;
    let (data, mime) = match &image.kind {
        ImageKind::Svg(drawing) => {
            let root = drawing.scene.root.view_box;
            return Node::Viewport(Viewport {
                x: Some(x),
                y: Some(y),
                width: w,
                height: h,
                view_box: Bounds { ..root },
                clipped: true,
                content: drawing.scene.content.clone(),
            });
        }
        ImageKind::Bitmap { padded: Some(p), .. } if padded => (p.clone(), "image/png".to_owned()),
        ImageKind::Bitmap { data, mime, .. } => (data.clone(), mime.clone()),
    };
    Node::Image(ooxml_scene::Image {
        x: Some(x),
        y: Some(y),
        width: w,
        height: h,
        stretch: true,
        pixelated: interpolation == 5,
        data: ImageData { mime, bytes: data },
    })
}

impl Plus<'_, '_> {
    fn src_rect(&self, d: &mut Data<'_>) -> Result<[f64; 4], RecordError> {
        let (_attributes, unit) = (d.u32()?, d.i32()?);
        let src = d.floats_n::<4>()?;
        let unit = u32::try_from(unit).unwrap_or(u32::MAX);
        if ![U_PIXEL, U_WORLD, U_DISPLAY].contains(&unit) {
            let k = self.unit(unit, 0);
            return Ok(src.map(|v| v * k));
        }
        Ok(src)
    }

    pub(super) fn image_record(&mut self, points: bool, flags: u16, d: &mut Data<'_>) -> Played {
        let image = match self.pp.objects.get(&u32::from(flags & 0xFF)) {
            Some(Object::Image(i)) => i.clone(),
            _ => return Ok(false),
        };
        let src = self.src_rect(d)?;
        let (compressed, relative) = (flags & 0x4000 != 0, flags & 0x0800 != 0);
        let local = if points {
            let n = d.count(if relative {
                1
            } else if compressed {
                4
            } else {
                8
            })?;
            let pts = d.points(n, compressed, relative)?;
            let (Some(&(ux, uy)), Some(&(rx, ry)), Some(&(lx, ly))) = (pts.first(), pts.get(1), pts.get(2)) else {
                return Ok(false);
            };
            Matrix::new(rx - ux, ry - uy, lx - ux, ly - uy, ux, uy)
        } else {
            let [x, y, w, h] = d.rects(1, compressed)?.first().copied().unwrap_or_default();
            Matrix::new(w, 0.0, 0.0, h, x, y)
        };
        self.draw_image(&image, src, local)?;
        Ok(true)
    }

    /// `image`'s source rectangle (pixels) drawn through `local` (the unit square of the
    /// source onto world space) and the current transform.
    fn draw_image(&mut self, image: &Image, src: [f64; 4], local: Matrix) -> Result<(), RecordError> {
        let [sx, sy, sw, sh] = src;
        if sw == 0.0 || sh == 0.0 {
            return Ok(());
        }
        let m = local.then(&self.matrix());
        let to_unit = Matrix::new(1.0 / sw, 0.0, 0.0, 1.0 / sh, -sx / sw, -sy / sh);
        let whole = to_unit.then(&m);
        let interpolation = self.pp.state.interpolation;
        let bitmap = matches!(image.kind, ImageKind::Bitmap { .. });
        let mut inner = if let ImageKind::Bitmap { padded, .. } = &image.kind {
            let pad = if padded.is_some() { 1.0 } else { 0.0 };
            image_markup(
                image,
                (-0.5, -0.5, image.width + pad, image.height + pad),
                interpolation,
                true,
            )
        } else {
            image_markup(image, (0.0, 0.0, image.width, image.height), interpolation, false)
        };
        if bitmap || (sx, sy, sw, sh) != (0.0, 0.0, image.width, image.height) {
            let id = self.p.new_id(ResourceKind::Clip);
            self.p.define(ooxml_scene::Def::ClipPath {
                id,
                parent: None,
                paths: vec![Path {
                    data: PathData::new(vec![geom::rect(sx, sy, sx + sw, sy + sh)]),
                    ..Path::default()
                }],
            })?;
            inner = Node::Group(Group {
                clip: Some(id),
                children: vec![inner],
                ..Group::default()
            });
        }
        self.write(Node::Group(Group {
            transform: transform_of(&whole).map(GroupTransform::Matrix),
            children: vec![inner],
            ..Group::default()
        }))?;
        Ok(())
    }
}
