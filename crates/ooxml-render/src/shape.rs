//! Owns: a shape, a connector, a picture and a table as elements (ooxml-common's
//! `drawingml.shape`): each in a group carrying its placement (translate, turn about the
//! centre, flip), its geometry painted with its fill and outline, its text in its text
//! rectangle, its effects; a picture cropped, stretched or tiled and clipped to its frame;
//! a table's cells, rows grown to their text, and borders over the fills.
//!
//! Must not: decide what a fill or an effect is (`fill`, `effect` do), or lay text out.

use crate::context::Painter;
use crate::effect::{render_blip_effects, render_effects};
use crate::fill::{fill_attrs, markers, outline_attrs, tile_pattern};
use crate::geometry::{render_geometry, TextGeometry};
use crate::rules::{Autofit, CustomPathStrokes};
use crate::text::{render_text_body, sp_autofit_height};
use crate::units::emu_to_px;
use ooxml_drawingml::drawable::{
    CellBorders, Connector, Geometry, Image, Outline, Paragraph, RunProperties, Shape, Table, TextBody, TextRun,
    Transform,
};
use ooxml_scene::markup::{Attr, Element, Op, Sep, Tag, Value};

/// Translate, then turn about the centre, then flip: PowerPoint's order.
pub fn placement(t: &Transform) -> Vec<Op> {
    let (x, y, w, h) = (
        emu_to_px(t.offset_x),
        emu_to_px(t.offset_y),
        emu_to_px(t.extent_width),
        emu_to_px(t.extent_height),
    );
    let mut ops = vec![Op::Translate(x, y, Sep::CommaSpace)];
    if t.rotation != 0.0 {
        ops.push(Op::RotateAbout(t.rotation, w / 2.0, h / 2.0, Sep::CommaSpace));
    }
    if t.flip_h || t.flip_v {
        ops.push(Op::Translate(
            if t.flip_h { w } else { 0.0 },
            if t.flip_v { h } else { 0.0 },
            Sep::CommaSpace,
        ));
        ops.push(Op::Scale(
            if t.flip_h { -1.0 } else { 1.0 },
            if t.flip_v { -1.0 } else { 1.0 },
            Sep::CommaSpace,
        ));
    }
    ops
}

fn placed(t: &Transform, filter: Option<Attr>) -> Element {
    let mut g = Element::open(Tag::G).with("transform", Value::Transform(placement(t)));
    g.attrs.extend(filter);
    g
}

fn scale_paths(painter: &Painter<'_>) -> bool {
    painter.rules.custom_path_strokes == CustomPathStrokes::Stated
}

/// The shape's geometry as `text_rect` takes it: `None` (the whole box) for a `rect`.
pub fn text_geometry(shape: &Shape) -> Option<TextGeometry<'_>> {
    match &shape.geometry {
        Geometry::Preset { name, .. } if name == "rect" => None,
        Geometry::Preset { name, adjust } => Some(TextGeometry::Preset(name, adjust)),
        Geometry::Custom(_) => shape.text_rect.as_ref().map(TextGeometry::Custom),
    }
}

fn outline_width(outline: Option<&Outline>) -> f64 {
    outline.map_or(0.0, |o| o.width)
}

const BOXED_PRESETS: [&str; 12] = [
    "rect",
    "roundRect",
    "ellipse",
    "snip1Rect",
    "snip2SameRect",
    "snip2DiagRect",
    "snipRoundRect",
    "round1Rect",
    "round2SameRect",
    "round2DiagRect",
    "flowChartProcess",
    "flowChartAlternateProcess",
];

/// What a geometry over the box draws within, its stroke included; `None` for a geometry
/// not known to stay in its box (`geometry` `None` is a rectangle).
fn drawn_bounds(
    geometry: Option<&Geometry>,
    w: f64,
    h: f64,
    outline: Option<&Outline>,
) -> Option<(f64, f64, f64, f64)> {
    if let Some(g) = geometry {
        if !matches!(g, Geometry::Preset { name, .. } if BOXED_PRESETS.contains(&name.as_str())) {
            return None;
        }
    }
    let reach = match outline {
        Some(o) if o.fill.is_some() => 2.0 * emu_to_px(o.width),
        _ => 0.0,
    };
    Some((-reach, -reach, w + reach, h + reach))
}

