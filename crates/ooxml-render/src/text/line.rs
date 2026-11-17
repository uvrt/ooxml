//! Owns: one laid-out line as text spans (ooxml-common's `textbody._render_line` and what
//! it calls): a run's spans split by script with their faces and styles, chunks started at
//! tab stops and at a change to a face that may not draw the run, italic runs in a face
//! with no italic sheared out of the flow, hyperlinks, and the line's advance kept on
//! whichever span comes first; a line's highlights.
//!
//! Must not: decide where a line breaks or how tall it is.

use super::wrap::{faces, Segment};
use crate::context::Painter;
use crate::measure::{is_cjk, PX_PER_PT};
use crate::rules::SyntheticBold;
use crate::units::emu_to_px;
use ooxml_drawingml::drawable::{Colour, RunProperties};
use ooxml_drawingml::text::TabStop;
use ooxml_fonts::symbols::{symbol_face, translate, FALLBACK_FAMILIES};
use ooxml_fonts::{font_family_value, office_metrics_for};
use ooxml_scene::markup::{Attr, Element, Op, Sep, Tag, Value};

/// The shear PowerPoint gives an italic it has to fake, `dx/dy`.
const SYNTHETIC_OBLIQUE_SHEAR: f64 = 0.33984;
/// PowerPoint's synthetic bold line: this many points plus a share of the size.
const SYNTHETIC_BOLD_STROKE_PT: f64 = 0.12;
const SYNTHETIC_BOLD_STROKE_RATIO: f64 = 0.02;

/// A line's advance as written: `0` on the first line, else two decimals.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dy {
    /// `0`.
    Zero,
    /// A step, written with two decimals.
    Step(f64),
}

impl Dy {
    /// The value written, read back.
    pub fn value(self) -> f64 {
        match self {
            Dy::Zero => 0.0,
            Dy::Step(v) => format!("{v:.2}").parse().unwrap_or(0.0),
        }
    }

    fn attr(self) -> Attr {
        let value = match self {
            Dy::Zero => Value::Word("0".into()),
            Dy::Step(v) => Value::Fixed2(v),
        };
        Attr { name: "dy", value }
    }
}

/// `x`, the advance where it is owed, and `text-anchor`: a span that starts a chunk.
pub fn leading(x: f64, dy: Option<Dy>, anchor: &str) -> Vec<Attr> {
    let mut out = vec![Attr {
        name: "x",
        value: Value::Num(x),
    }];
    out.extend(dy.map(Dy::attr));
    out.push(Attr {
        name: "text-anchor",
        value: Value::Word(anchor.to_owned()),
    });
    out
}

fn tspan(prefix: Vec<Attr>, styles: &[Attr], text: &str) -> Element {
    let mut e = Element::open(Tag::Tspan);
    e.attrs = prefix;
    e.attrs.extend(styles.iter().cloned());
    e.push_text(text);
    e
}

/// What a run must share with the run before it to stay in its chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Key {
    /// Nothing open yet.
    Nothing,
    /// Plain Latin text, whatever its face.
    Latin,
    /// A face (perhaps none).
    Face(Option<String>),
}

fn plain_latin(ch: char) -> bool {
    let c = u32::from(ch);
    c < 0x0370 || (0x1E00..=0x1EFF).contains(&c) || (0x2000..=0x206F).contains(&c) || (0x20A0..=0x20CF).contains(&c)
}

fn chunk_key(family: Option<&String>, text: &str) -> Key {
    if text.chars().all(plain_latin) {
        Key::Latin
    } else {
        Key::Face(family.cloned())
    }
}

/// One span of a run: its face stack, styles, text, and whether it is sheared.
struct Span {
    family: Option<String>,
    styles: Vec<Attr>,
    text: String,
    oblique: bool,
}

fn needs_script_split(p: &RunProperties) -> bool {
    matches!((&p.font_family, &p.font_family_ea), (Some(a), Some(b)) if a != b)
}

fn split_by_script(text: &str) -> Vec<(String, bool)> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut kind: Option<bool> = None;
    for ch in text.chars() {
        let ea = is_cjk(ch);
        if kind.is_some_and(|k| k != ea) {
            parts.push((std::mem::take(&mut current), kind.unwrap_or(false)));
        }
        kind = Some(ea);
        current.push(ch);
    }
    if let (false, Some(k)) = (current.is_empty(), kind) {
        parts.push((current, k));
    }
    parts
}

fn has_no_bold_cut(family: Option<&str>) -> bool {
    family
        .and_then(office_metrics_for)
        .is_some_and(|m| m.has_bold_widths() && m.bold_is_indistinguishable())
}

