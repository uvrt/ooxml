//! Owns: a text body laid out and drawn in its frame (ooxml-common's
//! `textbody.render_text_body`): the frame unscaled out of an enclosing group, shrunk to its
//! geometry's text rectangle, turned for vertical text and `bodyPr@rot`; paragraphs wrapped,
//! bulleted and numbered, spaced and anchored; columns; and `spAutoFit`'s height.
//!
//! Must not: draw the shape the text belongs to.

use super::bullet::{bullet_size_pt, bullet_span, bullet_text, picture_bullet};
use super::line::{leading, line_highlights, render_line, Dy, Highlight, LineContext, Oblique};
use super::metrics::{
    default_ascender_ratio, default_font_size, default_line_ratio, estimate_height, first_baseline_px, line_descent,
    line_font_size, line_height_px, line_natural_height, paragraph_descent, paragraph_font_size,
    paragraph_natural_height, run_segments, shrink_to_fit, spacing_px, stored_autofit,
};
use super::wrap::{wrap_paragraph, Line, Segment};
use crate::context::Painter;
use crate::geometry::{text_area, TextGeometry};
use crate::measure::PX_PER_PT;
use crate::rules::{Autofit, BlankParagraphs, FirstBaseline};
use crate::units::{emu_to_px, px_to_emu};
use ooxml_drawingml::drawable::{BodyProperties, Paragraph, ParagraphProperties, RunProperties, TextBody, Transform};
use ooxml_scene::markup::{Element, Op, Sep, Tag, Value};
use std::collections::HashMap;

/// The text box's dimensions and insets, px, turned for vertical text.
#[derive(Debug, Clone, Copy)]
struct Dims {
    width: f64,
    height: f64,
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
}

fn vertical(vert: &str) -> bool {
    matches!(vert, "vert" | "eaVert" | "wordArtVert" | "mongolianVert")
}

fn dims(body: &BodyProperties, width: f64, height: f64) -> Dims {
    let px = emu_to_px;
    if vertical(&body.vert) {
        return Dims {
            width: height,
            height: width,
            left: px(body.margin_top),
            right: px(body.margin_bottom),
            top: px(body.margin_right),
            bottom: px(body.margin_left),
        };
    }
    if body.vert == "vert270" {
        return Dims {
            width: height,
            height: width,
            left: px(body.margin_bottom),
            right: px(body.margin_top),
            top: px(body.margin_left),
            bottom: px(body.margin_right),
        };
    }
    Dims {
        width,
        height,
        left: px(body.margin_left),
        right: px(body.margin_right),
        top: px(body.margin_top),
        bottom: px(body.margin_bottom),
    }
}

fn translate(x: f64, y: f64) -> Op {
    Op::Translate(x, y, Sep::CommaSpace)
}

fn group(ops: Vec<Op>, children: Vec<Element>) -> Element {
    Element::open(Tag::G)
        .with("transform", Value::Transform(ops))
        .holding(children)
}

/// The frame laid out in slide units inside a uniformly scaled group, and the scale that
/// puts it back.
fn unscaled(transform: &Transform, painter: &Painter<'_>) -> (Transform, Option<Op>) {
    let (sx, sy) = painter.group_scale;
    if (sx, sy) == (1.0, 1.0) || sx == 0.0 || sy == 0.0 {
        return (*transform, None);
    }
    let frame = Transform {
        extent_width: transform.extent_width * sx,
        extent_height: transform.extent_height * sy,
        ..*transform
    };
    (frame, Some(Op::Scale(1.0 / sx, 1.0 / sy, Sep::CommaSpace)))
}

fn area(
    frame: Transform,
    geometry: Option<TextGeometry<'_>>,
    outline_width: f64,
    painter: &Painter<'_>,
) -> (Transform, f64, f64) {
    if geometry.is_none() {
        return (frame, 0.0, 0.0);
    }
    let (l, t, r, b) = text_area(
        geometry,
        frame.extent_width,
        frame.extent_height,
        outline_width,
        &painter.rules,
    );
    if (l, t, r, b) == (0.0, 0.0, frame.extent_width, frame.extent_height) {
        return (frame, 0.0, 0.0);
    }
    let shrunk = Transform {
        extent_width: (r - l).max(0.0),
        extent_height: (b - t).max(0.0),
        ..frame
    };
    (shrunk, l, t)
}