fn turned(t: &Transform, painter: &Painter<'_>) -> bool {
    t.rotation != 0.0 || painter.turned
}

fn has_text(body: Option<&TextBody>) -> bool {
    body.is_some_and(|b| {
        b.paragraphs
            .iter()
            .any(|p| p.properties.bullet.is_some() || p.runs.iter().any(|r| !r.text.is_empty()))
    })
}

/// `geometry` with `styles` put before its own attributes.
fn styled(mut geometry: Element, styles: Vec<Attr>) -> Element {
    geometry.prepend(styles);
    geometry
}

fn shape_text(shape: &Shape, transform: &Transform, painter: &mut Painter<'_>) -> Vec<Element> {
    let Some(body) = &shape.text else { return Vec::new() };
    let frame = shape.text_transform.as_ref().unwrap_or(transform);
    let geometry = if shape.text_transform.is_none() {
        text_geometry(shape)
    } else {
        None
    };
    let text = render_text_body(body, frame, painter, geometry, outline_width(shape.outline.as_ref()));
    let Some(own) = &shape.text_transform else { return text };
    if text.is_empty() {
        return text;
    }
    let dx = emu_to_px(own.offset_x - transform.offset_x);
    let dy = emu_to_px(own.offset_y - transform.offset_y);
    if dx == 0.0 && dy == 0.0 {
        return text;
    }
    vec![Element::open(Tag::G)
        .with(
            "transform",
            Value::Transform(vec![Op::Translate(dx, dy, Sep::CommaSpace)]),
        )
        .holding(text)]
}

/// A shape.
pub fn render_shape(shape: &Shape, painter: &mut Painter<'_>) -> Element {
    let mut transform = shape.transform;
    if let Some(body) = shape.text.as_ref().filter(|b| b.properties.auto_fit == "spAutofit") {
        if painter.rules.autofit == Autofit::Fit {
            let geometry = if shape.text_transform.is_none() {
                text_geometry(shape)
            } else {
                None
            };
            if let Some(required) = sp_autofit_height(
                body,
                &transform,
                painter,
                geometry,
                outline_width(shape.outline.as_ref()),
            ) {
                transform.extent_height = required;
            }
        }
    }
    let (w, h) = (emu_to_px(transform.extent_width), emu_to_px(transform.extent_height));
    let mut styles = fill_attrs(shape.fill.as_ref(), painter, Some((0.0, 0.0, w, h)));
    styles.extend(outline_attrs(shape.outline.as_ref(), painter, None));
    let bounds = if has_text(shape.text.as_ref()) || turned(&transform, painter) {
        None
    } else {
        drawn_bounds(Some(&shape.geometry), w, h, shape.outline.as_ref())
    };
    let filter = render_effects(shape.effects.as_ref(), painter, bounds);
    let mut g = placed(&transform, filter);
    g.push(styled(
        render_geometry(&shape.geometry, w, h, scale_paths(painter)),
        styles,
    ));
    if shape.text.is_some() {
        let mut text = shape_text(shape, &transform, painter);
        if let (false, Some(owner), true) = (text.is_empty(), &shape.meta.element_id, painter.edit_marks) {
            let kind = if owner.contains('/') { "diagram" } else { "shape" };
            text = vec![Element::open(Tag::G).word("data-pptx-text", kind).holding(text)];
        }
        for t in text {
            g.push(t);
        }
    }
    g
}

