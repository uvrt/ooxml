//! Owns: what GDI draws without a pen -- bitmaps (copied, blended at a constant alpha, or
//! with a transparent colour; cropped by a nested viewport), raster operations that fill,
//! single pixels, region rectangles, gradient rectangles and flat triangles.
//!
//! A 1-bit DIB's palette is used, and an enlarged bitmap is smoothed (the SVG default;
//! Windows' slide show smooths it).
//!
//! Must not: decode a bitmap itself (`dib` does).

use super::paint::path_node;
use super::{transform_of, ClipChoice, Player};
use crate::bytes::Bytes;
use crate::dc::{self, colorref, Brush};
use crate::dib;
use crate::error::Halt;
use crate::geom;
use crate::png;
use ooxml_scene::{
    Bounds, Content, Def, Fill, Group, GroupTransform, Image, ImageData, Matrix, Node, Paint, PathData, Rect,
    ResourceKind, Rgb, Stop, Viewport,
};

/// How a bitmap is drawn beyond its raster operation.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Blend {
    /// A constant opacity (`ALPHABLEND`).
    pub alpha: Option<f64>,
    /// A colour drawn transparent (`TRANSPARENTBLT`), as a `COLORREF`.
    pub transparent: Option<u32>,
}

/// The rectangle a bitmap is drawn to (logical) and from (its pixels; `y` from the bottom
/// for a bottom-up DIB, as GDI counts it).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Blit {
    pub dest: (f64, f64, f64, f64),
    pub src: (f64, f64, f64, f64),
}

