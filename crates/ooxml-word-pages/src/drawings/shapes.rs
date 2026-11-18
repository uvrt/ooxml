//! Owns: what a drawing's graphic paints, as docx2svg's `drawing.draw` paints it: a picture
//! in its box (its crop, its outline), a shape's fill then its outline (Word's drawn
//! arrowheads, the line cut back for them) in a group carrying its effects, a group's
//! members in its child coordinates scaled onto its box (a member's `a:grpFill` the
//! group's fill over the group's box), a VML shape as its preset and a VML text path
//! ([`super::vml`]), a chart or SmartArt by their painters, and a marked stand-in box for
//! what cannot be drawn -- each element turned by its own and its groups' rotations and
//! flips; what is not drawn faithfully said as warning codes.
//!
//! Must not: lay out a text box's text (the caller does, from [`Drawn::text_boxes`]), or
//! write SVG beyond what the renderers return.

use super::colours::{Said, WordColours};
use super::vml::{Inks, TextPath};
use super::write::{attrs_text, Primitive, PxBox};
use ooxml_base::Q;
use ooxml_drawingml::drawable::{Effects, Fill};
use ooxml_drawingml::read::{fill as read_fill, geometry as read_geometry};
use ooxml_drawingml::source::{self, GeometrySpec, ShapeStyle};
use ooxml_render::arrowhead::render_arrowheads;
use ooxml_render::effect::render_effects;
use ooxml_render::fill::{fill_attrs, outline_attrs, ShapeFrame};
use ooxml_render::paths::{path_ends, preset_path_data, spec_path_data, trim_path, GeometryPath};
use ooxml_render::Painter;
use ooxml_scene::drawing_num as num;
use ooxml_scene::markup::{path_text, Op, Sep};
use ooxml_schema::a;
use ooxml_word_model::model::Graphic;
use ooxml_xml::view::child;
use ooxml_xml::Element;

/// The device grid drawings are drawn on (Word's PDF export's): 300 px an inch.
pub const DPI: f64 = 300.0;

/// Presets whose `shape` path gradient is a rectangle's (`_RECTANGULAR`).
const RECTANGULAR: &[&str] = &["rect", "flowChartProcess"];

/// How a graphic's source says it is painted (`_paint`, `picture_crop`).
#[derive(Debug, Clone, Default)]
pub struct Paint {
    /// Its geometry (`None`: a rectangle).
    pub geometry: Option<GeometrySpec>,
    /// Its fill (a group's `grpSpPr`'s).
    pub fill: Option<source::Fill>,
    /// Its outline.
    pub line: Option<source::Outline>,
    /// Its effects.
    pub effects: Option<source::EffectList>,
    /// Its theme style references.
    pub style: Option<ShapeStyle>,
    /// A picture's crop: the fractions cut off its left, top, right and bottom.
    pub crop: Option<[Q; 4]>,
}

impl Paint {
    /// What `graphic`'s source element says.
    pub fn of(graphic: &Graphic) -> Paint {
        let Some(element) = graphic.source.as_ref() else {
            return Paint::default();
        };
        let kind = ooxml_xml::view::local(element);
        if matches!(kind, "pict" | "object" | "AlternateContent") {
            return super::vml::paint(element).unwrap_or_default();
        }
        let properties = child(
            element,
            if matches!(kind, "wgp" | "grpSp") {
                "grpSpPr"
            } else {
                "spPr"
            },
        );
        let mut paint = Paint {
            fill: read_fill::fill(properties),
            effects: properties
                .and_then(|p| child(p, "effectLst"))
                .and_then(|e| read_fill::effects(Some(a::EffectList::new(e)))),
            ..Paint::default()
        };
        if matches!(kind, "wgp" | "grpSp") {
            return paint;
        }
        paint.geometry = read_geometry::geometry_spec(properties.map(a::ShapeProperties::new));
        paint.line = read_fill::line(properties.and_then(|p| child(p, "ln")).map(a::Ln::new));
        paint.style = read_fill::shape_style(child(element, "style").map(a::ShapeStyle::new));
        if kind == "pic" {
            paint.fill = None;
            paint.crop = crop(element);
        }
        paint
    }
}