/// A connector.
pub fn render_connector(connector: &Connector, painter: &mut Painter<'_>) -> Element {
    let t = &connector.transform;
    let (w, h) = (emu_to_px(t.extent_width), emu_to_px(t.extent_height));
    let mut styles = outline_attrs(connector.outline.as_ref(), painter, None);
    let filter = render_effects(connector.effects.as_ref(), painter, None);
    styles.push(Attr {
        name: "fill",
        value: Value::Word("none".into()),
    });
    styles.extend(markers(connector.outline.as_ref(), painter));
    let mut g = placed(t, filter);
    g.push(styled(
        render_geometry(&connector.geometry, w, h, scale_paths(painter)),
        styles,
    ));
    g
}

fn is_plain_rect(geometry: &Geometry) -> bool {
    matches!(geometry, Geometry::Preset { name, .. } if name == "rect" || name == "flowChartProcess")
}

fn image_bounds(image: &Image, w: f64, h: f64) -> Option<(f64, f64, f64, f64)> {
    let mut bounds = drawn_bounds(image.geometry.as_ref(), w, h, image.outline.as_ref())?;
    let cropped = image.src_rect.is_some_and(|r| r.any());
    if let (false, None, Some(s)) = (cropped, &image.tile, image.stretch.filter(|s| s.any())) {
        let (left, top) = (s.left * w, s.top * h);
        let (right, bottom) = (w * (1.0 - s.right), h * (1.0 - s.bottom));
        bounds = (
            bounds.0.min(left),
            bounds.1.min(top),
            bounds.2.max(right),
            bounds.3.max(bottom),
        );
    }
    Some(bounds)
}

fn picture(href: &str, x: f64, y: f64, w: f64, h: f64, blip: Option<&Attr>) -> Element {
    let mut e = Element::empty(Tag::Image)
        .with("href", Value::Text(href.to_owned()))
        .num("x", x)
        .num("y", y)
        .num("width", w)
        .num("height", h)
        .word("preserveAspectRatio", "none");
    e.attrs.extend(blip.cloned());
    e
}

/// A picture.
pub fn render_image(image: &Image, painter: &mut Painter<'_>) -> Element {
    let t = &image.transform;
    let (w, h) = (emu_to_px(t.extent_width), emu_to_px(t.extent_height));
    let href = format!("data:{};base64,{}", image.mime, image.data);
    let bounds = if turned(t, painter) {
        None
    } else {
        image_bounds(image, w, h)
    };
    let filter = render_effects(image.effects.as_ref(), painter, bounds);
    let blip = render_blip_effects(image.blip_effects.as_ref(), painter);
    let mut clip = None;
    if let Some(g) = image.geometry.as_ref().filter(|g| !is_plain_rect(g)) {
        let id = painter.new_id("clip");
        painter.add_def(
            Element::open(Tag::ClipPath)
                .word("id", &id)
                .holding(vec![render_geometry(g, w, h, false)]),
        );
        clip = Some(Attr {
            name: "clip-path",
            value: Value::Url(id),
        });
    }
    let mut inner = Vec::new();
    if let Some(crop) = image.src_rect.filter(|r| r.any()) {
        let kept_w = (1.0 - crop.left - crop.right).max(1e-6);
        let kept_h = (1.0 - crop.top - crop.bottom).max(1e-6);
        let (sw, sh) = (w / kept_w, h / kept_h);
        let id = painter.new_id("crop");
        painter.add_def(
            Element::open(Tag::ClipPath)
                .word("id", &id)
                .holding(vec![Element::empty(Tag::Rect).num("width", w).num("height", h)]),
        );
        inner.push(
            Element::open(Tag::G)
                .with("clip-path", Value::Url(id))
                .holding(vec![picture(
                    &href,
                    -crop.left * sw,
                    -crop.top * sh,
                    sw,
                    sh,
                    blip.as_ref(),
                )]),
        );
    } else if let Some(tile) = image.tile.as_ref().and_then(|tile| {
        let id = painter.new_id("imgtile");
        tile_pattern(&id, &href, &image.data, tile, Some((0.0, 0.0, w, h)), blip.as_slice()).map(|p| (id, p))
    }) {
        let (id, pattern) = tile;
        painter.add_def(pattern);
        inner.push(
            Element::empty(Tag::Rect)
                .num("width", w)
                .num("height", h)
                .with("fill", Value::Url(id)),
        );
    } else {
        let (mut left, mut top, mut dw, mut dh) = (0.0, 0.0, w, h);
        if let Some(s) = image.stretch.filter(|s| s.any()) {
            left = s.left * w;
            top = s.top * h;
            dw = (w * (1.0 - s.left - s.right)).max(1e-6);
            dh = (h * (1.0 - s.top - s.bottom)).max(1e-6);
        }
        inner.push(picture(&href, left, top, dw, dh, blip.as_ref()));
    }
    if let Some(outline) = &image.outline {
        let mut styles = vec![Attr {
            name: "fill",
            value: Value::Word("none".into()),
        }];
        styles.extend(outline_attrs(Some(outline), painter, None));
        let geometry = image.geometry.clone().unwrap_or_default();
        inner.push(styled(render_geometry(&geometry, w, h, scale_paths(painter)), styles));
    }
    let mut g = placed(t, filter);
    g.attrs.extend(clip);
    g.holding(inner)
}