fn slants_italic(family: Option<&str>) -> bool {
    family
        .and_then(ooxml_fonts::substitutes::substitution_for)
        .is_some_and(|row| !row.has_italic_cut)
}

fn underline_style(style: Option<&str>) -> Option<&'static str> {
    Some(match style? {
        "dbl" | "wavyDbl" => "double",
        "wavy" | "wavyHeavy" => "wavy",
        "dotted" | "dottedHeavy" | "dotDash" | "dotDashHeavy" | "dotDotDash" | "dotDotDashHeavy" => "dotted",
        "dash" | "dashHeavy" | "dashLong" | "dashLongHeavy" => "dashed",
        _ => return None,
    })
}

fn word(name: &'static str, value: &str) -> Attr {
    Attr {
        name,
        value: Value::Word(value.to_owned()),
    }
}

fn num(name: &'static str, value: f64) -> Attr {
    Attr {
        name,
        value: Value::Num(value),
    }
}

fn colour_attrs(out: &mut Vec<Attr>, name: &'static str, opacity: &'static str, c: &Colour) {
    out.push(word(name, &c.hex));
    if c.alpha < 1.0 {
        out.push(num(opacity, c.alpha));
    }
}

/// A run's style attributes: size, faces, weight, slant, colour, decoration, baseline and
/// outline.
fn style_attrs(
    p: &RunProperties,
    scale: f64,
    chain: &[Option<&str>],
    default: f64,
    east_asian: bool,
    painter: &Painter<'_>,
) -> Vec<Attr> {
    let mut styles = Vec::new();
    let size = super::metrics::size_or(p, default);
    if size != 0.0 {
        let mut drawn = size * scale;
        if let Some(grid) = painter.rules.text_size_grid {
            let grid = f64::from(grid);
            drawn = (drawn * grid / 72.0).round_ties_even().max(1.0) * 72.0 / grid;
        }
        styles.push(num("font-size", drawn * PX_PER_PT));
    }
    let names: Vec<&str> = chain.iter().flatten().copied().collect();
    if let Some(family) = font_family_value(&names, &painter.font_mapping) {
        styles.push(Attr {
            name: "font-family",
            value: Value::Text(family),
        });
    }
    let emboldened = p.bold
        && east_asian
        && painter.rules.synthetic_bold == SyntheticBold::Stroke
        && p.outline.is_none()
        && has_no_bold_cut(p.font_family_ea.as_deref());
    if p.bold && !emboldened {
        styles.push(word("font-weight", "bold"));
    }
    if p.italic {
        styles.push(word("font-style", "italic"));
    }
    if let Some(c) = &p.colour {
        colour_attrs(&mut styles, "fill", "fill-opacity", c);
    }
    if emboldened {
        let width_pt = SYNTHETIC_BOLD_STROKE_PT + SYNTHETIC_BOLD_STROKE_RATIO * size * scale;
        styles.push(word("stroke", p.colour.as_ref().map_or("#000000", |c| c.hex.as_str())));
        styles.push(num("stroke-width", width_pt * PX_PER_PT));
        if let Some(c) = p.colour.as_ref().filter(|c| c.alpha < 1.0) {
            styles.push(num("stroke-opacity", c.alpha));
        }
    }
    decorations(p, &mut styles);
    styles
}

fn decorations(p: &RunProperties, styles: &mut Vec<Attr>) {
    let mut lines = Vec::new();
    if p.underline {
        lines.push("underline");
    }
    if p.strikethrough {
        lines.push("line-through");
    }
    if !lines.is_empty() {
        styles.push(word("text-decoration", &lines.join(" ")));
    }
    if let (true, Some(style)) = (p.underline, underline_style(p.underline_style.as_deref())) {
        styles.push(word("text-decoration-style", style));
    }
    if p.baseline > 0.0 {
        styles.push(word("baseline-shift", "super"));
    } else if p.baseline < 0.0 {
        styles.push(word("baseline-shift", "sub"));
    }
    if let Some(o) = &p.outline {
        styles.push(word("stroke", &o.colour.hex));
        styles.push(num("stroke-width", emu_to_px(o.width)));
        if o.colour.alpha < 1.0 {
            styles.push(num("stroke-opacity", o.colour.alpha));
        }
        styles.push(word("paint-order", "stroke"));
    }
}