/// A text body drawn in `transform`'s box, in `geometry`'s text rectangle where given.
pub fn render_text_body(
    text: &TextBody,
    transform: &Transform,
    painter: &mut Painter<'_>,
    geometry: Option<TextGeometry<'_>>,
    outline_width: f64,
) -> Vec<Element> {
    let stored = stored_autofit(text, painter.rules.autofit);
    let text = stored.as_ref().unwrap_or(text);
    let body = &text.properties;
    let (frame, undo) = unscaled(transform, painter);
    let (frame, left, top) = area(frame, geometry, outline_width, painter);
    let mut rendered = if body.columns > 1 && body.vert == "horz" {
        render_columns(text, &frame, painter)
    } else {
        render_column(text, &frame, painter, &mut HashMap::new())
    };
    if !rendered.is_empty() && (left != 0.0 || top != 0.0) {
        rendered = vec![group(vec![translate(emu_to_px(left), emu_to_px(top))], rendered)];
    }
    if let (false, Some(op)) = (rendered.is_empty(), undo) {
        rendered = vec![group(vec![op], rendered)];
    }
    rendered
}

fn has_text(paragraphs: &[Paragraph]) -> bool {
    paragraphs.iter().flat_map(|p| &p.runs).any(|r| !r.text.is_empty())
}

fn render_columns(text: &TextBody, frame: &Transform, painter: &mut Painter<'_>) -> Vec<Element> {
    let body = &text.properties;
    if !has_text(&text.paragraphs) {
        return Vec::new();
    }
    let columns = body.columns.max(1);
    let d = dims(body, emu_to_px(frame.extent_width), emu_to_px(frame.extent_height));
    let column_width = (d.width - d.left - d.right) / columns as f64;
    let available = d.height - d.top - d.bottom;
    let default = default_font_size(&text.paragraphs);
    let wrap = body.wrap != "none";
    let heights: Vec<f64> = text
        .paragraphs
        .iter()
        .map(|p| {
            estimate_height(
                std::slice::from_ref(p),
                default,
                wrap,
                column_width,
                (body.line_reduction, body.font_scale),
                painter.measurer,
            )
        })
        .collect();
    let mut groups: Vec<Vec<Paragraph>> = vec![Vec::new()];
    let mut used = 0.0;
    for (paragraph, height) in text.paragraphs.iter().zip(heights) {
        let full = groups.last().is_some_and(|g| !g.is_empty());
        if full && used + height > available && (groups.len() as i64) < columns {
            groups.push(Vec::new());
            used = 0.0;
        }
        if let Some(g) = groups.last_mut() {
            g.push(paragraph.clone());
        }
        used += height;
    }
    let mut counters = HashMap::new();
    let mut out = Vec::new();
    for (index, paragraphs) in groups.into_iter().enumerate() {
        if paragraphs.is_empty() {
            continue;
        }
        let index = index as f64;
        let column = TextBody {
            paragraphs,
            properties: BodyProperties {
                columns: 1,
                margin_left: body.margin_left + px_to_emu(index * column_width),
                margin_right: body.margin_right + px_to_emu((columns as f64 - 1.0 - index) * column_width),
                ..body.clone()
            },
        };
        out.extend(render_column(&column, frame, painter, &mut counters));
    }
    out
}

/// A picture bullet, positioned against the body's first baseline.
struct BulletImage {
    x: f64,
    baseline: f64,
    size: f64,
    data: String,
    mime: String,
}

impl BulletImage {
    fn element(&self, y_start: f64) -> Element {
        let y = y_start + self.baseline - self.size;
        Element::empty(Tag::Image)
            .num("x", self.x)
            .num("y", y)
            .num("width", self.size)
            .num("height", self.size)
            .word("preserveAspectRatio", "xMidYMid meet")
            .with(
                "xlink:href",
                Value::Text(format!("data:{};base64,{}", self.mime, self.data)),
            )
    }
}

/// One column being drawn: how its paragraphs are laid out, and what they have drawn.
struct Flow<'b> {
    dims: Dims,
    text_width: f64,
    wrap: bool,
    sizes: Sizes,
    body: &'b BodyProperties,
    spans: Vec<Element>,
    highlights: Vec<Highlight>,
    bullets: Vec<BulletImage>,
    obliques: Vec<Oblique>,
    baseline: f64,
    first_line: bool,
    previous_descent: Option<f64>,
    word_steps: bool,
}

/// The body's sizes as a paragraph is drawn with them.
#[derive(Clone, Copy)]
struct Sizes {
    default: f64,
    scale: f64,
    reduction: f64,
    line_ratio: f64,
}

/// A paragraph's horizontal placement and what leads its first line.
struct Place<'t> {
    x: f64,
    anchor: &'static str,
    bullet_x: f64,
    width: f64,
    bullet: Option<String>,
    picture: Option<(&'t str, &'t str)>,
}