/// A zero-width run at a paragraph's end size: what an empty cell's line is measured with.
const EMPTY_LINE: &str = "\u{200b}";

fn with_empty_lines(body: &TextBody) -> std::borrow::Cow<'_, TextBody> {
    if body.paragraphs.iter().flat_map(|p| &p.runs).any(|r| !r.text.is_empty()) {
        return std::borrow::Cow::Borrowed(body);
    }
    let mut out = body.clone();
    out.paragraphs = body
        .paragraphs
        .iter()
        .map(|p| Paragraph {
            runs: vec![TextRun {
                text: EMPTY_LINE.into(),
                properties: p.end_properties.clone().unwrap_or_else(RunProperties::default),
            }],
            ..p.clone()
        })
        .collect();
    std::borrow::Cow::Owned(out)
}

fn row_heights(table: &Table, painter: &Painter<'_>) -> Vec<f64> {
    let mut heights: Vec<f64> = table.rows.iter().map(|r| r.height).collect();
    for (r, row) in table.rows.iter().enumerate() {
        for (c, cell) in row.cells.iter().enumerate() {
            let Some(body) = &cell.text else { continue };
            if cell.h_merge || cell.v_merge || cell.row_span > 1 || c >= table.columns.len() {
                continue;
            }
            let width = table.columns.get(c).copied().unwrap_or(0.0) * cell.grid_span.max(1) as f64;
            let frame = Transform {
                extent_width: width,
                extent_height: heights.get(r).copied().unwrap_or(0.0),
                ..Transform::default()
            };
            if let Some(required) = sp_autofit_height(&with_empty_lines(body), &frame, painter, None, 0.0) {
                if let Some(slot) = heights.get_mut(r) {
                    *slot = required;
                }
            }
        }
    }
    heights
}

fn borders(b: &CellBorders, cell: (f64, f64, f64, f64), painter: &mut Painter<'_>) -> Vec<Element> {
    let (x, y, w, h) = cell;
    let edges = [
        (&b.top, (x, y, x + w, y)),
        (&b.bottom, (x, y + h, x + w, y + h)),
        (&b.left, (x, y, x, y + h)),
        (&b.right, (x + w, y, x + w, y + h)),
    ];
    let mut out = Vec::new();
    for (outline, (x1, y1, x2, y2)) in edges {
        let Some(outline) = outline.as_ref().filter(|o| o.fill.is_some()) else {
            continue;
        };
        let mut line = Element::empty(Tag::Line)
            .num("x1", x1)
            .num("y1", y1)
            .num("x2", x2)
            .num("y2", y2);
        line.attrs.extend(outline_attrs(Some(outline), painter, None));
        out.push(line);
    }
    out
}

fn offsets(sizes: impl Iterator<Item = f64>) -> Vec<f64> {
    let mut out = vec![0.0];
    let mut at = 0.0;
    for size in sizes {
        at += emu_to_px(size);
        out.push(at);
    }
    out
}

