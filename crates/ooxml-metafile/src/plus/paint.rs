//! Owns: how an EMF+ brush paints -- ARGB colours with their opacity, hatches (8 by 8
//! device pixels from the rendering origin, the fore colour over the back; percentages as
//! the fore colour's share), linear gradients (not repeated past their rectangle, the
//! record's reserved colours, blends as stops), textures (tiles from the brush space's
//! origin, drawn sharp; `Clamp` once, clipped to the shape) -- and filling figures with one.
//!
//! Must not: shade a path gradient (`shade` does) or stroke (`stroke` does).

use super::image::{image_markup, Image};
use super::objects::{Brush, Linear, Texture};
use super::Plus;
use crate::error::{Halt, RecordError};
use crate::player::paint::{cells, PatternKey};
use crate::player::transform_of;
use ooxml_scene::{
    Def, Figure, Fill, Group, GroupTransform, Matrix, Node, Paint, Path, PathData, ResourceId, ResourceKind, Rgb, Stop,
};
use std::sync::Arc;

const W_CLAMP: i32 = 4;

/// One painted layer of a brush: its paint and opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Layer {
    pub paint: Paint,
    pub opacity: Option<f64>,
}

impl Layer {
    pub fn fill(&self, even_odd: bool) -> Fill {
        Fill {
            paint: self.paint,
            opacity: self.opacity,
            even_odd,
        }
    }
}

/// An ARGB colour as a colour and its opacity.
pub(crate) fn argb(value: u32) -> (Rgb, f64) {
    (
        Rgb(
            ((value >> 16) & 0xFF) as u8,
            ((value >> 8) & 0xFF) as u8,
            (value & 0xFF) as u8,
        ),
        f64::from((value >> 24) & 0xFF) / 255.0,
    )
}

/// A colour layer: its opacity written when it is below 1.
pub(crate) fn colour(value: u32) -> Layer {
    let (rgb, alpha) = argb(value);
    Layer {
        paint: Paint::Color(rgb),
        opacity: (alpha < 1.0).then_some(alpha),
    }
}

/// A path filled with one ARGB colour (`prop` is `fill`).
pub(crate) fn colour_path(figures: Vec<Figure>, transform: Option<Matrix>, value: u32, _prop: &str) -> Node {
    Node::Path(Path {
        data: PathData::new(figures),
        transform,
        fill: Some(colour(value).fill(false)),
        ..Path::default()
    })
}

/// Hatch styles drawn exactly: their tiles.
fn hatch_shape(style: u32) -> Option<usize> {
    usize::try_from(style).ok().filter(|s| *s <= 5)
}

/// Named hatches drawn as the plain hatch of their direction (approximately).
fn like(style: u32) -> Option<usize> {
    Some(match style {
        18 | 20 | 22 | 30 => 2,
        19 | 21 | 23 | 31 => 3,
        24 | 26 | 28 | 33 => 1,
        25 | 27 | 29 | 32 => 0,
        43 | 48 => 4,
        44 | 46 | 51 => 5,
        _ => return None,
    })
}

/// `HatchStylePercent05` to `Percent90`: the fore colour's share of each tile.
fn percent(style: u32) -> Option<f64> {
    Some(match style {
        6 => 0.05,
        7 => 0.10,
        8 => 0.20,
        9 => 0.25,
        10 => 0.30,
        11 => 0.40,
        12 => 0.50,
        13 => 0.60,
        14 => 0.70,
        15 => 0.75,
        16 => 0.80,
        17 => 0.90,
        _ => return None,
    })
}

impl Plus<'_, '_> {
    /// The layers `brush` paints for a shape in the space of `m`; `None` for a brush drawn
    /// another way (a path gradient's fill, a clamped texture's fill).
    pub fn paint_layers(&mut self, brush: &Brush, m: &Matrix, stroke: bool) -> Result<Option<Vec<Layer>>, Halt> {
        Ok(Some(match brush {
            Brush::Solid(c) => {
                if argb(*c).1 > 0.0 {
                    vec![colour(*c)]
                } else {
                    Vec::new()
                }
            }
            Brush::Hatch { style, fore, back } => {
                let mut layers = Vec::new();
                if argb(*back).1 > 0.0 {
                    layers.push(colour(*back));
                }
                if let Some(shape) = hatch_shape(*style).or_else(|| like(*style)) {
                    if hatch_shape(*style).is_none() {
                        self.p.skip(format!(
                            "EmfPlus hatch style {style} (drawn as the plain hatch of its direction)"
                        ));
                    }
                    layers.push(Layer {
                        paint: Paint::Server(self.hatch(shape, *fore, m)?),
                        opacity: None,
                    });
                } else {
                    if percent(*style).is_none() {
                        self.p
                            .skip(format!("EmfPlus hatch style {style} (drawn as its mean colour)"));
                    }
                    let (rgb, alpha) = argb(*fore);
                    layers.push(Layer {
                        paint: Paint::Color(rgb),
                        opacity: Some(alpha * percent(*style).unwrap_or(0.5)),
                    });
                }
                layers
            }
            Brush::Linear(linear) => vec![Layer {
                paint: Paint::Server(self.linear(linear)?),
                opacity: None,
            }],
            Brush::Texture(texture) => {
                if texture.wrap == W_CLAMP && !stroke {
                    return Ok(None);
                }
                if texture.wrap == W_CLAMP {
                    self.p.skip("EmfPlus Clamp texture pens (drawn tiled)");
                }
                vec![Layer {
                    paint: Paint::Server(self.texture(texture)?),
                    opacity: None,
                }]
            }
            Brush::PathGradient(g) => {
                if !stroke {
                    return Ok(None);
                }
                self.p.skip("EmfPlus path gradient pens (drawn in their centre colour)");
                vec![colour(g.center_color)]
            }
        }))
    }

