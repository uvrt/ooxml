//! Owns: what a drawing paints, as docx2svg's `drawing.Primitive`s ([`Primitive`]), and
//! their markup as `svg._float` writes it, one line each: a path with its paint, an image
//! in its box (cropped as pptx2svg crops, turned by its transform), a placeholder of a box,
//! markup a renderer wrote whole (an arrowhead, a chart), and the groups that carry a
//! shape's effects.
//!
//! Must not: decide what is drawn (the painter has), or read a part other than through the
//! `Pictures` port.

use ooxml_base::Q;
use ooxml_scene::exact::{quote, write, Element, Value};
use ooxml_scene::markup::{Attr, Element as Markup, Tag};
use ooxml_word_layout::paint::svg::{number, Pictures};
use ooxml_word_layout::px::Px;

/// A box in device px, exact.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PxBox {
    /// Left.
    pub x: Px,
    /// Top.
    pub y: Px,
    /// Width.
    pub width: Px,
    /// Height.
    pub height: Px,
}

/// One thing a drawing paints, in page device px (`drawing.Primitive`).
#[derive(Debug, Clone, PartialEq)]
pub enum Primitive {
    /// Path data with its paint's attributes.
    Path {
        /// Its element's path.
        path: String,
        /// The path data.
        d: String,
        /// The paint, as attribute text.
        attrs: String,
        /// An SVG transform (rotation, flips).
        transform: Option<String>,
    },
    /// A picture part in a box.
    Image {
        /// Its element's path.
        path: String,
        /// The picture's relationship.
        relationship: Option<String>,
        /// Its box.
        at: PxBox,
        /// An SVG transform.
        transform: Option<String>,
        /// The fractions cut off its left, top, right and bottom.
        crop: Option<[Q; 4]>,
    },
    /// A placeholder of a box.
    Placeholder {
        /// Its element's path.
        path: String,
        /// Its box.
        at: PxBox,
        /// What it stands for.
        what: String,
    },
    /// Markup written whole.
    Markup {
        /// Its element's path.
        path: String,
        /// The markup.
        markup: String,
        /// An SVG transform.
        transform: Option<String>,
    },
    /// The start of a group carrying a shape's effects.
    Begin {
        /// Its element's path.
        path: String,
        /// The effects, as attribute text.
        attrs: String,
    },
    /// The end of that group.
    End,
}

impl Primitive {
    /// Its element's path, where it has one.
    pub fn path_mut(&mut self) -> Option<&mut String> {
        match self {
            Primitive::Path { path, .. }
            | Primitive::Image { path, .. }
            | Primitive::Placeholder { path, .. }
            | Primitive::Markup { path, .. }
            | Primitive::Begin { path, .. } => Some(path),
            Primitive::End => None,
        }
    }
}

/// Markup attributes as text, as ooxml-common writes them inside a tag.
pub fn attrs_text(attrs: &[Attr]) -> String {
    let mut element = Markup::empty(Tag::G);
    element.attrs = attrs.to_vec();
    let svg = element.to_svg();
    svg.strip_prefix("<g ")
        .and_then(|s| s.strip_suffix("/>"))
        .unwrap_or("")
        .to_owned()
}

fn n(value: Px) -> Value {
    Value::Number(number(value.value()))
}

fn q(value: Q) -> Value {
    Value::Number(number(value))
}

/// One element's text on its line.
fn line(element: &Element) -> String {
    let mut text = write(element);
    text.pop();
    text
}

/// A placeholder of a box (`_placeholder`).
pub fn placeholder(at: &PxBox, path: &str, what: &str) -> String {
    let rect = Element::new("rect")
        .attr("x", n(at.x))
        .attr("y", n(at.y))
        .attr("width", n(at.width))
        .attr("height", n(at.height))
        .attr("fill", Value::Plain("#F2F2F2".into()))
        .attr("stroke", Value::Plain("#9A9A9A".into()))
        .attr("stroke-width", Value::Plain("3".into()))
        .attr("stroke-dasharray", Value::Plain("12 8".into()));
    let mut g = Element::new("g")
        .inline()
        .attr("data-docx-unsupported", Value::Plain(what.into()))
        .attr("data-docx-path", Value::Text(path.into()));
    g.children.push(rect);
    line(&g)
}

const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/svg+xml"];

/// What an image needs beyond its primitive: the pictures and the part it is related from.
pub struct ImageSource<'a> {
    /// The pictures.
    pub pictures: &'a dyn Pictures,
    /// The part (a header's or footer's; `None`: the main document part).
    pub part: Option<&'a str>,
}

