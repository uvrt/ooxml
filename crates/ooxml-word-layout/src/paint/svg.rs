//! Owns: a page's paint as `ooxml-scene`'s exact display list, written as docx2svg's
//! `svg.render_page` writes it (byte for byte): the page's size in points and device px,
//! its white ground, the rules text sits on (in `_UNDER_TEXT`'s order), the lines grouped
//! by paragraph with their live-view ids (`data-docx-path`, `data-docx-id`,
//! `data-docx-line`, `data-docx-baseline`), each span a `<text>` with one x per glyph, the
//! rules drawn over text, and the pictures -- each from the [`Pictures`] port, or a
//! stand-in box of its extent where the port has none.
//!
//! Must not: lay out or paint anything (the paint has), or read a picture part.

use super::drawings::{DrawingSlot, Layer};
use super::names::{Namer, PageNames};
use super::text_box::TextBoxPaint;
use super::{DrawingName, Drawn, FaceNames, GlyphSize, PagePaint, PaintedLine, Rule, Span, SpanKind, TextEffects};
use crate::px::Px;
use ooxml_base::Q;
use ooxml_scene::exact::{Decimal4, Element, Value};
use ooxml_word_model::docx::VersionNames;
use std::sync::Arc;

/// Where the SVG reads a picture's bytes: the port to the document's parts. It answers
/// with what an SVG carries (`image/png`, `image/jpeg`, `image/gif`, `image/svg+xml` -- a
/// metafile already drawn as SVG), or `None`.
pub trait Pictures {
    /// The picture of `relationship` in `part` (`None`: the main document part).
    fn picture(&self, relationship: &str, part: Option<&str>) -> Option<(String, Arc<[u8]>)>;
}

/// No pictures: each is drawn as a stand-in box of its extent.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPictures;

impl Pictures for NoPictures {
    fn picture(&self, _relationship: &str, _part: Option<&str>) -> Option<(String, Arc<[u8]>)> {
        None
    }
}

/// The resources a page's drawings declare -- gradients, patterns, markers, filters, each
/// a line of markup -- and the count their ids are minted from: one per page, shared by
/// its drawings in the order they were placed (docx2svg's page `Defs`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageDefs {
    /// The resources, in order.
    pub defs: Vec<String>,
    /// The ids minted so far.
    pub ids: u32,
}

/// Where the SVG reads a drawing's paint: the port to the painters of DrawingML, charts and
/// SmartArt. It answers with the drawing's group's content in its box -- its primitives,
/// its text boxes' lines -- each a line of markup, its resources added to `defs`, or `None`
/// where it has no painter.
pub trait Drawings {
    /// The paint of the drawing in `slot`.
    fn drawing(&self, slot: &DrawingSlot, defs: &mut PageDefs) -> Option<Vec<String>>;

    /// The paint of glyphs drawn with Word 2010 text effects in `bounds` (`x`, `y`, width,
    /// height, px), their colour `colour` where the effects state no fill: its attributes
    /// as written (`fill="..." stroke="..."`), resources added to `defs` (docx2svg's
    /// `svg._paint`); `None` where there is no painter (the glyphs are drawn in their
    /// colour).
    fn text_paint(
        &self,
        effects: &TextEffects,
        bounds: (f64, f64, f64, f64),
        colour: Option<&str>,
        defs: &mut PageDefs,
    ) -> Option<String> {
        let _ = (effects, bounds, colour, defs);
        None
    }
}

/// No painters: no drawing is drawn.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDrawings;

impl Drawings for NoDrawings {
    fn drawing(&self, _slot: &DrawingSlot, _defs: &mut PageDefs) -> Option<Vec<String>> {
        None
    }
}

