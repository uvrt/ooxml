//! Owns: a page's paint as `ooxml-scene`'s exact display list, written as docx2svg's
//! `svg.render_page` writes it (byte for byte): the page's size in points and device px,
//! its white ground, the rules text sits on (in `_UNDER_TEXT`'s order), the lines grouped
//! by paragraph with their live-view ids (`data-docx-path`, `data-docx-id`,
//! `data-docx-line`, `data-docx-baseline`), each span a `<text>` with one x per glyph, the
//! rules drawn over text, and the pictures -- each from the [`Pictures`] port, or a
//! placeholder of its extent where the port has none.
//!
//! Must not: lay out or paint anything (the paint has), or read a picture part.

use super::{DrawingName, Drawn, FaceNames, GlyphSize, PagePaint, PaintedLine, Rule, Span, SpanKind};
use crate::px::Px;
use ooxml_base::Q;
use ooxml_scene::exact::{Decimal4, Element, Value};
use std::sync::Arc;

/// Where the SVG reads a picture's bytes: the port to the document's parts. It answers
/// with what an SVG carries (`image/png`, `image/jpeg`, `image/gif`, `image/svg+xml` -- a
/// metafile already drawn as SVG), or `None`.
pub trait Pictures {
    /// The picture of `relationship` in `part` (`None`: the main document part).
    fn picture(&self, relationship: &str, part: Option<&str>) -> Option<(String, Arc<[u8]>)>;
}

/// No pictures: each is drawn as a placeholder of its extent.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPictures;

impl Pictures for NoPictures {
    fn picture(&self, _relationship: &str, _part: Option<&str>) -> Option<(String, Arc<[u8]>)> {
        None
    }
}

/// What writing a page reads beyond its paint.
pub struct SvgContext<'a> {
    /// How faces are named.
    pub names: &'a dyn FaceNames,
    /// The pictures.
    pub pictures: &'a dyn Pictures,
    /// The glyph size.
    pub glyph_size: GlyphSize,
}

/// An exact length to four decimals, half up (`svg.number`).
pub fn number(value: Q) -> Decimal4 {
    let (n, d) = (*value.numer(), *value.denom());
    let whole = num_integer::Integer::div_floor(&(n * 20_000 + d), &(2 * d));
    Decimal4(i64::try_from(whole).unwrap_or(0))
}

fn px(value: Px) -> Value {
    Value::Number(number(value.value()))
}

fn colour(value: Option<&str>) -> Value {
    Value::Plain(format!("#{}", value.unwrap_or("000000").to_ascii_uppercase()))
}

/// The font size in device px: the size rounded to whole px, or exact.
fn font_size(half_points: i64, glyph_size: GlyphSize) -> Q {
    let exact = Px::points(Q::new(i128::from(half_points), 2));
    match glyph_size {
        GlyphSize::Exact => exact.value(),
        GlyphSize::Device => Q::from_integer(i128::from(exact.round_half_up())),
    }
}

/// A family name as a CSS family list keeps it: plain words as they are, any other quoted.
fn css_family(name: &str) -> String {
    let plain = name.split(' ').all(|word| {
        let mut chars = word.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    });
    if plain {
        name.to_owned()
    } else {
        format!("'{}'", name.replace('\\', "\\\\").replace('\'', "\\'"))
    }
}

/// The `font-*` attributes naming a span's face (`_font`).
fn font(span: &Span, name: Option<DrawingName>) -> Vec<(&'static str, Value)> {
    let Some(name) = name else {
        let mut out = vec![("font-family", Value::Text(span.face.to_string()))];
        if span.bold {
            out.push(("font-weight", Value::Plain("bold".into())));
        }
        if span.italic {
            out.push(("font-style", Value::Plain("italic".into())));
        }
        return out;
    };
    let mut families = vec![span.face.to_string()];
    if name.family.to_lowercase() != span.face.to_lowercase() {
        families.push(name.family.clone());
    }
    let list: Vec<String> = families.iter().map(|f| css_family(f)).collect();
    let mut out = vec![("font-family", Value::Text(list.join(", ")))];
    if name.weight != 400 {
        out.push(("font-weight", Value::Plain(name.weight.to_string())));
    }
    if name.stretch != "normal" {
        out.push(("font-stretch", Value::Plain(name.stretch.clone())));
    }
    if name.style != "normal" {
        out.push(("font-style", Value::Plain(name.style.clone())));
    }
    out
}

fn text(span: &Span, cx: &SvgContext<'_>) -> Element {
    let chars: String = span.chars.iter().collect();
    let attrs = font(span, cx.names.drawing_name(&span.face, span.bold, span.italic));
    let mut e = Element::new("text")
        .attr("data-docx-path", Value::Text(span.path.clone()))
        .attr("x", Value::Numbers(span.xs.iter().map(|x| number(x.value())).collect()))
        .attr("y", Value::Int(span.y));
    let mut attrs = attrs.into_iter();
    if let Some((name, value)) = attrs.next() {
        e = e.attr(name, value);
    }
    e = e.attr(
        "font-size",
        Value::Number(number(font_size(span.half_points, cx.glyph_size))),
    );
    for (name, value) in attrs {
        e = e.attr(name, value);
    }
    e = e.attr("fill", colour(span.color.as_deref()));
    if span.kind != SpanKind::Text {
        e = e.attr("data-docx-kind", Value::Plain(span.kind.name().into()));
    }
    let trimmed = chars.trim();
    if trimmed.len() != chars.len() || chars.contains("  ") {
        e = e.attr("xml:space", Value::Plain("preserve".into()));
    }
    e.content(chars)
}