fn alignment(alignment: Option<&str>, left: f64, width: f64, d: &Dims) -> (f64, &'static str) {
    match alignment {
        Some("ctr") => (left + width / 2.0, "middle"),
        Some("r") => (d.width - d.right, "end"),
        _ => (left, "start"),
    }
}

fn render_column(
    text: &TextBody,
    frame: &Transform,
    painter: &mut Painter<'_>,
    counters: &mut HashMap<String, i64>,
) -> Vec<Element> {
    let body = &text.properties;
    let paragraphs = &text.paragraphs;
    if !has_text(paragraphs) {
        return Vec::new();
    }
    let (original_width, original_height) = (emu_to_px(frame.extent_width), emu_to_px(frame.extent_height));
    let d = dims(body, original_width, original_height);
    let full = d.width - d.left - d.right;
    let columns = body.columns.max(1);
    let text_width = if columns > 1 { full / columns as f64 } else { full };
    let default = default_font_size(paragraphs);
    let wrap = body.wrap != "none";
    let mut scale = body.font_scale;
    let reduction = body.line_reduction;
    if body.auto_fit == "normAutofit" && wrap && painter.rules.autofit == Autofit::Fit {
        let room = (text_width, d.height - d.top - d.bottom);
        scale = shrink_to_fit(paragraphs, default, scale, reduction, room, painter.measurer);
    }
    let sizes = Sizes {
        default,
        scale,
        reduction,
        line_ratio: default_line_ratio(paragraphs, painter.measurer),
    };
    let ascender = default_ascender_ratio(paragraphs, painter.measurer);
    let mut flow = Flow {
        dims: d,
        text_width,
        wrap,
        sizes,
        body,
        spans: Vec::new(),
        highlights: Vec::new(),
        bullets: Vec::new(),
        obliques: Vec::new(),
        baseline: 0.0,
        first_line: true,
        previous_descent: None,
        word_steps: painter.rules.first_baseline == FirstBaseline::Descent,
    };
    let mut previous_after = 0.0;
    for paragraph in paragraphs {
        previous_after = flow.paragraph(paragraph, previous_after, painter, counters);
    }
    if flow.spans.is_empty() && flow.obliques.is_empty() {
        return Vec::new();
    }
    let total = estimate_height(
        paragraphs,
        default,
        wrap,
        text_width,
        (reduction, scale),
        painter.measurer,
    );
    let mut y_start = d.top;
    if body.anchor == "ctr" {
        y_start = (d.height - total) / 2.0;
    } else if body.anchor == "b" {
        y_start = d.height - total - d.bottom;
    }
    if let Some(first) = paragraphs.first() {
        let size = paragraph_font_size(first, default) * scale;
        y_start += first_baseline_px(first, size, ascender, reduction, painter);
    }
    let text_el = Element::open(Tag::Text)
        .word("x", "0")
        .num("y", y_start)
        .word("xml:space", "preserve")
        .holding(std::mem::take(&mut flow.spans));
    let mut out: Vec<Element> = flow.highlights.iter().map(|h| h.element(y_start)).collect();
    out.push(text_el);
    out.extend(flow.obliques.iter().map(|o| o.element(y_start)));
    out.extend(flow.bullets.iter().map(|b| b.element(y_start)));
    turned(out, body, original_width, original_height)
}

fn turned(mut out: Vec<Element>, body: &BodyProperties, width: f64, height: f64) -> Vec<Element> {
    if vertical(&body.vert) {
        out = vec![group(vec![translate(width, 0.0), Op::Rotate(90.0)], out)];
    } else if body.vert == "vert270" {
        out = vec![group(vec![translate(0.0, height), Op::Rotate(-90.0)], out)];
    }
    if body.rotation != 0.0 {
        out = vec![group(
            vec![Op::RotateAbout(
                body.rotation,
                width / 2.0,
                height / 2.0,
                Sep::CommaSpace,
            )],
            out,
        )];
    }
    out
}

fn first_line_whitespace(paragraph: &Paragraph) -> Vec<Segment<'_>> {
    let mut out = Vec::new();
    for (run, r) in paragraph.runs.iter().enumerate() {
        if r.text.is_empty() {
            continue;
        }
        let (head, newline) = match r.text.split_once('\n') {
            Some((head, _)) => (head, true),
            None => (r.text.as_str(), false),
        };
        if !head.is_empty() {
            out.push(Segment {
                text: head.to_owned(),
                run,
                properties: &r.properties,
            });
        }
        if newline {
            if out.is_empty() {
                out.push(Segment {
                    text: String::new(),
                    run,
                    properties: &r.properties,
                });
            }
            break;
        }
    }
    out
}