/// What writing a page reads beyond its paint.
pub struct SvgContext<'a> {
    /// The drawings' painters.
    pub drawings: &'a dyn Drawings,
    /// How faces are named.
    pub names: &'a dyn FaceNames,
    /// The pictures.
    pub pictures: &'a dyn Pictures,
    /// The glyph size.
    pub glyph_size: GlyphSize,
    /// The version's names: what its items are written under (`data-docx-path`).
    pub version: &'a VersionNames,
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
        out.push(("font-stretch", Value::Plain(name.stretch.to_owned())));
    }
    if name.style != "normal" {
        out.push(("font-style", Value::Plain(name.style.to_owned())));
    }
    out
}

fn text(span: &Span, cx: &SvgContext<'_>, defs: &mut PageDefs, names: &dyn Namer) -> Element {
    let chars: String = span.chars.iter().collect();
    let attrs = font(span, cx.names.drawing_name(&span.face, span.bold, span.italic));
    let mut e = Element::new("text")
        .attr("data-docx-path", Value::Text(names.written(span.anchor, &span.path)))
        .attr("x", Value::Numbers(span.pens().map(|x| number(x.value())).collect()))
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
    e = match span
        .effects
        .as_ref()
        .and_then(|effects| effects_paint(span, effects, cx, defs))
    {
        Some(paint) => e.attr("fill", Value::Attributes(paint)),
        None => e.attr("fill", colour(span.color.as_deref())),
    };
    if span.kind != SpanKind::Text {
        e = e.attr("data-docx-kind", Value::Plain(span.kind.name().into()));
    }
    let trimmed = chars.trim();
    if trimmed.len() != chars.len() || chars.contains("  ") {
        e = e.attr("xml:space", Value::Plain("preserve".into()));
    }
    e.content(chars)
}

/// A span's text effects as paint, the box their gradient spans its text (`_paint`): from
/// its first glyph to its end, an em tall with its baseline four fifths down.
fn effects_paint(span: &Span, effects: &TextEffects, cx: &SvgContext<'_>, defs: &mut PageDefs) -> Option<String> {
    let size = font_size(span.half_points, cx.glyph_size);
    let float = |q: Q| {
        #[allow(clippy::cast_precision_loss)]
        let out = *q.numer() as f64 / *q.denom() as f64;
        out
    };
    let x0 = span.x(0).unwrap_or(span.end).value();
    let top = Q::from_integer(i128::from(span.y)) - size * Q::new(4, 5);
    let bounds = (float(x0), float(top), float(span.end.value() - x0), float(size));
    cx.drawings.text_paint(effects, bounds, span.color.as_deref(), defs)
}