fn rect(rule: &Rule) -> Element {
    Element::new("rect")
        .attr("x", px(rule.x))
        .attr("y", px(rule.y))
        .attr("width", px(rule.width))
        .attr("height", px(rule.height))
        .attr("fill", colour(rule.color.as_deref()))
        .attr("data-docx-path", Value::Text(rule.path.clone()))
        .attr("data-docx-kind", Value::Plain(rule.kind.into()))
}

/// A rule's place in what is drawn under the text, `None` for one drawn over it.
fn under_rank(kind: &str) -> Option<u8> {
    match kind {
        "table-border" => Some(0),
        "cell-shading" => Some(1),
        "paragraph-shading" => Some(2),
        "shading" | "highlight" => Some(3),
        _ => None,
    }
}

/// Lines of text, grouped by paragraph.
fn lines(painted: &[PaintedLine], cx: &SvgContext<'_>) -> Vec<Element> {
    let mut out: Vec<Element> = Vec::new();
    let mut current: Option<&str> = None;
    for line in painted {
        if current != Some(&*line.path) {
            let mut group = Element::new("g").attr("data-docx-path", Value::Text(line.path.to_string()));
            if let Some(id) = line.paragraph_id.as_deref().filter(|i| !i.is_empty()) {
                group = group.attr("data-docx-id", Value::Text(id.to_owned()));
            }
            out.push(group);
            current = Some(&line.path);
        }
        let mut g = Element::new("g")
            .attr("data-docx-line", Value::Int(i64::try_from(line.number).unwrap_or(0)))
            .attr("data-docx-baseline", Value::Int(line.baseline));
        g.children = line
            .spans
            .iter()
            .filter(|s| !s.chars.is_empty())
            .map(|s| text(s, cx))
            .collect();
        if let Some(group) = out.last_mut() {
            group.children.push(g);
        }
    }
    out
}

/// A placeholder of a drawing's extent.
fn placeholder(d: &Drawn, what: &str) -> Element {
    Element::new("g")
        .inline()
        .attr("data-docx-unsupported", Value::Plain(what.into()))
        .attr("data-docx-path", Value::Text(d.path.clone()))
        .with_child(
            Element::new("rect")
                .attr("x", px(d.x))
                .attr("y", px(d.y))
                .attr("width", px(d.width))
                .attr("height", px(d.height))
                .attr("fill", Value::Plain("#F2F2F2".into()))
                .attr("stroke", Value::Plain("#9A9A9A".into()))
                .attr("stroke-width", Value::Plain("3".into()))
                .attr("stroke-dasharray", Value::Plain("12 8".into())),
        )
}

const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/svg+xml"];

/// A picture in its box, or a placeholder of the box where it cannot be carried.
fn picture(d: &Drawn, cx: &SvgContext<'_>) -> Element {
    let found = d
        .relationship
        .as_deref()
        .and_then(|r| cx.pictures.picture(r, d.part.as_deref()))
        .filter(|(mime, _)| IMAGE_TYPES.contains(&mime.as_str()));
    let Some((mime, bytes)) = found else {
        let what = if d.relationship.is_some() {
            "picture format"
        } else {
            "drawing"
        };
        return placeholder(d, what);
    };
    Element::new("image")
        .attr("data-docx-path", Value::Text(d.path.clone()))
        .attr("x", px(d.x))
        .attr("y", px(d.y))
        .attr("width", px(d.width))
        .attr("height", px(d.height))
        .attr("preserveAspectRatio", Value::Plain("none".into()))
        .attr("href", Value::Data { mime, bytes })
}

trait WithChild {
    fn with_child(self, child: Element) -> Element;
}

impl WithChild for Element {
    fn with_child(mut self, child: Element) -> Element {
        self.children.push(child);
        self
    }
}

/// The page as an SVG document's root element.
pub fn page_element(paint: &PagePaint, cx: &SvgContext<'_>) -> Element {
    let frame = &paint.geometry.frame;
    let mut root = Element::new("svg")
        .attr("xmlns", Value::Plain("http://www.w3.org/2000/svg".into()))
        .attr("version", Value::Plain("1.1".into()))
        .attr("width", Value::Plain(format!("{}pt", number(frame.width_pt))))
        .attr("height", Value::Plain(format!("{}pt", number(frame.height_pt))))
        .attr(
            "viewBox",
            Value::Plain(format!("0 0 {} {}", frame.width_px, frame.height_px)),
        )
        .attr(
            "data-docx-page",
            Value::Int(i64::try_from(paint.geometry.page + 1).unwrap_or(0)),
        );
    root.children.push(
        Element::new("rect")
            .attr("width", Value::Int(frame.width_px))
            .attr("height", Value::Int(frame.height_px))
            .attr("fill", Value::Plain("#FFFFFF".into())),
    );
    let mut under: Vec<&Rule> = paint.rules.iter().filter(|r| under_rank(r.kind).is_some()).collect();
    under.sort_by_key(|r| under_rank(r.kind));
    root.children.extend(under.into_iter().map(rect));
    root.children.extend(lines(&paint.lines, cx));
    root.children
        .extend(paint.rules.iter().filter(|r| under_rank(r.kind).is_none()).map(rect));
    root.children.extend(paint.pictures.iter().map(|d| picture(d, cx)));
    root
}

/// The page as SVG text.
pub fn page_svg(paint: &PagePaint, cx: &SvgContext<'_>) -> String {
    ooxml_scene::exact::write(&page_element(paint, cx))
}
