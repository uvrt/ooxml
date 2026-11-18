//! Owns: SmartArt in a Word drawing (`dgm:relIds`), as docx2svg's `diagram.py` draws it:
//! the drawing Word cached for the diagram (`dsp:drawing`, found from the data model the
//! drawing names, [`drawing_part`]), read as a shape tree, each shape resolved as Word
//! resolves it -- colours through the document's theme and colour map, the theme style
//! references (`dsp:style`) through its format scheme, text in the face the style's
//! `a:fontRef` names (the theme's minor or major face), in the reference's colour, at the
//! size the cache states -- a group filling the drawing's frame, painted by the chart's
//! renderer ([`super::chart::render`]).
//!
//! Must not: lay a diagram out from its data (Word's cache is drawn, or nothing), or
//! write SVG beyond what the renderer returns.

use super::colours::{Said, WordColours};
use super::shapes::to_f64;
use super::write::{Primitive, PxBox};
use ooxml_base::Q;
use ooxml_colour::SourceColour;
use ooxml_drawingml::drawable::{
    self, Bullet, Colour, Connector, Element as Drawable, Geometry, Group, Paragraph, ParagraphProperties, Shape,
    TextBody, TextRect, TextRun, Transform,
};
use ooxml_drawingml::read::geometry::group_xfrms;
use ooxml_drawingml::read::tree::shape_tree;
use ooxml_drawingml::source::{self, GeometrySpec, StyleRef, Xfrm};
use ooxml_drawingml::text::{self as text, Autofit};
use ooxml_drawingml::tree::{self, Node};
use ooxml_opc::Package;
use ooxml_render::Painter;
use ooxml_schema::a;
use ooxml_word_model::theme::{FontScheme, ThemeFonts};
use ooxml_xml::view::{attr, child, elements, local};
use ooxml_xml::Element;

/// DrawingML's run size where neither the run nor its paragraph states one, pt.
const DEFAULT_TEXT_PT: f64 = 18.0;

/// The relationship from a data model to its cached drawing, in Microsoft's and the
/// transitional spelling.
const DRAWING_TYPES: [&str; 2] = [
    "http://schemas.microsoft.com/office/2007/relationships/diagramDrawing",
    "http://purl.oclc.org/ooxml/officeDocument/relationships/diagramDrawing",
];
/// The relationship from the part a frame sits in to a diagram's data model.
const DATA_TYPES: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData",
    "http://purl.oclc.org/ooxml/officeDocument/relationships/diagramData",
];

/// The target of `owner`'s relationship `id`.
fn related(package: &Package, owner: &str, id: &str) -> Option<String> {
    package.relationships(owner).ok()?.get(id)?.target_part.clone()
}