impl Flow<'_> {
    fn dy(&mut self, height: f64, descent: f64, gap: f64) -> Dy {
        let mut height = height;
        if let (true, Some(previous)) = (self.word_steps, self.previous_descent) {
            height += previous - descent;
        }
        self.previous_descent = Some(descent);
        if self.first_line {
            Dy::Zero
        } else {
            Dy::Step(height + gap)
        }
    }

    fn blank(&mut self, x: f64, anchor: &str, dy: Dy) {
        let mut t = Element::open(Tag::Tspan);
        t.attrs = leading(x, Some(dy), anchor);
        t.push_text(" ");
        self.spans.push(t);
        self.baseline += dy.value();
        self.first_line = false;
    }

    /// Draws one paragraph; returns its space after.
    fn paragraph(
        &mut self,
        paragraph: &Paragraph,
        previous_after: f64,
        painter: &Painter<'_>,
        counters: &mut HashMap<String, i64>,
    ) -> f64 {
        let props = &paragraph.properties;
        let margin = emu_to_px(props.margin_left.unwrap_or(0.0));
        let indent = emu_to_px(props.indent.unwrap_or(0.0));
        let text_start = self.dims.left + margin;
        let width = self.text_width - margin;
        let has_text = paragraph.runs.iter().any(|r| !r.text.is_empty());
        let whitespace_is_text = painter.rules.blank_paragraphs == BlankParagraphs::Text;
        let bullet = if has_text || !whitespace_is_text {
            bullet_text(props, counters)
        } else {
            None
        };
        let (x, anchor) = alignment(props.alignment.as_deref(), text_start, width, &self.dims);
        let place = Place {
            x,
            anchor,
            bullet_x: text_start + indent,
            width,
            bullet: bullet.filter(|b| !b.is_empty()),
            picture: picture_bullet(props),
        };
        let m = painter.measurer;
        let natural = paragraph_natural_height(
            paragraph,
            self.sizes.default,
            self.sizes.scale,
            self.sizes.line_ratio,
            m,
        );
        let gap = previous_after + spacing_px(props.space_before, natural);
        if !has_text {
            let height = line_height_px(paragraph, natural, self.sizes.reduction);
            let descent = paragraph_descent(paragraph, self.sizes.default, self.sizes.scale, m);
            let dy = self.dy(height, descent, gap);
            self.blank(place.x, place.anchor, dy);
        } else if self.wrap {
            self.wrapped(paragraph, &place, gap, painter);
        } else {
            self.unwrapped(paragraph, &place, gap, painter);
        }
        spacing_px(props.space_after, natural)
    }

    fn wrapped(&mut self, paragraph: &Paragraph, place: &Place<'_>, gap: f64, painter: &Painter<'_>) {
        let m = painter.measurer;
        let sizes = self.sizes;
        let mut lines: Vec<Line<'_>> =
            wrap_paragraph(paragraph, place.width, sizes.default * sizes.scale, sizes.scale, m);
        if painter.rules.blank_paragraphs == BlankParagraphs::Text {
            if let Some(first) = lines.first_mut().filter(|l| l.segments.is_empty()) {
                first.segments = first_line_whitespace(paragraph);
            }
        }
        let props = &paragraph.properties;
        for (index, line) in lines.iter().enumerate() {
            let line_gap = if index == 0 { gap } else { 0.0 };
            if line.segments.is_empty() {
                let height = line_height_px(
                    paragraph,
                    sizes.default * sizes.scale * sizes.line_ratio,
                    sizes.reduction,
                );
                let descent = paragraph_descent(paragraph, sizes.default, sizes.scale, m);
                let dy = self.dy(height, descent, line_gap);
                self.blank(place.x, place.anchor, dy);
                continue;
            }
            let natural = line_natural_height(&line.segments, sizes.default, sizes.scale, m);
            let height = line_height_px(paragraph, natural, sizes.reduction);
            let descent = line_descent(&line.segments, sizes.default, sizes.scale, m);
            let dy = self.dy(height, descent, line_gap);
            let size = line_font_size(&line.segments, sizes.default) * sizes.scale;
            let mut line_dy = Some(dy);
            match (index, place.bullet.as_deref(), line.segments.first()) {
                (0, Some(text), Some(first)) => {
                    self.spans.push(bullet_span(
                        props,
                        size,
                        first.properties,
                        text,
                        place.bullet_x,
                        dy,
                        painter,
                    ));
                    line_dy = None;
                }
                _ => {
                    if let (0, Some((data, mime))) = (index, place.picture) {
                        self.picture(place.bullet_x, dy, bullet_size_pt(props, size), data, mime);
                    }
                }
            }
            self.line(&line.segments, place, line_dy, dy, props, painter);
        }
    }

    fn picture(&mut self, x: f64, dy: Dy, size_pt: f64, data: &str, mime: &str) {
        self.bullets.push(BulletImage {
            x,
            baseline: self.baseline + dy.value(),
            size: size_pt * PX_PER_PT,
            data: data.to_owned(),
            mime: mime.to_owned(),
        });
    }

    fn unwrapped(&mut self, paragraph: &Paragraph, place: &Place<'_>, gap: f64, painter: &Painter<'_>) {
        let m = painter.measurer;
        let sizes = self.sizes;
        let all = run_segments(paragraph);
        let natural = line_natural_height(&all, sizes.default, sizes.scale, m);
        let height = line_height_px(paragraph, natural, sizes.reduction);
        let descent = line_descent(&all, sizes.default, sizes.scale, m);
        let dy = self.dy(height, descent, gap);
        let props = &paragraph.properties;
        let first_run = paragraph.runs.iter().find(|r| !r.text.is_empty());
        let size = first_run
            .and_then(|r| r.properties.font_size)
            .filter(|s| *s != 0.0)
            .unwrap_or(sizes.default)
            * sizes.scale;
        let empty = RunProperties::default();
        if let Some(text) = place.bullet.as_deref() {
            let first = first_run.map_or(&empty, |r| &r.properties);
            self.spans
                .push(bullet_span(props, size, first, text, place.bullet_x, dy, painter));
        }
        if let Some((data, mime)) = place.picture {
            self.picture(place.bullet_x, dy, bullet_size_pt(props, size), data, mime);
        }
        let line_dy = if place.bullet.is_some() { None } else { Some(dy) };
        let segments: Vec<Segment<'_>> = all.into_iter().filter(|s| !s.text.is_empty()).collect();
        self.line(&segments, place, line_dy, dy, props, painter);
    }

    fn line(
        &mut self,
        segments: &[Segment<'_>],
        place: &Place<'_>,
        line_dy: Option<Dy>,
        dy: Dy,
        props: &ParagraphProperties,
        painter: &Painter<'_>,
    ) {
        let context = LineContext {
            x: place.x,
            anchor: place.anchor,
            origin: self.dims.left,
            stops: &props.tab_stops,
            default_tab: self.body.default_tab_size,
            default_size: self.sizes.default,
            scale: self.sizes.scale,
            baseline: self.baseline + dy.value(),
        };
        let spans = render_line(segments, line_dy, &context, painter, &mut self.obliques);
        self.spans.extend(spans);
        self.baseline += dy.value();
        let sizes = (self.sizes.default, self.sizes.scale);
        self.highlights.extend(line_highlights(
            segments,
            place.x,
            place.anchor,
            self.baseline,
            sizes,
            painter,
        ));
        self.first_line = false;
    }
}

