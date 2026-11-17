//! Owns: how GDI paints -- a pen as stroke attributes (Windows' widths, caps, joins and
//! dashes with their caps inside), a brush as fill layers (solid, hatch over its background,
//! pattern), the hatch and pattern tiles, figures drawn interior first and outline after,
//! and `LINETO`s gathered into one path.
//!
//! Measured on Windows (and followed): a null pen strokes nothing and a null brush fills
//! nothing; every pen width is honoured and is at least one device pixel; a `CREATEPEN`
//! pen has round caps and mitred joins, a geometric pen's default is round and round; dash
//! lengths include the caps (`PS_DASH` is three widths on and one off, `PS_DOT` one and
//! one); a user style's entries are logical units.
//!
//! Must not: decide what a record means.

use super::{transform_of, ClipChoice, Player, Run};
use crate::dc::{self, Brush, Pen, BLACK, WHITE};
use crate::error::Halt;
use crate::pymath::fsum;
use ooxml_scene::svg::write_node;
use ooxml_scene::{
    Def, Figure, Fill, Image, ImageData, LineCap, LineJoin, Matrix, Node, Paint, Path, PathData, ResourceId,
    ResourceKind, Rgb, Segment, StrokeAttr,
};

/// What a pattern or gradient is kept for, so that one is drawn once per key.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PatternKey {
    GdiHatch {
        hatch: u32,
        color: Rgb,
        m: Matrix,
    },
    GdiImage {
        serial: u64,
        m: Matrix,
    },
    PlusHatch {
        shape: usize,
        fore: u32,
        m: Matrix,
        origin: (f64, f64),
    },
    PlusLinear(crate::plus::objects::Linear),
    PlusTexture {
        serial: u64,
        wrap: i32,
        m: Matrix,
    },
}

/// GDI's hatch tiles (8 by 8 device pixels, one pixel wide), as cells.
pub(crate) fn hatch_cells(shape: usize) -> Vec<(f64, f64, f64, f64)> {
    let horizontal = vec![(0.0, 0.0, 8.0, 1.0)];
    let vertical = vec![(0.0, 0.0, 1.0, 8.0)];
    let down: Vec<_> = (0..8).map(|i| (f64::from(i), f64::from(i), 1.0, 1.0)).collect();
    let up: Vec<_> = (0..8).map(|i| (f64::from(7 - i), f64::from(i), 1.0, 1.0)).collect();
    match shape {
        1 => vertical,
        2 => down,
        3 => up,
        4 => [horizontal, vertical].concat(),
        5 => [down, up].concat(),
        _ => horizontal,
    }
}

/// Dash patterns in pen widths, as drawn (caps included): measured on Windows.
fn gdi_dashes(kind: u32) -> Option<&'static [f64]> {
    match kind {
        dc::PS_DASH => Some(&[3.0, 1.0]),
        dc::PS_DOT | dc::PS_ALTERNATE => Some(&[1.0, 1.0]),
        dc::PS_DASHDOT => Some(&[3.0, 1.0, 1.0, 1.0]),
        dc::PS_DASHDOTDOT => Some(&[3.0, 1.0, 1.0, 1.0, 1.0, 1.0]),
        _ => None,
    }
}

/// Dashes for SVG: GDI's (and GDI+'s round) dashes include their caps, which SVG adds
/// outside each dash; an odd list is repeated.
pub(crate) fn dashes_for_caps(dashes: Vec<f64>, width: f64, inside: bool) -> Vec<f64> {
    let mut out: Vec<f64> = if inside {
        dashes
            .iter()
            .enumerate()
            .map(|(i, v)| if i % 2 == 0 { (v - width).max(0.0) } else { v + width })
            .collect()
    } else {
        dashes
    };
    if out.len() % 2 == 1 {
        out = [out.clone(), out].concat();
    }
    out
}

/// `max(a, b)` as the reference picks: `a` unless `b` is greater.
pub(crate) fn py_max(a: f64, b: f64) -> f64 {
    if b > a {
        b
    } else {
        a
    }
}