/// A picture's `a:srcRect`, `None` where it cuts nothing (`picture_crop`).
fn crop(pic: &Element) -> Option<[Q; 4]> {
    let rect = child(pic, "blipFill").and_then(|b| child(b, "srcRect"));
    let found = read_fill::relative_rect(rect.map(a::RelativeRect::new))?;
    if found.iter().all(|v| *v == 0.0) {
        return None;
    }
    // The fractions in hundred-thousandths, as `Fraction(v).limit_denominator(100000)`
    // reads a value stated in them.
    #[allow(clippy::cast_possible_truncation)]
    Some(found.map(|v| Q::new((v * 100_000.0).round() as i128, 100_000)))
}

/// One box's rotation and flips about its centre (`_Turn`).
#[derive(Debug, Clone, Copy)]
struct Turn {
    cx: f64,
    cy: f64,
    rotation: f64,
    flip_h: bool,
    flip_v: bool,
}

impl Turn {
    fn of(graphic: &Graphic, at: &PxBox) -> Option<Turn> {
        if graphic.rotation == 0 && !graphic.flip_h && !graphic.flip_v {
            return None;
        }
        let half = |v: ooxml_word_layout::px::Px| v.value() / Q::from_integer(2);
        Some(Turn {
            cx: to_f64(at.x.value() + half(at.width)),
            cy: to_f64(at.y.value() + half(at.height)),
            #[allow(clippy::cast_precision_loss)]
            rotation: graphic.rotation as f64 / 60000.0,
            flip_h: graphic.flip_h,
            flip_v: graphic.flip_v,
        })
    }

    fn scale(self) -> String {
        format!(
            "scale({} {})",
            if self.flip_h { -1 } else { 1 },
            if self.flip_v { -1 } else { 1 }
        )
    }

    fn forward(self) -> String {
        let mut parts = vec![format!("translate({} {})", num(self.cx), num(self.cy))];
        if self.rotation != 0.0 {
            parts.push(format!("rotate({})", num(self.rotation)));
        }
        if self.flip_h || self.flip_v {
            parts.push(self.scale());
        }
        parts.push(format!("translate({} {})", num(-self.cx), num(-self.cy)));
        parts.join(" ")
    }

    fn inverse(self) -> String {
        let mut parts = vec![format!("translate({} {})", num(self.cx), num(self.cy))];
        if self.flip_h || self.flip_v {
            parts.push(self.scale());
        }
        if self.rotation != 0.0 {
            parts.push(format!("rotate({})", num(-self.rotation)));
        }
        parts.push(format!("translate({} {})", num(-self.cx), num(-self.cy)));
        parts.join(" ")
    }
}

/// An exact length as a float.
pub fn to_f64(value: Q) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let (n, d) = (*value.numer() as f64, *value.denom() as f64);
    n / d
}

/// The turns (outermost first) as one SVG transform (`_transform`).
fn transform(turns: &[Turn]) -> Option<String> {
    let text: Vec<String> = turns.iter().map(|t| t.forward()).collect();
    (!text.is_empty()).then(|| text.join(" "))
}

/// A transform list as written, read back as operations.
fn ops(text: &str) -> Vec<Op> {
    text.split(')')
        .filter_map(|part| {
            let (name, args) = part.trim().split_once('(')?;
            let n: Vec<f64> = args.split_whitespace().filter_map(|a| a.parse().ok()).collect();
            Some(match (name, n.as_slice()) {
                ("translate", [x, y]) => Op::Translate(*x, *y, Sep::Space),
                ("scale", [x, y]) => Op::Scale(*x, *y, Sep::Space),
                ("rotate", [a]) => Op::Rotate(*a),
                _ => return None,
            })
        })
        .collect()
}

/// Where an element under `turns` sits on the page (`_frame`).
fn frame(turns: &[Turn]) -> ShapeFrame {
    let own = turns.last();
    let inverse: Vec<String> = turns.iter().rev().map(|t| t.inverse()).collect();
    ShapeFrame {
        rotation: own.map_or(0.0, |t| t.rotation),
        flip_h: own.is_some_and(|t| t.flip_h),
        flip_v: own.is_some_and(|t| t.flip_v),
        page_transform: (!inverse.is_empty()).then(|| ops(&inverse.join(" "))),
        rectangular: true,
    }
}