/// The symbol face `family` names where the drawing has no copy of it
/// ([`Painter::mapped_symbol_faces`]), else `None`.
pub fn mapped_symbol_face(family: Option<&str>, painter: &Painter<'_>) -> Option<&'static str> {
    if painter.mapped_symbol_faces.is_empty() {
        return None;
    }
    let face = symbol_face(family?)?;
    painter.mapped_symbol_faces.contains(face).then_some(face)
}

/// The text a span's width is measured from: its own, except a symbol face drawn as
/// Unicode, whose one span is laid out at the face's advances for the codes it was
/// written in.
fn measured<'a>(segment: &'a Segment<'_>, text: &'a str, painter: &Painter<'_>) -> &'a str {
    if mapped_symbol_face(segment.properties.font_family.as_deref(), painter).is_some() {
        &segment.text
    } else {
        text
    }
}

fn spans(segment: &Segment<'_>, scale: f64, painter: &Painter<'_>, default: f64) -> Vec<Span> {
    let p = segment.properties;
    let jpan = painter.jpan_fallback_font.as_deref();
    let chains: Vec<(Option<Vec<Option<&str>>>, String)> =
        if let Some(face) = mapped_symbol_face(p.font_family.as_deref(), painter) {
            // A symbol face the drawing has no copy of: the characters it stands for, in faces
            // that have them (the layout measured the face's own advances).
            let fallbacks = FALLBACK_FAMILIES.iter().map(|f| Some(*f)).collect();
            vec![(Some(fallbacks), translate(face, &segment.text).0)]
        } else if needs_script_split(p) {
            split_by_script(&segment.text)
                .into_iter()
                .map(|(text, ea)| {
                    let mut chain = if ea {
                        vec![p.font_family_ea.as_deref(), jpan, p.font_family.as_deref()]
                    } else {
                        vec![p.font_family.as_deref(), p.font_family_ea.as_deref()]
                    };
                    chain.push(p.font_family_cs.as_deref());
                    (Some(chain), text)
                })
                .collect()
        } else {
            vec![(None, segment.text.clone())]
        };
    let own = [
        p.font_family.as_deref(),
        p.font_family_ea.as_deref(),
        p.font_family_cs.as_deref(),
    ];
    chains
        .into_iter()
        .map(|(fonts, text)| {
            let east_asian = fonts.as_ref().and_then(|f| f.first().copied().flatten()) == p.font_family_ea.as_deref()
                && fonts.is_some()
                && p.font_family_ea != p.font_family;
            let chain: Vec<Option<&str>> = fonts.unwrap_or_else(|| own.to_vec());
            let mut styles = style_attrs(p, scale, &chain, default, east_asian, painter);
            let drawn = chain
                .iter()
                .flatten()
                .copied()
                .find(|n| ooxml_fonts::substitutes::substitution_for(n).is_some());
            let oblique = p.italic && slants_italic(drawn);
            if oblique {
                if let Some(at) = styles.iter().skip(1).position(|a| a.name == "font-style") {
                    styles.remove(at + 1);
                }
            }
            let names: Vec<&str> = chain.iter().flatten().copied().collect();
            Span {
                family: font_family_value(&names, &painter.font_mapping),
                styles,
                text,
                oblique,
            }
        })
        .collect()
}

fn link(content: Vec<Element>, p: &RunProperties, painter: &Painter<'_>, even_empty: bool) -> Vec<Element> {
    match (&p.hyperlink, painter.hyperlinks) {
        (Some(h), true) if even_empty || !content.is_empty() => {
            vec![Element::open(Tag::A)
                .with("href", Value::Text(h.url.clone()))
                .holding(content)]
        }
        _ => content,
    }
}

/// The width of `text` in a segment's run, px.
pub fn text_width(text: &str, p: &RunProperties, default: f64, scale: f64, painter: &Painter<'_>) -> f64 {
    painter
        .measurer
        .width(text, super::metrics::size_or(p, default) * scale, p.bold, faces(p))
}

fn next_tab(x: f64, origin: f64, stops: &[TabStop], default_size: f64) -> (f64, &'static str) {
    for stop in stops {
        let position = origin + emu_to_px(stop.position);
        if position > x + 0.01 {
            let anchor = match stop.align.as_str() {
                "ctr" => "middle",
                "r" | "dec" => "end",
                _ => "start",
            };
            return (position, anchor);
        }
    }
    let grid = emu_to_px(default_size);
    let step = if grid == 0.0 { 1.0 } else { grid };
    let steps = ((x - origin) / step).trunc() + 1.0;
    (origin + steps * step, "start")
}

fn chunk_end(start: f64, width: f64, anchor: &str) -> f64 {
    match anchor {
        "end" => start,
        "middle" => start + width / 2.0,
        _ => start + width,
    }
}

/// A sheared italic run, drawn as its own `text` after the body's.
#[derive(Debug, Clone)]
pub struct Oblique {
    x: f64,
    baseline: f64,
    styles: Vec<Attr>,
    text: String,
}

impl Oblique {
    /// The run as an element, the body's first baseline at `y_start`.
    pub fn element(&self, y_start: f64) -> Element {
        let y = y_start + self.baseline;
        let shear = -SYNTHETIC_OBLIQUE_SHEAR.atan().to_degrees();
        let mut text = Element::open(Tag::Text)
            .num("x", self.x)
            .num("y", y)
            .with(
                "transform",
                Value::Transform(vec![
                    Op::Translate(self.x, y, Sep::Comma),
                    Op::SkewX(shear),
                    Op::Translate(-self.x, -y, Sep::Comma),
                ]),
            )
            .word("xml:space", "preserve");
        text.push(tspan(Vec::new(), &self.styles, &self.text));
        text
    }
}

/// What a line is drawn with beyond its segments.
pub struct LineContext<'a> {
    /// The line's anchor point.
    pub x: f64,
    /// `start`, `middle` or `end`.
    pub anchor: &'static str,
    /// The left inset tab stops are measured from.
    pub origin: f64,
    /// The paragraph's tab stops.
    pub stops: &'a [TabStop],
    /// `a:bodyPr@defTabSz`, EMU.
    pub default_tab: f64,
    /// The body's default size, points.
    pub default_size: f64,
    /// The font scale.
    pub scale: f64,
    /// The line's baseline below the body's first.
    pub baseline: f64,
}