impl Player<'_> {
    fn pen_color(&self) -> Rgb {
        match self.dc.rop2 {
            1 => BLACK,
            16 => WHITE,
            _ => self.dc.pen.color,
        }
    }

    /// The current pen's stroke attributes for a shape in the space of `m` (logical to
    /// device), or `None` for no stroke. `device` is given for a path already in device
    /// space: the matrix its logical width is scaled by.
    pub fn stroke_attrs(&self, m: &Matrix, device: Option<&Matrix>) -> Option<Vec<StrokeAttr>> {
        let pen: &Pen = &self.dc.pen;
        if pen.null || pen.kind() == dc::PS_NULL {
            return None;
        }
        let scale = m.mean_scale();
        if scale == 0.0 {
            return None;
        }
        let pixel = 1.0 / scale;
        let device_scale = device.map_or(1.0, Matrix::mean_scale);
        let width = if (!pen.geometric && !pen.plain) || pen.width <= 0.0 {
            pixel
        } else {
            py_max(pen.width * device_scale, pixel)
        };
        let (cap, join) = if pen.plain || !pen.geometric {
            (LineCap::Round, LineJoin::Miter)
        } else {
            let cap = match pen.style & dc::PS_ENDCAP_MASK {
                dc::PS_ENDCAP_SQUARE => LineCap::Square,
                dc::PS_ENDCAP_FLAT => LineCap::Butt,
                _ => LineCap::Round,
            };
            let join = match pen.style & dc::PS_JOIN_MASK {
                dc::PS_JOIN_BEVEL => LineJoin::Bevel,
                dc::PS_JOIN_MITER => LineJoin::Miter,
                _ => LineJoin::Round,
            };
            (cap, join)
        };
        let mut out = vec![
            StrokeAttr::Paint(Paint::Color(self.pen_color())),
            StrokeAttr::Width(width),
        ];
        if cap != LineCap::Butt {
            out.push(StrokeAttr::Cap(cap));
        }
        if join != LineJoin::Miter {
            out.push(StrokeAttr::Join(join));
        } else if self.dc.miter_limit != 4.0 {
            out.push(StrokeAttr::MiterLimit(py_max(self.dc.miter_limit, 1.0)));
        }
        let dashes: Option<Vec<f64>> = if let Some(pattern) = gdi_dashes(pen.kind()) {
            Some(pattern.iter().map(|v| v * width).collect())
        } else if pen.kind() == dc::PS_USERSTYLE && !pen.user_style.is_empty() {
            let unit = if pen.geometric { 1.0 } else { pixel } * device_scale;
            Some(pen.user_style.iter().map(|&v| f64::from(v) * unit).collect())
        } else {
            None
        };
        if let Some(dashes) = dashes {
            if !dashes.is_empty() && fsum(dashes.iter().copied()) > 0.0 {
                out.push(StrokeAttr::Dashes(dashes_for_caps(dashes, width, cap != LineCap::Butt)));
            }
        }
        Some(out)
    }

    /// The brush's fill layers, one per painted layer (an opaque hatch is its background,
    /// then its lines); empty for no fill.
    pub fn fill_attrs(&mut self, m: &Matrix, brush: Option<&Brush>) -> Result<Vec<Fill>, Halt> {
        let brush = brush.cloned().unwrap_or_else(|| self.dc.brush.clone());
        let even_odd = self.dc.fill_mode == dc::ALTERNATE;
        let layer = |paint: Paint| Fill {
            paint,
            opacity: None,
            even_odd,
        };
        Ok(match &brush {
            Brush::Solid(color) => {
                let fill = match self.dc.rop2 {
                    1 => BLACK,
                    16 => WHITE,
                    _ => *color,
                };
                vec![layer(Paint::Color(fill))]
            }
            Brush::Hatch { color, hatch } => {
                let mut layers = Vec::new();
                if self.dc.bk_mode == dc::OPAQUE {
                    layers.push(layer(Paint::Color(self.dc.bk_color)));
                }
                layers.push(layer(Paint::Server(self.hatch(*hatch, *color, m)?)));
                layers
            }
            Brush::Pattern(image) => vec![layer(Paint::Server(self.image_pattern(image, m)?))],
            Brush::Null => Vec::new(),
        })
    }

    /// A known pattern's id.
    pub fn pattern(&self, key: &PatternKey) -> Option<ResourceId> {
        self.patterns.iter().find(|(k, _)| k == key).map(|(_, id)| *id)
    }

    fn pattern_transform(m: &Matrix) -> Option<Matrix> {
        m.invert().filter(|inv| !inv.is_identity())
    }

    fn hatch(&mut self, hatch: u32, color: Rgb, m: &Matrix) -> Result<ResourceId, Halt> {
        let key = PatternKey::GdiHatch { hatch, color, m: *m };
        if let Some(id) = self.pattern(&key) {
            return Ok(id);
        }
        let id = self.new_id(ResourceKind::Hatch);
        let shape = usize::try_from(hatch).ok().filter(|h| *h <= 5).unwrap_or(0);
        self.define(Def::Pattern {
            id,
            width: 8.0,
            height: 8.0,
            transform: Self::pattern_transform(m),
            children: vec![cells(shape, Fill::of(Paint::Color(color)))],
        })?;
        self.patterns.push((key, id));
        Ok(id)
    }

    fn image_pattern(&mut self, image: &dc::BrushImage, m: &Matrix) -> Result<ResourceId, Halt> {
        let key = PatternKey::GdiImage {
            serial: image.serial,
            m: *m,
        };
        if let Some(id) = self.pattern(&key) {
            return Ok(id);
        }
        let id = self.new_id(ResourceKind::Pattern);
        let (w, h) = (image.width as f64, image.height as f64);
        self.define(Def::Pattern {
            id,
            width: w,
            height: h,
            transform: Self::pattern_transform(m),
            children: vec![Node::Image(Image {
                x: None,
                y: None,
                width: w,
                height: h,
                stretch: false,
                pixelated: false,
                data: ImageData {
                    mime: "image/png".to_owned(),
                    bytes: image.png.clone(),
                },
            })],
        })?;
        self.patterns.push((key, id));
        Ok(id)
    }

    /// Draws figures in the space of `m` (default: the logical space) with the brush and
    /// the pen, as GDI does: the interior first, then the outline.
    pub fn paint(
        &mut self,
        figures: Vec<Figure>,
        fill: bool,
        stroke: bool,
        m: Option<Matrix>,
        device: Option<Matrix>,
    ) -> Result<(), Halt> {
        if figures.is_empty() || !self.visible() {
            return Ok(());
        }
        self.flush()?;
        let m = m.unwrap_or_else(|| self.matrix());
        let data = PathData::new(figures);
        let transform = transform_of(&m);
        let layers = if fill { self.fill_attrs(&m, None)? } else { Vec::new() };
        let stroke_attrs = if stroke {
            self.stroke_attrs(&m, device.as_ref())
        } else {
            None
        };
        if layers.is_empty() {
            let Some(stroke_attrs) = stroke_attrs else {
                return Ok(());
            };
            let node = path_node(data, transform, Some(Fill::of(Paint::None)), stroke_attrs);
            return self.write(node, true, ClipChoice::Dc);
        }
        let last = layers.len() - 1;
        for (i, layer) in layers.into_iter().enumerate() {
            let stroke = if i == last {
                stroke_attrs.clone().unwrap_or_default()
            } else {
                Vec::new()
            };
            self.write(
                path_node(data.clone(), transform, Some(layer), stroke),
                true,
                ClipChoice::Dc,
            )?;
        }
        Ok(())
    }

    /// One `LINETO`: stroked on its own, caps at both ends, as GDI draws it.
    pub fn segment(&mut self, x0: f64, y0: f64, x1: f64, y1: f64) -> Result<(), Halt> {
        if !self.visible() {
            return Ok(());
        }
        let m = self.matrix();
        let Some(stroke) = self.stroke_attrs(&m, None) else {
            return Ok(());
        };
        let transform = transform_of(&m);
        let mut key = String::new();
        let shape = path_node(
            PathData::default(),
            transform,
            Some(Fill::of(Paint::None)),
            stroke.clone(),
        );
        write_node(&mut key, &shape, "");
        let figure = Figure {
            start: (x0, y0),
            segments: vec![Segment::Line(x1, y1)],
            closed: false,
        };
        if let Some(run) = self.run.as_mut().filter(|r| r.key == key) {
            run.figures.push(figure);
            return Ok(());
        }
        self.flush()?;
        self.run = Some(Run {
            key,
            transform,
            stroke,
            figures: vec![figure],
        });
        Ok(())
    }
}

/// A path element of these parts.
pub(crate) fn path_node(
    data: PathData,
    transform: Option<Matrix>,
    fill: Option<Fill>,
    stroke: Vec<StrokeAttr>,
) -> Node {
    Node::Path(Path {
        data,
        transform,
        fill,
        stroke,
        ..Path::default()
    })
}

/// A hatch tile's cells as a path.
pub(crate) fn cells(shape: usize, fill: Fill) -> Node {
    Node::Cells(ooxml_scene::Cells {
        cells: hatch_cells(shape),
        fill,
    })
}