fn floats(at: &PxBox) -> (f64, f64, f64, f64) {
    (
        to_f64(at.x.value()),
        to_f64(at.y.value()),
        to_f64(at.width.value()),
        to_f64(at.height.value()),
    )
}

/// The geometry's paths over the box (`geometry_paths`).
fn geometry_paths(geometry: Option<&GeometrySpec>, at: &PxBox, said: &mut Said) -> Vec<GeometryPath> {
    let (x, y, w, h) = floats(at);
    match geometry {
        None => preset_path_data("rect", &[], (x, y, w, h)).unwrap_or_default(),
        Some(GeometrySpec::Preset { name, adjust }) => {
            preset_path_data(name, adjust, (x, y, w, h)).unwrap_or_else(|| {
                said.push(format!("drawing-geometry-unknown:{name}"));
                Vec::new()
            })
        }
        Some(GeometrySpec::Custom {
            adjustments,
            guides,
            paths,
        }) => spec_path_data(adjustments, guides, paths, &[], (x, y, w, h)),
    }
}

/// The painter of a chart or a SmartArt diagram in a box (`chart.Frames.draw`).
pub trait Frames {
    /// The primitives `graphic` paints in `at`, or `None` where it cannot be drawn (warned).
    fn draw(
        &self,
        graphic: &Graphic,
        at: &PxBox,
        path: &str,
        painter: &mut Painter<'_>,
        said: &mut Said,
    ) -> Option<Vec<Primitive>>;
}

/// What drawing a graphic made (`Drawn`).
#[derive(Debug, Default)]
pub struct Drawn {
    /// Its primitives, in paint order.
    pub primitives: Vec<Primitive>,
    /// The shapes with text boxes to lay out, with their boxes.
    pub text_boxes: Vec<(std::sync::Arc<Graphic>, PxBox)>,
    /// What could not be drawn faithfully, as warning codes in the order met.
    pub said: Said,
    /// The faces its text is drawn in (a text path's): `(face, bold, italic)`.
    pub faces: Vec<(String, bool, bool)>,
}

/// One drawing pass over a graphic.
pub struct Draw<'a, 'p, 'm> {
    /// The colours.
    pub colours: &'a WordColours<'a>,
    /// The resources and their ids.
    pub painter: &'p mut Painter<'m>,
    /// The charts' and diagrams' painter.
    pub frames: &'a dyn Frames,
    /// A text path's faces' outlines (`None`: none at hand).
    pub inks: Option<&'a dyn Inks>,
    /// The theme's minor Latin face (a text path in no face is drawn in it).
    pub minor: Option<&'a str>,
    /// What is drawn.
    pub drawn: Drawn,
}

/// The enclosing groups' fill and its box, and their turns.
#[derive(Clone, Default)]
struct Enclosing {
    fill: Option<Fill>,
    fill_box: Option<PxBox>,
    turns: Vec<Turn>,
}