/// `spAutoFit`'s height for a body in `transform`, EMU in the caller's space; `None` where
/// the text already fits.
pub fn sp_autofit_height(
    text: &TextBody,
    transform: &Transform,
    painter: &Painter<'_>,
    geometry: Option<TextGeometry<'_>>,
    outline_width: f64,
) -> Option<f64> {
    let body = &text.properties;
    if !has_text(&text.paragraphs) {
        return None;
    }
    let (frame, _) = unscaled(transform, painter);
    let whole = frame.extent_height;
    let (frame, _, _) = area(frame, geometry, outline_width, painter);
    let outside = whole - frame.extent_height;
    let (_, scale_y) = painter.group_scale;
    let d = dims(body, emu_to_px(frame.extent_width), emu_to_px(frame.extent_height));
    let full = d.width - d.left - d.right;
    let columns = body.columns.max(1);
    let width = if columns > 1 { full / columns as f64 } else { full };
    let mut height = estimate_height(
        &text.paragraphs,
        default_font_size(&text.paragraphs),
        body.wrap != "none",
        width,
        (0.0, 1.0),
        painter.measurer,
    );
    if columns > 1 && body.vert == "horz" {
        height /= columns as f64;
    }
    let required = height + d.top + d.bottom;
    if required <= d.height {
        return None;
    }
    let divisor = if scale_y == 0.0 { 1.0 } else { scale_y };
    Some((px_to_emu(required) + outside) / divisor)
}