/// A picture in its box, or a placeholder (`_picture`); `None` beside the markup, or what
/// is said of a picture that cannot be carried.
fn image(
    (path, relationship, at): (&str, Option<&str>, &PxBox),
    transform: Option<&str>,
    crop: Option<&[Q; 4]>,
    source: &ImageSource<'_>,
) -> (String, Option<(String, String)>) {
    let found = relationship.and_then(|r| source.pictures.picture(r, source.part));
    let Some((mime, bytes)) = found.clone().filter(|(mime, _)| IMAGE_TYPES.contains(&mime.as_str())) else {
        let said = relationship.map(|_| {
            let what = found.map_or_else(|| "a missing part".to_owned(), |(mime, _)| mime);
            (
                "picture-not-drawn".to_owned(),
                format!("a picture in {what} is drawn as a placeholder of its extent ({path})"),
            )
        });
        let what = if relationship.is_none() {
            "drawing"
        } else {
            "picture format"
        };
        return (placeholder(at, path, what), said);
    };
    let href = Value::Data { mime, bytes };
    let Some([left, top, right, bottom]) = crop else {
        let mut e = Element::new("image")
            .attr("data-docx-path", Value::Text(path.into()))
            .attr("x", n(at.x))
            .attr("y", n(at.y))
            .attr("width", n(at.width))
            .attr("height", n(at.height))
            .attr("preserveAspectRatio", Value::Plain("none".into()));
        if let Some(t) = transform {
            e = e.attr("transform", Value::Plain(t.into()));
        }
        return (line(&e.attr("href", href)), None);
    };
    let floor = Q::new(1, 1_000_000);
    let kept_w = (Q::from_integer(1) - left - right).max(floor);
    let kept_h = (Q::from_integer(1) - top - bottom).max(floor);
    let (scaled_w, scaled_h) = (at.width.value() / kept_w, at.height.value() / kept_h);
    let inner = Element::new("image")
        .attr("x", q(-*left * scaled_w))
        .attr("y", q(-*top * scaled_h))
        .attr("width", q(scaled_w))
        .attr("height", q(scaled_h))
        .attr("preserveAspectRatio", Value::Plain("none".into()))
        .attr("href", href);
    let mut svg = Element::new("svg")
        .inline()
        .attr("data-docx-path", Value::Text(path.into()))
        .attr("x", n(at.x))
        .attr("y", n(at.y))
        .attr("width", n(at.width))
        .attr("height", n(at.height))
        .attr(
            "viewBox",
            Value::Plain(format!(
                "0 0 {} {}",
                number(at.width.value()),
                number(at.height.value())
            )),
        )
        .attr("overflow", Value::Plain("hidden".into()));
    svg.children.push(inner);
    let cropped = line(&svg);
    match transform {
        Some(t) => (format!("<g transform=\"{t}\">{cropped}</g>"), None),
        None => (cropped, None),
    }
}

/// A drawing's primitives as `_float` writes them, one line each; beside them, what is
/// said of pictures that could not be carried (`(code, message)`).
pub fn lines(primitives: &[Primitive], source: &ImageSource<'_>) -> (Vec<String>, Vec<(String, String)>) {
    let mut out = Vec::with_capacity(primitives.len());
    let mut said = Vec::new();
    let with = |t: &Option<String>| t.as_ref().map(|t| format!(" transform=\"{t}\"")).unwrap_or_default();
    for p in primitives {
        out.push(match p {
            Primitive::Path {
                path,
                d,
                attrs,
                transform,
            } => format!(
                "<path data-docx-path={} d={} {attrs}{}/>",
                quote(path),
                quote(d),
                with(transform)
            ),
            Primitive::Markup {
                path,
                markup,
                transform,
            } => format!("<g data-docx-path={}{}>{markup}</g>", quote(path), with(transform)),
            Primitive::Begin { path, attrs } => format!("<g data-docx-path={} {attrs}>", quote(path)),
            Primitive::End => "</g>".to_owned(),
            Primitive::Image {
                path,
                relationship,
                at,
                transform,
                crop,
            } => {
                let (text, warned) = image(
                    (path, relationship.as_deref(), at),
                    transform.as_deref(),
                    crop.as_ref(),
                    source,
                );
                said.extend(warned);
                text
            }
            Primitive::Placeholder { path, at, what } => placeholder(at, path, what),
        });
    }
    (out, said)
}