fn rect(rule: &Rule, names: &dyn Namer) -> Element {
    Element::new("rect")
        .attr("x", px(rule.x))
        .attr("y", px(rule.y))
        .attr("width", px(rule.width))
        .attr("height", px(rule.height))
        .attr("fill", colour(rule.color.as_deref()))
        .attr("data-docx-path", Value::Text(names.written(rule.anchor, &rule.path)))
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
fn lines(painted: &[PaintedLine], cx: &SvgContext<'_>, defs: &mut PageDefs, names: &dyn Namer) -> Vec<Element> {
    let mut out: Vec<Element> = Vec::new();
    let mut current: Option<(&str, Option<&(&'static str, ooxml_word_model::value::Text)>)> = None;
    for line in painted {
        let key = (&*line.path, line.story.as_ref());
        if current != Some(key) {
            let mut group = Element::new("g")
                .paired()
                .attr("data-docx-path", Value::Text(names.written(line.anchor, &line.path)));
            if let Some(id) = line.paragraph_id.as_deref().filter(|i| !i.is_empty()) {
                group = group.attr("data-docx-id", Value::Text(id.to_owned()));
            }
            if let Some((kind, part)) = &line.story {
                group = group
                    .attr("data-docx-story", Value::Plain((*kind).to_owned()))
                    .attr("data-docx-part", Value::Text(part.to_string()));
            }
            out.push(group);
            current = Some(key);
        }
        let mut g = Element::new("g")
            .paired()
            .attr("data-docx-line", Value::Int(i64::try_from(line.number).unwrap_or(0)))
            .attr("data-docx-baseline", Value::Int(line.baseline));
        g.children = line
            .spans
            .iter()
            .filter(|s| !s.chars.is_empty())
            .map(|s| text(s, cx, defs, names))
            .collect();
        if let Some(group) = out.last_mut() {
            group.children.push(g);
        }
    }
    out
}

/// A stand-in box of a drawing's extent.
fn stand_in(d: &Drawn, what: &str, names: &dyn Namer) -> Element {
    Element::new("g")
        .inline()
        .attr("data-docx-unsupported", Value::Plain(what.into()))
        .attr("data-docx-path", Value::Text(names.written(d.anchor, &d.path)))
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

/// A picture in its box, or a stand-in box where it cannot be carried.
fn picture(d: &Drawn, cx: &SvgContext<'_>, names: &dyn Namer) -> Element {
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
        return stand_in(d, what, names);
    };
    Element::new("image")
        .attr("data-docx-path", Value::Text(names.written(d.anchor, &d.path)))
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
    let text_lines = paint.text_boxes.iter().flat_map(|t| t.lines.iter());
    let names = PageNames::new(cx.version, paint.lines.iter().chain(text_lines));
    let names = &names;
    // Painted in the order they were placed, which is the order their resources' ids are
    // minted in; written by layer.
    let mut defs = PageDefs::default();
    let painted = painted_slots(paint, cx, names, &mut defs);
    let mut slots: Vec<&Painted<'_>> = painted.iter().collect();
    slots.sort_by_key(|p| p.slot.order);
    let layer = |l: Layer, story: bool| -> Vec<&Painted<'_>> {
        slots
            .iter()
            .copied()
            .filter(|p| p.slot.layer == l && p.slot.story.is_some() == story)
            .collect()
    };
    let (story_behind, story_front) = (layer(Layer::Behind, true), layer(Layer::Front, true));
    let (body_behind, body_front) = (layer(Layer::Behind, false), layer(Layer::Front, false));
    let story_lines = paint.lines.iter().any(|l| l.story.is_some());
    let pictures = paint.pictures.iter().map(|d| picture(d, cx, names));
    let d = &mut defs;
    if story_behind.is_empty() && story_front.is_empty() && !(story_lines && !body_behind.is_empty()) {
        root.children.extend(floats(&body_behind, cx, d, names));
        root.children.extend(under_rules(&paint.rules, names));
        root.children.extend(lines(&paint.lines, cx, d, names));
        root.children.extend(over_rules(&paint.rules, names));
        root.children.extend(pictures);
        root.children.extend(floats(&body_front, cx, d, names));
    } else {
        let (stories, body): (Vec<PaintedLine>, Vec<PaintedLine>) =
            paint.lines.iter().cloned().partition(|l| l.story.is_some());
        root.children.extend(floats(&story_behind, cx, d, names));
        root.children.extend(lines(&stories, cx, d, names));
        root.children.extend(floats(&story_front, cx, d, names));
        root.children.extend(floats(&body_behind, cx, d, names));
        root.children.extend(under_rules(&paint.rules, names));
        root.children.extend(lines(&body, cx, d, names));
        root.children.extend(over_rules(&paint.rules, names));
        root.children.extend(pictures);
        root.children.extend(floats(&body_front, cx, d, names));
    }
    if !defs.defs.is_empty() {
        // After the page's white ground, as docx2svg puts them once everything is drawn.
        root.children
            .insert(1, Element::raw(format!("<defs>{}</defs>", defs.defs.concat())));
    }
    root
}

/// The page's drawings painted in the order they were placed, which is the order their
/// resources' ids are minted in; each handed its painter named at the edge.
fn painted_slots<'p>(
    paint: &'p PagePaint,
    cx: &SvgContext<'_>,
    names: &PageNames<'_>,
    defs: &mut PageDefs,
) -> Vec<Painted<'p>> {
    paint
        .drawings
        .iter()
        .enumerate()
        .map(|(k, slot)| {
            // A text box's inline drawings are painted after its own (docx2svg draws them as
            // it lays the box out), their resources' ids minted in that order.
            let drawn = cx.drawings.drawing(&names.named_slot(slot), defs);
            let text: Vec<&TextBoxPaint> = paint.text_boxes.iter().filter(|t| t.slot == k).collect();
            let boxed = names.boxed(slot);
            let inner = text
                .iter()
                .flat_map(|t| t.drawings.iter())
                .map(|inner| cx.drawings.drawing(&boxed.named_slot(inner), defs))
                .collect();
            Painted {
                slot,
                paint: drawn,
                text,
                inner,
            }
        })
        .collect()
}