/// A table: the style's background, each cell's fill and text, then every border.
pub fn render_table(table: &Table, painter: &mut Painter<'_>) -> Element {
    let filter = render_effects(table.effects.as_ref(), painter, None);
    let mut g = placed(&table.transform, filter);
    let columns = offsets(table.columns.iter().copied());
    let heights = row_heights(table, painter);
    let rows = offsets(heights.iter().copied());
    let last = |v: &[f64]| v.last().copied().unwrap_or(0.0);
    if let Some(background) = &table.background {
        let (w, h) = (last(&columns), last(&rows));
        let mut r = Element::empty(Tag::Rect)
            .word("x", "0")
            .word("y", "0")
            .num("width", w)
            .num("height", h);
        r.attrs
            .extend(fill_attrs(Some(background), painter, Some((0.0, 0.0, w, h))));
        g.push(r);
    }
    let mut lines = Vec::new();
    for (r, row) in table.rows.iter().enumerate() {
        for (c, cell) in row.cells.iter().enumerate() {
            if cell.h_merge || cell.v_merge || c + 1 >= columns.len() {
                continue;
            }
            let at = |v: &[f64], i: usize| v.get(i).copied().unwrap_or(0.0);
            let (x, y) = (at(&columns, c), at(&rows, r));
            let end_c = usize::try_from(c as i64 + cell.grid_span)
                .unwrap_or(c)
                .min(columns.len() - 1);
            let end_r = usize::try_from(r as i64 + cell.row_span)
                .unwrap_or(r)
                .min(rows.len() - 1);
            let (w, h) = (at(&columns, end_c) - x, at(&rows, end_r) - y);
            cell_contents(table, (r, c), (x, y, w, h), &heights, painter, &mut g, &mut lines);
        }
    }
    for line in lines {
        g.push(line);
    }
    g
}

fn cell_contents(
    table: &Table,
    at: (usize, usize),
    cell_box: (f64, f64, f64, f64),
    heights: &[f64],
    painter: &mut Painter<'_>,
    g: &mut Element,
    lines: &mut Vec<Element>,
) {
    let (r, c) = at;
    let Some(cell) = table.rows.get(r).and_then(|row| row.cells.get(c)) else {
        return;
    };
    let (x, y, w, h) = cell_box;
    let marked = painter.edit_marks && table.meta.element_id.is_some();
    let mut target = Vec::new();
    if let Some(fill) = &cell.fill {
        let mut rect = Element::empty(Tag::Rect)
            .num("x", x)
            .num("y", y)
            .num("width", w)
            .num("height", h);
        rect.attrs.extend(fill_attrs(Some(fill), painter, Some(cell_box)));
        target.push(rect);
    } else if marked {
        target.push(
            Element::empty(Tag::Rect)
                .num("x", x)
                .num("y", y)
                .num("width", w)
                .num("height", h)
                .word("fill", "none"),
        );
    }
    if let Some(b) = &cell.borders {
        lines.extend(borders(b, cell_box, painter));
    }
    if let Some(body) = &cell.text {
        let span = usize::try_from(cell.row_span.max(1)).unwrap_or(1);
        let frame = Transform {
            extent_width: table.columns.get(c).copied().unwrap_or(0.0) * cell.grid_span.max(1) as f64,
            extent_height: heights.iter().skip(r).take(span).sum(),
            ..Transform::default()
        };
        let text = render_text_body(body, &frame, painter, None, 0.0);
        if !text.is_empty() {
            let mut tg = Element::open(Tag::G).with(
                "transform",
                Value::Transform(vec![Op::Translate(x, y, Sep::CommaSpace)]),
            );
            if marked {
                tg.set("data-pptx-text", Value::Word("cell".into()));
            }
            target.push(tg.holding(text));
        }
    }
    match (&table.meta.element_id, marked) {
        (Some(id), true) => g.push(
            Element::open(Tag::G)
                .with("data-pptx-id", Value::Text(format!("{id}/cell{r},{c}")))
                .holding(target),
        ),
        _ => {
            for e in target {
                g.push(e);
            }
        }
    }
}