/// The targets of `owner`'s relationships of `kind`, in order.
fn related_of_type(package: &Package, owner: &str, kind: &str) -> Vec<String> {
    package
        .relationships(owner)
        .map(|rels| {
            rels.iter()
                .filter(|r| r.rel_type == kind)
                .filter_map(|r| r.target_part.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// `dsp:dataModelExt@relId` in the data model part, where it states one.
fn data_model_drawing_id(package: &Package, data_part: &str) -> Option<String> {
    fn find(e: &Element) -> Option<String> {
        if local(e) == "dataModelExt" {
            if let Some(id) = attr(e, "relId").filter(|i| !i.is_empty()) {
                return Some(id.to_owned());
            }
        }
        elements(e).find_map(find)
    }
    let tree = package.tree(data_part).ok().flatten()?;
    find(tree.root())
}

/// The cached drawing that belongs to the diagram whose data model is `data_part`, from
/// the part the frame sits in (ooxml-common's `diagram_drawing_part`): the data model's
/// `dsp:dataModelExt@relId` resolved against the owner, else the data model's own
/// relationship, else the owner's one diagram drawing no other data model names.
pub fn drawing_part(package: &Package, owner: &str, data_part: &str) -> Option<String> {
    if let Some(id) = data_model_drawing_id(package, data_part) {
        if let Some(target) = related(package, owner, &id).filter(|t| package.has_part(t)) {
            return Some(target);
        }
    }
    for kind in DRAWING_TYPES {
        if let Some(target) = related_of_type(package, data_part, kind).into_iter().next() {
            if package.has_part(&target) {
                return Some(target);
            }
        }
    }
    let candidates: Vec<String> = DRAWING_TYPES
        .iter()
        .flat_map(|kind| related_of_type(package, owner, kind))
        .filter(|t| package.has_part(t))
        .collect();
    let [only] = candidates.as_slice() else {
        return None;
    };
    let mut claimed = Vec::new();
    for kind in DATA_TYPES {
        for other in related_of_type(package, owner, kind) {
            if other == data_part || !package.has_part(&other) {
                continue;
            }
            if let Some(target) = data_model_drawing_id(package, &other).and_then(|id| related(package, owner, &id)) {
                claimed.push(target);
            }
            for drawing in DRAWING_TYPES {
                claimed.extend(related_of_type(package, &other, drawing));
            }
        }
    }
    (!claimed.contains(only)).then(|| only.clone())
}

/// What drawing a diagram reads: the document's colours and theme faces.
pub struct DiagramJob<'a> {
    /// The document's colours.
    pub colours: &'a WordColours<'a>,
    /// The theme's faces.
    pub fonts: Option<&'a FontScheme>,
}

/// The cached drawing of the SmartArt whose data model `relationship` names from `owner`,
/// laid in `at` and painted (`Frames.draw` for a diagram); `None` where it cannot be drawn
/// (warned).
pub fn draw(
    job: &DiagramJob<'_>,
    (package, owner, relationship): (&Package, &str, Option<&str>),
    (at, path): (&PxBox, &str),
    painter: &mut Painter<'_>,
    said: &mut Said,
) -> Option<Vec<Primitive>> {
    let data_part = relationship
        .and_then(|r| related(package, owner, r))
        .filter(|p| package.has_part(p));
    let Some(data_part) = data_part else {
        said.push("diagram-unreadable:no data model".into());
        return None;
    };
    let Some(drawing) = drawing_part(package, owner, &data_part) else {
        said.push("diagram-no-cached-drawing".into());
        return None;
    };
    let tree = package.tree(&drawing).ok().flatten();
    let Some(sp_tree) = tree.as_ref().and_then(|t| child(t.root(), "spTree")) else {
        said.push("diagram-unreadable:no shape tree".into());
        return None;
    };
    let emu = |v: ooxml_word_layout::px::Px| to_f64(v.value() * Q::from_integer(914_400) / Q::from_integer(300));
    let frame = Transform::of(0.0, 0.0, emu(at.width), emu(at.height));
    let mut fonts = Vec::new();
    font_refs(sp_tree, &mut fonts);
    let mut reading = Reading {
        job,
        fonts: fonts.into_iter(),
        said,
    };
    let children: Vec<Drawable> = shape_tree(Some(sp_tree))
        .iter()
        .filter_map(|node| reading.element(node))
        .collect();
    if children.is_empty() {
        reading.said.push("diagram-no-cached-drawing".into());
        return None;
    }
    let group = Drawable::Group(Box::new(Group {
        child_transform: child_transform(sp_tree, &frame),
        transform: frame,
        children,
        ..Group::default()
    }));
    Some(vec![super::chart::render(&group, at, path, painter)])
}

/// Each `sp`'s `dsp:style/a:fontRef@idx` under the tree, in document order: what the
/// shapes read take in turn, hidden ones too (`diagram_element`'s `fonts`).
fn font_refs(e: &Element, out: &mut Vec<Option<String>>) {
    if local(e) == "sp" {
        let idx = child(e, "style")
            .and_then(|s| child(s, "fontRef"))
            .and_then(|f| attr(f, "idx"));
        out.push(idx.map(str::to_owned));
    }
    for c in elements(e) {
        font_refs(c, out);
    }
}

/// The space the cached shapes were laid out in (`diagram_child_transform`): the tree's
/// own `chOff`/`chExt`, else the frame's extent from the origin.
fn child_transform(sp_tree: &Element, frame: &Transform) -> Transform {
    let view = child(sp_tree, "grpSpPr")
        .map(a::GroupProperties::new)
        .and_then(a::GroupProperties::xfrm);
    match group_xfrms(view).1 {
        Some(inner) if inner.width != 0.0 && inner.height != 0.0 => {
            Transform::of(inner.x, inner.y, inner.width, inner.height)
        }
        _ => Transform::of(0.0, 0.0, frame.extent_width, frame.extent_height),
    }
}

fn transform(xfrm: Option<&Xfrm>) -> Transform {
    xfrm.map_or_else(Transform::default, |x| Transform {
        offset_x: x.x,
        offset_y: x.y,
        extent_width: x.width,
        extent_height: x.height,
        rotation: x.rotation / 60000.0,
        flip_h: x.flip_h,
        flip_v: x.flip_v,
    })
}

fn geometry(geometry: Option<&source::Geometry>) -> Geometry {
    match geometry {
        None => Geometry::preset("rect"),
        Some(source::Geometry::Custom(paths)) => Geometry::Custom(
            paths
                .iter()
                .map(|p| drawable::CustomPath {
                    width: p.width,
                    height: p.height,
                    commands: p.commands.clone(),
                })
                .collect(),
        ),
        Some(source::Geometry::Preset { name, adjust }) => Geometry::Preset {
            name: name.clone(),
            adjust: adjust.clone(),
        },
    }
}

fn text_rect(rect: Option<&tree::TextRect>) -> Option<TextRect> {
    let rect = rect?;
    let Some(GeometrySpec::Custom {
        adjustments, guides, ..
    }) = &rect.geometry
    else {
        return None;
    };
    Some(TextRect {
        adjustments: adjustments.clone(),
        guides: guides.clone(),
        rect: rect.rect.clone(),
    })
}

/// One reading of a cached drawing's shapes.
struct Reading<'j, 's> {
    job: &'j DiagramJob<'j>,
    fonts: std::vec::IntoIter<Option<String>>,
    said: &'s mut Said,
}

impl Reading<'_, '_> {
    fn element(&mut self, node: &Node) -> Option<Drawable> {
        let hidden = match node {
            Node::Shape(s) => s.info.hidden,
            Node::Connector(c) => c.info.hidden,
            Node::Picture(p) => p.info.hidden,
            Node::Table(t) => t.info.hidden,
            Node::Group(g) => g.info.hidden,
            Node::Frame(f) => f.info.hidden,
        };
        if hidden {
            if matches!(node, Node::Shape(_)) {
                self.fonts.next();
            }
            return None;
        }
        match node {
            Node::Group(g) => {
                let children = g.children.iter().filter_map(|c| self.element(c)).collect();
                let own = transform(g.xfrm.as_ref());
                let inner = g
                    .child_xfrm
                    .as_ref()
                    .map_or_else(|| own, |c| Transform::of(c.x, c.y, c.width, c.height));
                // The group's effects are read (and said) as docx2svg reads them; a group
                // draws none.
                let _ = self.job.colours.effects(g.effects.as_ref(), None, self.said);
                Some(Drawable::Group(Box::new(Group {
                    transform: own,
                    child_transform: inner,
                    children,
                    ..Group::default()
                })))
            }
            Node::Connector(c) => Some(self.connector(c)),
            Node::Shape(s) => Some(self.shape(s)),
            Node::Picture(_) => self.not_drawn("image"),
            Node::Table(_) => self.not_drawn("table"),
            Node::Frame(_) => self.not_drawn("unsupported"),
        }
    }

    fn not_drawn(&mut self, kind: &str) -> Option<Drawable> {
        self.said.push(format!("diagram-not-drawn:{kind}"));
        None
    }

    fn connector(&mut self, c: &tree::Connector) -> Drawable {
        let style = c.style.as_ref();
        let colours = self.job.colours;
        let outline = colours.outline(c.outline.as_ref(), style.and_then(|s| s.line.as_ref()), self.said);
        let effects = colours.effects(c.effects.as_ref(), style.and_then(|s| s.effect.as_ref()), self.said);
        Drawable::Connector(Box::new(Connector {
            transform: transform(c.xfrm.as_ref()),
            geometry: geometry(c.geometry.as_ref()),
            outline,
            effects,
            ..Connector::default()
        }))
    }

    fn shape(&mut self, s: &tree::Shape) -> Drawable {
        let font_ref = self.fonts.next().flatten();
        let style = s.style.as_ref();
        let colours = self.job.colours;
        let fill = match &s.fill {
            Some(f) => colours.fill(f, None, None, self.said),
            None => colours.style_fill(style.and_then(|st| st.fill.as_ref()), self.said),
        };
        let outline = colours.outline(s.outline.as_ref(), style.and_then(|st| st.line.as_ref()), self.said);
        let effects = colours.effects(s.effects.as_ref(), style.and_then(|st| st.effect.as_ref()), self.said);
        let text = self.text_body(
            s.text.as_ref(),
            style.and_then(|st| st.font.as_ref()),
            font_ref.as_deref(),
        );
        Drawable::Shape(Box::new(Shape {
            transform: transform(s.xfrm.as_ref()),
            geometry: geometry(s.geometry.as_ref()),
            fill,
            outline,
            effects,
            text,
            text_transform: s.text_xfrm.as_ref().map(|x| transform(Some(x))),
            text_rect: text_rect(s.text_rect.as_ref()),
            ..Shape::default()
        }))
    }

    fn resolve(&mut self, colour: &SourceColour) -> Option<Colour> {
        self.job.colours.resolve(colour, None, self.said)
    }

    fn face(&self, typeface: Option<&str>) -> Option<String> {
        super::chart::theme_typeface(self.job.fonts, typeface)
    }

    /// A shape's text body (`_text_body`): `None` where no run holds text.
    fn text_body(
        &mut self,
        body: Option<&text::TextBody>,
        reference: Option<&StyleRef>,
        font_ref: Option<&str>,
    ) -> Option<TextBody> {
        let body = body.filter(|b| b.paragraphs.iter().any(|p| p.runs.iter().any(|r| !r.text.is_empty())))?;
        let theme: Option<&ThemeFonts> =
            self.job
                .fonts
                .map(|f| if font_ref == Some("major") { &f.major } else { &f.minor });
        let faces = Faces {
            latin: theme.map(|t| t.latin.to_string()).filter(|f| !f.is_empty()),
            east_asian: theme.map(|t| t.east_asian.to_string()).filter(|f| !f.is_empty()),
            colour: match reference.and_then(|r| r.colour.as_ref()) {
                Some(c) => self.resolve(c),
                None => None,
            },
        };
        let mut paragraphs = Vec::new();
        for paragraph in &body.paragraphs {
            let none = text::ParagraphProperties::default();
            let props = paragraph.properties.as_ref().unwrap_or(&none);
            let defaults = props.default_run.as_ref();
            let runs = paragraph
                .runs
                .iter()
                .map(|r| {
                    let merged = merge(defaults, r.properties.as_ref());
                    TextRun {
                        text: r.text.clone(),
                        properties: self.run(&merged, &faces),
                    }
                })
                .collect();
            let bullet = match &props.bullet {
                None | Some(text::Bullet::Picture(_)) => None,
                Some(text::Bullet::None) => Some(Bullet::None),
                Some(text::Bullet::Char(c)) => Some(Bullet::Char(c.clone())),
                Some(text::Bullet::AutoNumber { scheme, start }) => Some(Bullet::AutoNumber {
                    scheme: scheme.clone(),
                    start: *start,
                }),
            };
            let bullet_colour = match &props.bullet_colour {
                Some(c) => self.resolve(c),
                None => None,
            };
            let properties = ParagraphProperties {
                alignment: props.align.clone(),
                line_spacing: props.line_spacing,
                space_before: props.space_before.unwrap_or(text::Spacing::Percent(0.0)),
                space_after: props.space_after.unwrap_or(text::Spacing::Percent(0.0)),
                level: props.level.unwrap_or(0),
                bullet,
                bullet_font: self.face(props.bullet_font.as_deref()),
                bullet_colour,
                bullet_size_percent: props.bullet_size_percent,
                bullet_size_points: props.bullet_size_points,
                margin_left: props.margin_left,
                indent: props.indent,
                tab_stops: props.tabs.clone().unwrap_or_default(),
            };
            let end_properties = paragraph
                .end
                .as_ref()
                .map(|end| self.run(&merge(defaults, Some(end)), &faces));
            paragraphs.push(Paragraph {
                runs,
                properties,
                end_properties,
            });
        }
        Some(TextBody {
            paragraphs,
            properties: body_properties(body.properties.as_ref()),
        })
    }

    /// A run's properties as Word draws a diagram's (`_run`).
    fn run(&mut self, props: &text::RunProperties, faces: &Faces) -> drawable::RunProperties {
        let resolved = match &props.colour {
            Some(c) => self.resolve(c),
            None => faces.colour.clone(),
        };
        let colour = match resolved {
            Some(c) => Some(c),
            None => self.resolve(&SourceColour::Scheme {
                slot: "tx1".into(),
                transforms: Vec::new(),
            }),
        };
        drawable::RunProperties {
            font_size: Some(props.size.filter(|s| *s != 0.0).unwrap_or(DEFAULT_TEXT_PT)),
            font_family: self.face(props.latin.as_deref()).or_else(|| faces.latin.clone()),
            font_family_ea: self
                .face(props.east_asian.as_deref())
                .or_else(|| faces.east_asian.clone()),
            font_family_cs: self.face(props.complex_script.as_deref()),
            bold: props.bold == Some(true),
            italic: props.italic == Some(true),
            underline: props.underline == Some(true),
            underline_style: props.underline_style.clone(),
            strikethrough: props.strike == Some(true),
            baseline: props.baseline.unwrap_or(0.0),
            colour,
            ..drawable::RunProperties::default()
        }
    }
}

/// The faces and colour a diagram's text takes from its style's `a:fontRef`.
struct Faces {
    latin: Option<String>,
    east_asian: Option<String>,
    colour: Option<Colour>,
}

/// The run properties stated in `layers`, later over earlier (`_merge`).
fn merge(defaults: Option<&text::RunProperties>, own: Option<&text::RunProperties>) -> text::RunProperties {
    let mut out = text::RunProperties::default();
    for layer in [defaults, own].into_iter().flatten() {
        out.bold = layer.bold.or(out.bold);
        out.italic = layer.italic.or(out.italic);
        out.underline = layer.underline.or(out.underline);
        out.underline_style = layer.underline_style.clone().or(out.underline_style);
        out.strike = layer.strike.or(out.strike);
        out.baseline = layer.baseline.or(out.baseline);
        out.size = layer.size.or(out.size);
        out.latin = layer.latin.clone().or(out.latin);
        out.east_asian = layer.east_asian.clone().or(out.east_asian);
        out.complex_script = layer.complex_script.clone().or(out.complex_script);
        out.colour = layer.colour.clone().or(out.colour);
    }
    out
}

/// A body's stated properties over the defaults (`_body`).
fn body_properties(props: Option<&text::BodyProperties>) -> drawable::BodyProperties {
    let mut out = drawable::BodyProperties::default();
    let Some(p) = props else {
        return out;
    };
    let set = |slot: &mut f64, value: Option<f64>| {
        if let Some(v) = value {
            *slot = v;
        }
    };
    set(&mut out.margin_left, p.inset_left);
    set(&mut out.margin_right, p.inset_right);
    set(&mut out.margin_top, p.inset_top);
    set(&mut out.margin_bottom, p.inset_bottom);
    if let Some(anchor) = &p.anchor {
        out.anchor.clone_from(anchor);
    }
    if let Some(wrap) = &p.wrap {
        out.wrap.clone_from(wrap);
    }
    if let Some(fit) = p.autofit {
        out.auto_fit = match fit {
            Autofit::None => "noAutofit",
            Autofit::Normal => "normAutofit",
            Autofit::Shape => "spAutofit",
        }
        .to_owned();
    }
    set(&mut out.font_scale, p.font_scale);
    set(&mut out.line_reduction, p.line_reduction);
    if let Some(columns) = p.columns {
        out.columns = columns;
    }
    if let Some(vert) = &p.vert {
        out.vert.clone_from(vert);
    }
    set(&mut out.rotation, p.rotation.map(|r| r / 60000.0));
    set(&mut out.default_tab_size, p.tab_size);
    out
}