impl Draw<'_, '_, '_> {
    /// What `graphic` (`None`: one not read) paints in `at`.
    pub fn graphic(&mut self, graphic: Option<&std::sync::Arc<Graphic>>, at: &PxBox, path: &str) {
        self.draw(graphic, at, path, &Enclosing::default());
    }

    fn stand_in(&mut self, graphic: Option<&Graphic>, at: &PxBox, path: &str) {
        self.drawn.primitives.push(Primitive::StandIn {
            path: path.to_owned(),
            at: *at,
            what: "drawing".into(),
        });
        let what = graphic.map_or_else(
            || "none".to_owned(),
            |g| {
                g.uri.as_ref().map_or_else(
                    || {
                        if &*g.kind == "other" {
                            "graphic".to_owned()
                        } else {
                            g.kind.to_string()
                        }
                    },
                    ToString::to_string,
                )
            },
        );
        let last = what.rsplit('/').next().unwrap_or(&what).to_owned();
        self.drawn.said.push(format!("drawing-not-drawn:{last}"));
    }

    fn draw(&mut self, graphic: Option<&std::sync::Arc<Graphic>>, at: &PxBox, path: &str, up: &Enclosing) {
        if let Some(g) = graphic.filter(|g| matches!(&*g.kind, "chart" | "diagram")) {
            if let Some(found) = self.frames.draw(g, at, path, self.painter, &mut self.drawn.said) {
                let outer = transform(&up.turns);
                for mut p in found {
                    if let Primitive::Markup { markup, .. } = &p {
                        self.drawn.faces.extend(super::write::text_faces(markup));
                    }
                    if let (Some(outer), Primitive::Markup { transform, .. }) = (&outer, &mut p) {
                        *transform = Some(match transform.take() {
                            Some(t) => format!("{outer} {t}"),
                            None => format!("{outer} None"),
                        });
                    }
                    self.drawn.primitives.push(p);
                }
                return;
            }
        }
        let Some(g) = graphic.filter(|g| !matches!(&*g.kind, "other" | "chart" | "diagram")) else {
            self.stand_in(graphic.map(AsRef::as_ref), at, path);
            return;
        };
        for name in g.unsupported.iter() {
            self.drawn.said.push(format!("drawing-not-drawn:{name}"));
        }
        let mut here = up.clone();
        here.turns.extend(Turn::of(g, at));
        let paint = Paint::of(g);
        match &*g.kind {
            "picture" => {
                self.drawn.primitives.push(Primitive::Image {
                    path: path.to_owned(),
                    relationship: g.relationship.as_deref().map(str::to_owned),
                    at: *at,
                    transform: transform(&here.turns),
                    crop: paint.crop,
                });
                self.outline(&paint, at, path, &here.turns, None);
            }
            "group" => self.group(g, &paint, at, path, here),
            "textpath" => self.text_path(g, at, path, &here.turns),
            _ => self.shape(g, &paint, at, path, &here),
        }
    }

    /// A VML text path (`drawing.draw`'s `textpath`): nothing where its shape type does not
    /// fit it, else its text, fitted to the box by its outlines where they are at hand.
    fn text_path(&mut self, g: &Graphic, at: &PxBox, path: &str, turns: &[Turn]) {
        let Some(text) = g.source.as_ref().and_then(|s| TextPath::of(s, self.minor)) else {
            return;
        };
        if text.fit == Some(false) {
            return;
        }
        let outlines = text.ink(self.inks);
        self.drawn.primitives.push(Primitive::Markup {
            path: path.to_owned(),
            markup: text.markup(at, outlines.as_ref()),
            transform: transform(turns),
        });
        self.drawn.faces.push(text.drawn_face(outlines.as_ref()));
        if outlines.is_none() {
            self.drawn.said.push("drawing-approximate:text path".into());
        }
    }

    fn group(&mut self, g: &std::sync::Arc<Graphic>, paint: &Paint, at: &PxBox, path: &str, mut here: Enclosing) {
        let (cw, ch) = g.child_extent;
        let (ox, oy) = g.child_offset;
        let ratio = |a: ooxml_word_layout::px::Px, b: i64| {
            if b == 0 {
                Q::from_integer(1)
            } else {
                a.value() / Q::from_integer(i128::from(b))
            }
        };
        let (sx, sy) = (ratio(at.width, cw), ratio(at.height, ch));
        let own = paint
            .fill
            .as_ref()
            .and_then(|f| self.colours.fill(f, None, here.fill.as_ref(), &mut self.drawn.said));
        if let Some(own) = own {
            here.fill = Some(own);
            here.fill_box = Some(*at);
        }
        let begin = self.drawn.primitives.len();
        for member in g.children.iter() {
            let (mx, my) = member.offset;
            let (mw, mh) = member.extent;
            let q = |v: i64| Q::from_integer(i128::from(v));
            let member_box = PxBox {
                x: (at.x.value() + (q(mx) - q(ox)) * sx).into(),
                y: (at.y.value() + (q(my) - q(oy)) * sy).into(),
                width: (q(mw) * sx).into(),
                height: (q(mh) * sy).into(),
            };
            let member_path = if member.path.is_empty() { path } else { &member.path };
            self.draw(Some(member), &member_box, member_path, &here);
        }
        let effects = self.colours.effects(paint.effects.as_ref(), None, &mut self.drawn.said);
        self.wrap(begin, effects.as_ref(), path);
    }

    fn shape(&mut self, g: &std::sync::Arc<Graphic>, paint: &Paint, at: &PxBox, path: &str, here: &Enclosing) {
        let style = paint.style.as_ref();
        let fill = match &paint.fill {
            Some(f) => self.colours.fill(f, None, here.fill.as_ref(), &mut self.drawn.said),
            None => self
                .colours
                .style_fill(style.and_then(|s| s.fill.as_ref()), &mut self.drawn.said),
        };
        let fill_box = match (&paint.fill, here.fill_box) {
            (Some(source::Fill::Group), Some(b)) => b,
            _ => *at,
        };
        let paths = geometry_paths(paint.geometry.as_ref(), at, &mut self.drawn.said);
        let begin = self.drawn.primitives.len();
        let turned = transform(&here.turns);
        if let Some(fill) = fill.filter(|f| *f != Fill::None) {
            let name = match &paint.geometry {
                None => Some("rect"),
                Some(GeometrySpec::Preset { name, .. }) => Some(name.as_str()),
                Some(GeometrySpec::Custom { .. }) => None,
            };
            let mut frame = frame(&here.turns);
            frame.rectangular = name.is_some_and(|n| RECTANGULAR.contains(&n));
            let attrs = attrs_text(&fill_attrs(
                Some(&fill),
                self.painter,
                Some(floats(&fill_box)),
                Some(&frame),
            ));
            for p in paths.iter().filter(|p| p.fill != "none" && !p.d.is_empty()) {
                if p.fill != "norm" {
                    self.drawn.said.push(format!("drawing-fill-approximate:{}", p.fill));
                }
                self.drawn.primitives.push(Primitive::Path {
                    path: path.to_owned(),
                    d: path_text(&p.d),
                    attrs: attrs.clone(),
                    transform: turned.clone(),
                });
            }
        }
        self.outline(paint, at, path, &here.turns, Some(&paths));
        let effects = self.colours.effects(
            paint.effects.as_ref(),
            style.and_then(|s| s.effect.as_ref()),
            &mut self.drawn.said,
        );
        self.wrap(begin, effects.as_ref(), path);
        if !g.text.is_empty() {
            self.drawn.text_boxes.push((std::sync::Arc::clone(g), *at));
        }
    }

    fn outline(&mut self, paint: &Paint, at: &PxBox, path: &str, turns: &[Turn], paths: Option<&[GeometryPath]>) {
        let reference = paint.style.as_ref().and_then(|s| s.line.as_ref());
        let Some(outline) = self
            .colours
            .outline(paint.line.as_ref(), reference, &mut self.drawn.said)
        else {
            return;
        };
        let owned;
        let paths = match paths {
            Some(p) => p,
            None => {
                owned = geometry_paths(paint.geometry.as_ref(), at, &mut self.drawn.said);
                &owned
            }
        };
        let attrs = attrs_text(&outline_attrs(
            Some(&outline),
            self.painter,
            Some(floats(at)),
            Some(&frame(turns)),
        ));
        let turned = transform(turns);
        for p in paths.iter().filter(|p| p.stroke && !p.d.is_empty()) {
            let mut d = p.d.clone();
            let mut heads = Vec::new();
            if outline.head_end.is_some() || outline.tail_end.is_some() {
                if let Some(ends) = path_ends(&d) {
                    let drawn = render_arrowheads(Some(&outline), &ends, DPI);
                    heads = drawn
                        .elements
                        .iter()
                        .map(ooxml_scene::markup::Element::to_svg)
                        .collect();
                    d = trim_path(&d, drawn.start, drawn.end);
                }
            }
            self.drawn.primitives.push(Primitive::Path {
                path: path.to_owned(),
                d: path_text(&d),
                attrs: format!("fill=\"none\" {attrs}"),
                transform: turned.clone(),
            });
            for markup in heads {
                self.drawn.primitives.push(Primitive::Markup {
                    path: path.to_owned(),
                    markup,
                    transform: turned.clone(),
                });
            }
        }
    }

    /// The primitives from `begin` on in a group carrying `effects` (`_wrap`).
    fn wrap(&mut self, begin: usize, effects: Option<&Effects>, path: &str) {
        if effects.is_none() || begin == self.drawn.primitives.len() {
            return;
        }
        if let Some(attr) = render_effects(effects, self.painter, None) {
            self.drawn.primitives.insert(
                begin,
                Primitive::Begin {
                    path: path.to_owned(),
                    attrs: attrs_text(&[attr]),
                },
            );
            self.drawn.primitives.push(Primitive::End);
        }
    }
}