impl Player<'_> {
    /// A bitmap from a DIB.
    pub fn image(&mut self, blit: Blit, header: &[u8], bits: &[u8], rop: u32, blend: Blend) -> Result<(), Halt> {
        if !self.visible() {
            return Ok(());
        }
        let (x, y, w, h) = blit.dest;
        if rop == dc::BLACKNESS || rop == dc::WHITENESS {
            self.flush()?;
            let m = self.matrix();
            let fill = if rop == dc::BLACKNESS { dc::BLACK } else { dc::WHITE };
            let data = PathData::new(vec![geom::rect(x, y, x + w, y + h)]);
            let node = path_node(data, transform_of(&m), Some(Fill::of(Paint::Color(fill))), Vec::new());
            return self.write(node, true, ClipChoice::Dc);
        }
        if rop == dc::PATCOPY || header.is_empty() {
            if rop == dc::PATCOPY {
                self.pattern_rect(x, y, w, h)?;
            } else {
                self.skip(format!("raster operation 0x{rop:08X} without a bitmap"));
            }
            return Ok(());
        }
        if !dc::COPY_LIKE.contains(&rop) && blend.alpha.is_none() && blend.transparent.is_none() {
            self.skip(format!("raster operation 0x{rop:08X}"));
            return Ok(());
        }
        let head = Bytes(header);
        if let (Ok(bw), Ok(bh)) = (head.i32(4), head.i32(8)) {
            self.pixels += i64::from(bw).abs() * i64::from(bh).abs();
            if self.pixels > i64::try_from(self.limits.max_pixels).unwrap_or(i64::MAX) {
                self.skip("bitmaps beyond the pixel cap");
                return Ok(());
            }
        }
        let Some((data, mime, bw, bh)) = encoded(header, bits, blend) else {
            self.skip("a bitmap that cannot be decoded");
            return Ok(());
        };
        self.place_bitmap(
            blit,
            ImageData { mime, bytes: data },
            (bw, bh),
            head.i32(8).unwrap_or(0) > 0,
            blend,
        )
    }

    fn place_bitmap(
        &mut self,
        blit: Blit,
        data: ImageData,
        size: (f64, f64),
        bottom_up: bool,
        blend: Blend,
    ) -> Result<(), Halt> {
        let (x, y, w, h) = blit.dest;
        let (bw, bh) = size;
        let (mut sx, mut sy, mut sw, mut sh) = blit.src;
        if sw == 0.0 || sh == 0.0 {
            (sx, sy, sw, sh) = (0.0, 0.0, bw, bh);
        }
        let top = if bottom_up { bh - sy - sh } else { sy };
        self.flush()?;
        let mut m = Matrix::new(w / sw, 0.0, 0.0, h / sh, x, y).then(&self.matrix());
        let mut node = Node::Image(Image {
            x: None,
            y: None,
            width: bw,
            height: bh,
            stretch: true,
            pixelated: false,
            data,
        });
        if (sx, top, sw, sh) != (0.0, 0.0, bw, bh) {
            node = Node::Viewport(Viewport {
                x: None,
                y: None,
                width: sw.abs(),
                height: sh.abs(),
                view_box: Bounds {
                    x: sx,
                    y: top,
                    width: sw.abs(),
                    height: sh.abs(),
                },
                clipped: false,
                content: Content {
                    body: vec![node],
                    ..Content::default()
                },
            });
            if sw < 0.0 || sh < 0.0 {
                m = Matrix::scale(1f64.copysign(sw), 1f64.copysign(sh)).then(&m);
            }
        }
        let group = Group {
            transform: transform_of(&m).map(GroupTransform::Matrix),
            opacity: blend.alpha.filter(|a| *a < 1.0),
            children: vec![node],
            ..Group::default()
        };
        self.write(Node::Group(group), true, ClipChoice::Dc)
    }

    /// `PATCOPY`: the rectangle filled with the brush, no outline.
    pub fn pattern_rect(&mut self, x: f64, y: f64, w: f64, h: f64) -> Result<(), Halt> {
        let saved = std::mem::replace(&mut self.dc.pen, dc::null_pen());
        let painted = self.paint(vec![geom::rect(x, y, x + w, y + h)], true, false, None, None);
        self.dc.pen = saved;
        painted
    }

    pub fn pixel(&mut self, x: f64, y: f64, color: u32) -> Result<(), Halt> {
        self.flush()?;
        let (dx, dy) = self.matrix().apply(x, y);
        let node = Node::Rect(Rect {
            x: dx,
            y: dy,
            width: 1.0,
            height: 1.0,
            fill: Fill::of(Paint::Color(colorref(color))),
        });
        self.write(node, true, ClipChoice::Dc)
    }

    /// Rectangles in device pixels filled with `brush` (a region).
    pub fn device_rects(&mut self, rects: &[(i32, i32, i32, i32)], brush: &Brush) -> Result<(), Halt> {
        let figures: Vec<_> = rects
            .iter()
            .map(|&(l, t, r, b)| geom::rect(f64::from(l), f64::from(t), f64::from(r), f64::from(b)))
            .collect();
        if figures.is_empty() {
            return Ok(());
        }
        self.flush()?;
        let layers = self.fill_attrs(&Matrix::IDENTITY, Some(brush))?;
        for layer in layers {
            let node = path_node(PathData::new(figures.clone()), None, Some(layer), Vec::new());
            self.write(node, true, ClipChoice::Dc)?;
        }
        Ok(())
    }

    /// A rectangle shaded from `c0` to `c1`, across or down.
    pub fn gradient_rect(&mut self, box_: (f64, f64, f64, f64), c0: Rgb, c1: Rgb, vertical: bool) -> Result<(), Halt> {
        let (l, t, r, b) = box_;
        self.flush()?;
        let m = self.matrix();
        let id = self.new_id(ResourceKind::Gradient);
        let (x1, y1, x2, y2) = if vertical { (l, t, l, b) } else { (l, t, r, t) };
        let stop = |offset, color| Stop {
            offset,
            color,
            opacity: None,
        };
        self.define(Def::LinearGradient {
            id,
            x1,
            y1,
            x2,
            y2,
            transform: None,
            stops: vec![stop(0.0, c0), stop(1.0, c1)],
        })?;
        let data = PathData::new(vec![geom::rect(l, t, r, b)]);
        let node = path_node(data, transform_of(&m), Some(Fill::of(Paint::Server(id))), Vec::new());
        self.write(node, true, ClipChoice::Dc)
    }

    /// A polygon in one colour (a gradient triangle at its mean colour).
    pub fn flat_polygon(&mut self, points: &[(f64, f64)], fill: Rgb) -> Result<(), Halt> {
        self.flush()?;
        let m = self.matrix();
        let data = PathData::new(geom::polyline(points, true).into_iter().collect());
        let node = path_node(data, transform_of(&m), Some(Fill::of(Paint::Color(fill))), Vec::new());
        self.write(node, true, ClipChoice::Dc)
    }
}

/// The bitmap encoded for the drawing: its bytes, type and size; `None` when it cannot be
/// decoded.
fn encoded(header: &[u8], bits: &[u8], blend: Blend) -> Option<(Vec<u8>, String, f64, f64)> {
    if blend.transparent.is_some() || blend.alpha.is_some() {
        if let Some(mut decoded) = dib::decode(header, bits) {
            if let Some(key) = blend.transparent {
                let rgb = [
                    (key & 0xFF) as u8,
                    ((key >> 8) & 0xFF) as u8,
                    ((key >> 16) & 0xFF) as u8,
                ];
                for [r, g, b, a] in decoded.rgba.as_chunks_mut::<4>().0 {
                    if [*r, *g, *b] == rgb {
                        *a = 0;
                    }
                }
                decoded.has_alpha = true;
            }
            let png = png::encode(decoded.width, decoded.height, &decoded.rgba, decoded.has_alpha);
            return Some((png, "image/png".to_owned(), decoded.width as f64, decoded.height as f64));
        }
    }
    let image = dib::to_image(header, bits)?;
    let head = Bytes(header);
    let (bw, bh) = (head.i32(4).ok()?, head.i32(8).ok()?);
    Some((
        image.data,
        image.mime.to_owned(),
        f64::from(bw).abs(),
        f64::from(bh).abs(),
    ))
}