    fn hatch(&mut self, shape: usize, fore: u32, m: &Matrix) -> Result<ResourceId, Halt> {
        let origin = self.pp.state.origin;
        let key = PatternKey::PlusHatch {
            shape,
            fore,
            m: *m,
            origin,
        };
        if let Some(id) = self.p.pattern(&key) {
            return Ok(id);
        }
        let id = self.p.new_id(ResourceKind::Hatch);
        let inverse = m.invert().unwrap_or(Matrix::IDENTITY);
        let transform = Matrix::translate(origin.0, origin.1).then(&inverse);
        self.p.define(Def::Pattern {
            id,
            width: 8.0,
            height: 8.0,
            transform: Some(transform),
            children: vec![cells(shape, colour(fore).fill(false))],
        })?;
        self.p.patterns.push((key, id));
        Ok(id)
    }

    fn linear(&mut self, brush: &Linear) -> Result<ResourceId, Halt> {
        let key = PatternKey::PlusLinear(brush.clone());
        if let Some(id) = self.p.pattern(&key) {
            return Ok(id);
        }
        let id = self.p.new_id(ResourceKind::Gradient);
        let [x, y, w, _h] = brush.rect;
        let stops = brush
            .stops
            .iter()
            .map(|&(pos, c)| {
                let (color, alpha) = argb(c);
                Stop {
                    offset: pos.clamp(0.0, 1.0),
                    color,
                    opacity: (alpha < 1.0).then_some(alpha),
                }
            })
            .collect();
        self.p.define(Def::LinearGradient {
            id,
            x1: x,
            y1: y,
            x2: x + w,
            y2: y,
            transform: transform_of(&brush.matrix),
            stops,
        })?;
        self.p.patterns.push((key, id));
        Ok(id)
    }

    fn texture(&mut self, brush: &Texture) -> Result<ResourceId, Halt> {
        let key = PatternKey::PlusTexture {
            serial: brush.image.serial,
            wrap: brush.wrap,
            m: brush.matrix,
        };
        if let Some(id) = self.p.pattern(&key) {
            return Ok(id);
        }
        let id = self.p.new_id(ResourceKind::Pattern);
        let img = &brush.image;
        // Windows tiles every wrap mode but Clamp alike and draws the tiles' pixels sharp.
        let content = image_markup(img, (0.0, 0.0, img.width, img.height), 5, false);
        self.p.define(Def::Pattern {
            id,
            width: img.width,
            height: img.height,
            transform: transform_of(&brush.matrix),
            children: vec![content],
        })?;
        self.p.patterns.push((key, id));
        Ok(id)
    }

    /// Fills world-space figures with a brush, in the current transform and clip.
    pub fn fill(&mut self, figures: Vec<Figure>, brush: Option<Brush>, even_odd: bool) -> Result<(), RecordError> {
        let Some(brush) = brush.filter(|_| !figures.is_empty()) else {
            return Ok(());
        };
        let m = self.matrix();
        let transform = transform_of(&m);
        let Some(layers) = self.paint_layers(&brush, &m, false)? else {
            return match brush {
                Brush::Texture(t) => Ok(self.clamped(&t, figures, &m, even_odd)?),
                Brush::PathGradient(g) => self.path_gradient(&g, figures, &m, even_odd),
                _ => Ok(()),
            };
        };
        let data = PathData::new(figures);
        for layer in layers {
            self.write(Node::Path(Path {
                data: data.clone(),
                transform,
                fill: Some(layer.fill(even_odd)),
                ..Path::default()
            }))?;
        }
        Ok(())
    }

    /// A `Clamp` texture: the image once, through the brush transform, clipped to the shape.
    fn clamped(&mut self, brush: &Texture, figures: Vec<Figure>, m: &Matrix, even_odd: bool) -> Result<(), Halt> {
        let id = self.clip_path(PathData::new(figures), None, even_odd)?;
        let img: &Arc<Image> = &brush.image;
        let image = image_markup(img, (0.0, 0.0, img.width, img.height), 5, false);
        let inner = Node::Group(Group {
            clip: Some(id),
            children: vec![Node::Group(Group {
                transform: transform_of(&brush.matrix).map(GroupTransform::Matrix),
                children: vec![image],
                ..Group::default()
            })],
            ..Group::default()
        });
        let node = match transform_of(m) {
            Some(t) => Node::Group(Group {
                transform: Some(GroupTransform::Matrix(t)),
                children: vec![inner],
                ..Group::default()
            }),
            None => inner,
        };
        self.write(node)
    }

    /// A clip path of one path (with a transform, even-odd or not).
    pub fn clip_path(&mut self, data: PathData, transform: Option<Matrix>, even_odd: bool) -> Result<ResourceId, Halt> {
        let id = self.p.new_id(ResourceKind::Clip);
        self.p.define(Def::ClipPath {
            id,
            parent: None,
            paths: vec![Path {
                data,
                transform,
                clip_even_odd: even_odd,
                ..Path::default()
            }],
        })?;
        Ok(id)
    }
}