/// The rules drawn under the text, in their kinds' order (`_rules`).
fn under_rules(rules: &[Rule], names: &dyn Namer) -> Vec<Element> {
    let mut under: Vec<&Rule> = rules.iter().filter(|r| under_rank(r.kind).is_some()).collect();
    under.sort_by_key(|r| under_rank(r.kind));
    under.into_iter().map(|r| rect(r, names)).collect()
}

/// The rules drawn over the text, in the order made.
fn over_rules(rules: &[Rule], names: &dyn Namer) -> Vec<Element> {
    rules
        .iter()
        .filter(|r| under_rank(r.kind).is_none())
        .map(|r| rect(r, names))
        .collect()
}

/// A drawing's slot, its painter's paint, its text boxes' paint, and the paint of their
/// inline drawings.
struct Painted<'a> {
    slot: &'a DrawingSlot,
    paint: Option<Vec<String>>,
    text: Vec<&'a TextBoxPaint>,
    inner: Vec<Option<Vec<String>>>,
}

/// Drawings as groups of their painters' paint, then their text boxes' pictures, their
/// inline drawings' paint, rules and lines (`_float`); one with no painter is not written.
fn floats(slots: &[&Painted<'_>], cx: &SvgContext<'_>, defs: &mut PageDefs, names: &PageNames<'_>) -> Vec<Element> {
    let mut out = Vec::new();
    for painted in slots {
        let (slot, Some(paint)) = (painted.slot, &painted.paint) else {
            continue;
        };
        let side = if slot.layer == Layer::Behind { "behind" } else { "front" };
        let mut g = Element::new("g")
            .paired()
            .attr("data-docx-path", Value::Text(names.slot(slot)))
            .attr("data-docx-float", Value::Plain(side.to_owned()));
        if let Some((kind, part)) = &slot.story {
            g = g
                .attr("data-docx-story", Value::Plain((*kind).to_owned()))
                .attr("data-docx-part", Value::Text(part.to_string()));
        }
        g.children = paint.iter().cloned().map(Element::raw).collect();
        // a text box's items are named below its drawing's name
        let boxed = names.boxed(slot);
        let boxed: &dyn Namer = &boxed;
        for text in &painted.text {
            g.children.extend(text.pictures.iter().map(|d| picture(d, cx, boxed)));
        }
        for inner in painted.inner.iter().flatten() {
            g.children.extend(inner.iter().cloned().map(Element::raw));
        }
        let rules: Vec<Rule> = painted.text.iter().flat_map(|t| t.rules.iter().cloned()).collect();
        let text_lines: Vec<PaintedLine> = painted.text.iter().flat_map(|t| t.lines.iter().cloned()).collect();
        g.children.extend(under_rules(&rules, boxed));
        g.children.extend(lines(&text_lines, cx, defs, boxed));
        g.children.extend(over_rules(&rules, boxed));
        out.push(g);
    }
    out
}

/// The page as SVG text.
pub fn page_svg(paint: &PagePaint, cx: &SvgContext<'_>) -> String {
    ooxml_scene::exact::write(&page_element(paint, cx))
}