/// One line's spans, starting a chunk at every tab and at a change of face.
pub fn render_line(
    segments: &[Segment<'_>],
    dy: Option<Dy>,
    line: &LineContext<'_>,
    painter: &Painter<'_>,
    obliques: &mut Vec<Oblique>,
) -> Vec<Element> {
    let mut pieces: Vec<Option<Segment<'_>>> = Vec::new();
    for s in segments {
        if !s.text.contains('\t') {
            pieces.push(Some(s.clone()));
            continue;
        }
        for (i, part) in s.text.split('\t').enumerate() {
            if i > 0 {
                pieces.push(None);
            }
            if !part.is_empty() {
                pieces.push(Some(Segment {
                    text: part.to_owned(),
                    ..s.clone()
                }));
            }
        }
    }
    let honour_tabs = line.anchor == "start" && pieces.iter().any(Option::is_none);
    let planned: Vec<Option<(Segment<'_>, Vec<Span>)>> = pieces
        .into_iter()
        .map(|p| {
            p.map(|s| {
                let sp = spans(&s, line.scale, painter, line.default_size);
                (s, sp)
            })
        })
        .collect();
    let all = || {
        planned
            .iter()
            .flatten()
            .flat_map(|(s, sp)| sp.iter().map(move |x| (s, x)))
    };
    let mut keys: Vec<Key> = Vec::new();
    for (_, span) in all() {
        let key = chunk_key(span.family.as_ref(), &span.text);
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let any_oblique = all().any(|(_, s)| s.oblique);
    if keys.len() <= 1 && !honour_tabs && !any_oblique {
        return flowing(&planned, dy, line, painter);
    }
    let left = if honour_tabs {
        line.x
    } else {
        let total: f64 = all()
            .map(|(s, span)| {
                text_width(
                    measured(s, &span.text, painter),
                    s.properties,
                    line.default_size,
                    line.scale,
                    painter,
                )
            })
            .sum();
        match line.anchor {
            "middle" => line.x - total / 2.0,
            "end" => line.x - total,
            _ => line.x,
        }
    };
    chunked(&planned, dy, left, line, painter, obliques)
}

fn flowing(
    planned: &[Option<(Segment<'_>, Vec<Span>)>],
    dy: Option<Dy>,
    line: &LineContext<'_>,
    painter: &Painter<'_>,
) -> Vec<Element> {
    let mut out = Vec::new();
    let mut first = true;
    for (segment, spans) in planned.iter().flatten() {
        let content: Vec<Element> = spans
            .iter()
            .enumerate()
            .map(|(i, span)| {
                let prefix = if first && i == 0 {
                    leading(line.x, dy, line.anchor)
                } else {
                    Vec::new()
                };
                tspan(prefix, &span.styles, &span.text)
            })
            .collect();
        let linked = link(content, segment.properties, painter, true);
        first = first && linked.is_empty();
        out.extend(linked);
    }
    out
}

fn chunked(
    planned: &[Option<(Segment<'_>, Vec<Span>)>],
    dy: Option<Dy>,
    left: f64,
    line: &LineContext<'_>,
    painter: &Painter<'_>,
    obliques: &mut Vec<Oblique>,
) -> Vec<Element> {
    let mut out = Vec::new();
    let (mut cursor, mut chunk_start, mut chunk_width) = (left, left, 0.0);
    let mut chunk_anchor = "start";
    let mut previous = Key::Nothing;
    let mut pending: Option<(f64, &str)> = Some((left, "start"));
    let mut advance = dy;
    for entry in planned {
        let Some((segment, spans)) = entry else {
            cursor = chunk_end(chunk_start, chunk_width, chunk_anchor);
            let (stop, anchor) = next_tab(cursor, line.origin, line.stops, line.default_tab);
            chunk_anchor = anchor;
            pending = Some((stop, anchor));
            cursor = stop;
            chunk_start = stop;
            chunk_width = 0.0;
            previous = Key::Nothing;
            continue;
        };
        let mut content = Vec::new();
        for span in spans {
            let text = measured(segment, &span.text, painter);
            let width = text_width(text, segment.properties, line.default_size, line.scale, painter);
            if span.oblique && chunk_anchor == "start" {
                obliques.push(Oblique {
                    x: cursor,
                    baseline: line.baseline,
                    styles: span.styles.clone(),
                    text: span.text.clone(),
                });
                pending = None;
                previous = Key::Nothing;
                cursor += width;
                chunk_width += width;
                continue;
            }
            let key = chunk_key(span.family.as_ref(), &span.text);
            let prefix = if let Some((x, anchor)) = pending.take() {
                previous = key;
                leading(x, advance.take(), anchor)
            } else if key != previous {
                previous = key;
                if chunk_anchor == "start" {
                    leading(cursor, advance.take(), "start")
                } else {
                    advance.take().map(Dy::attr).into_iter().collect()
                }
            } else {
                advance.take().map(Dy::attr).into_iter().collect()
            };
            content.push(tspan(prefix, &span.styles, &span.text));
            cursor += width;
            chunk_width += width;
        }
        out.extend(link(content, segment.properties, painter, false));
    }
    if let Some(dy) = advance {
        let mut spacer = Element::open(Tag::Tspan);
        spacer.attrs = leading(left, Some(dy), "start");
        spacer.push_text(" ");
        out.insert(0, spacer);
    }
    out
}

/// A run's highlight, positioned against the body's first baseline.
#[derive(Debug, Clone)]
pub struct Highlight {
    x: f64,
    baseline: f64,
    width: f64,
    ascent: f64,
    descent: f64,
    colour: Colour,
}

impl Highlight {
    /// The highlight as a rectangle.
    pub fn element(&self, y_start: f64) -> Element {
        let y = y_start + self.baseline - self.ascent;
        let mut r = Element::empty(Tag::Rect)
            .num("x", self.x)
            .num("y", y)
            .num("width", self.width)
            .num("height", self.ascent + self.descent)
            .word("fill", &self.colour.hex);
        if self.colour.alpha < 1.0 {
            r = r.num("fill-opacity", self.colour.alpha);
        }
        r
    }
}

/// Rectangles behind the highlighted runs of a line anchored at `x`.
pub fn line_highlights(
    segments: &[Segment<'_>],
    x: f64,
    anchor: &str,
    baseline: f64,
    sizes: (f64, f64),
    painter: &Painter<'_>,
) -> Vec<Highlight> {
    if !segments.iter().any(|s| s.properties.highlight.is_some()) {
        return Vec::new();
    }
    let (default, scale) = sizes;
    let widths: Vec<f64> = segments
        .iter()
        .map(|s| text_width(&s.text, s.properties, default, scale, painter))
        .collect();
    let total: f64 = widths.iter().sum();
    let mut left = match anchor {
        "middle" => x - total / 2.0,
        "end" => x - total,
        _ => x,
    };
    let mut out = Vec::new();
    for (s, width) in segments.iter().zip(widths) {
        if let (Some(colour), true) = (&s.properties.highlight, width > 0.0) {
            let size = super::metrics::size_or(s.properties, default) * scale * PX_PER_PT;
            let ascent = painter.measurer.ascender_ratio(faces(s.properties));
            let line = painter.measurer.line_height_ratio(faces(s.properties));
            out.push(Highlight {
                x: left,
                baseline,
                width,
                ascent: size * ascent,
                descent: size * (line - ascent).max(0.0),
                colour: colour.clone(),
            });
        }
        left += width;
    }
    out
}
