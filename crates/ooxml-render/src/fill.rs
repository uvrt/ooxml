//! Owns: a fill's and an outline's paint as attributes, and the resources they reference
//! (ooxml-common's `drawingml.fill`): solid colours, gradients (over the box as pptx2svg
//! draws them, or Word's measured geometry, held to the page through the element's
//! [`ShapeFrame`]), picture fills and tiles, pattern fills of the measured 8 pt cell
//! (laid from the page's corner where the rules say so), dashes, and connector arrowheads as
//! markers -- all at the painter's dpi.
//!
//! Must not: draw the shape the paint goes on, or resolve a colour.

use crate::context::Painter;
use crate::image::{base64_decode, cell_rectangles, natural_size_pt, PATTERN_CELL_BITS, PATTERN_CELL_PT};
use crate::rules::{Dashes, Gradients, PatternPhase};
use crate::units::{emu_to_px_at, px_per_pt};
use ooxml_drawingml::drawable::{ArrowEnd, Colour, Fill, GradientFill, GradientStop, ImageTile, Outline};
use ooxml_scene::markup::{Attr, Element, Op, Sep, Tag, Value};

/// A rectangle, px: `x, y, width, height`.
pub type Box4 = (f64, f64, f64, f64);

/// How the element a fill is referenced from is placed on its page, for the rules that
/// measured a fill against the page rather than the shape (ooxml-common's `ShapeFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeFrame {
    /// Its rotation, degrees: a gradient with `rotWithShape="0"` keeps its angle to the page.
    pub rotation: f64,
    /// Flipped across the vertical axis.
    pub flip_h: bool,
    /// Flipped across the horizontal axis.
    pub flip_v: bool,
    /// From the page's coordinates into the element's user space: a pattern anchored to
    /// the page is laid in the page's.
    pub page_transform: Option<Vec<Op>>,
    /// Whether the shape's outline is its box: a `shape` path gradient draws rings of the
    /// outline, rectangles on a rectangle and ellipses on an ellipse (measured).
    pub rectangular: bool,
}

impl Default for ShapeFrame {
    fn default() -> ShapeFrame {
        ShapeFrame {
            rotation: 0.0,
            flip_h: false,
            flip_v: false,
            page_transform: None,
            rectangular: true,
        }
    }
}

fn attr(name: &'static str, value: Value) -> Attr {
    Attr { name, value }
}

fn word(name: &'static str, value: &str) -> Attr {
    attr(name, Value::Word(value.to_owned()))
}

/// `fill-opacity` where the colour is translucent.
pub fn opacity(name: &'static str, colour: &Colour) -> Option<Attr> {
    (colour.alpha < 1.0).then(|| attr(name, Value::Num(colour.alpha)))
}

/// `fill="..."` (and `fill-opacity`) for a fill over `bounds`, on an element placed on its
/// page by `frame` where the caller knows it.
pub fn fill_attrs(
    fill: Option<&Fill>,
    painter: &mut Painter<'_>,
    bounds: Option<Box4>,
    frame: Option<&ShapeFrame>,
) -> Vec<Attr> {
    match fill {
        None | Some(Fill::None) => vec![word("fill", "none")],
        Some(Fill::Solid(c)) => {
            let mut out = vec![word("fill", &c.hex)];
            out.extend(opacity("fill-opacity", c));
            out
        }
        Some(Fill::Gradient(g)) => {
            let id = match (painter.rules.gradients, bounds) {
                (Gradients::Office, Some(b)) => office_gradient(g, painter, b, frame),
                _ => box_gradient(g, painter),
            };
            vec![attr("fill", Value::Url(id))]
        }
        Some(Fill::Image { data, mime, tile }) => vec![attr(
            "fill",
            Value::Url(image_fill(data, mime, tile.as_ref(), painter, bounds)),
        )],
        Some(Fill::Pattern {
            preset,
            foreground,
            background,
        }) => {
            let page = frame.filter(|_| painter.rules.pattern_phase == PatternPhase::Page);
            pattern_fill(preset, foreground, background, painter, page)
        }
    }
}

fn stop(position: f64, colour: &Colour) -> Element {
    let mut e = Element::empty(Tag::Stop)
        .with("offset", Value::Percent(position * 100.0))
        .word("stop-color", &colour.hex);
    if colour.alpha < 1.0 {
        e = e.num("stop-opacity", colour.alpha);
    }
    e
}

fn box_gradient(g: &GradientFill, painter: &mut Painter<'_>) -> String {
    let id = painter.new_id("grad");
    let stops: Vec<Element> = g.stops.iter().map(|s| stop(s.position, &s.colour)).collect();
    if g.gradient_type == "radial" {
        let cx = g.center_x.unwrap_or(0.5) * 100.0;
        let cy = g.center_y.unwrap_or(0.5) * 100.0;
        let r = cx.max(100.0 - cx).hypot(cy.max(100.0 - cy));
        painter.add_def(
            Element::open(Tag::RadialGradient)
                .word("id", &id)
                .with("cx", Value::Percent(cx))
                .with("cy", Value::Percent(cy))
                .with("r", Value::Percent(r))
                .holding(stops),
        );
        return id;
    }
    let radians = g.angle.to_radians();
    painter.add_def(
        Element::open(Tag::LinearGradient)
            .word("id", &id)
            .with("x1", Value::Percent(50.0 - radians.cos() * 50.0))
            .with("y1", Value::Percent(50.0 - radians.sin() * 50.0))
            .with("x2", Value::Percent(50.0 + radians.cos() * 50.0))
            .with("y2", Value::Percent(50.0 + radians.sin() * 50.0))
            .holding(stops),
    );
    id
}

/// Word writes a two-stop 0-100 % gradient in linear light and eases it: this many stops.
const LINEAR_LIGHT_STOPS: u32 = 16;
const LINEAR_LIGHT_GAMMA: f64 = 2.2;
/// A circle path gradient's widening when its point is as far out as a corner.
const CIRCLE_CLEARANCE: f64 = 1_023_056.0 / 1_006_001.0;

fn channel(hex: &str, i: usize) -> f64 {
    let v = hex
        .get(1 + 2 * i..3 + 2 * i)
        .and_then(|t| u8::from_str_radix(t, 16).ok())
        .unwrap_or(0);
    f64::from(v) / 255.0
}

fn office_stops(stops: &[GradientStop]) -> Vec<(f64, Colour)> {
    let mut ordered: Vec<(f64, Colour)> = stops
        .iter()
        .map(|s| (s.position, Colour::opaque(&s.colour.hex)))
        .collect();
    ordered.sort_by(|a, b| a.0.total_cmp(&b.0));
    let [(p0, first), (p1, last)] = ordered.as_slice() else {
        return ordered;
    };
    if *p0 != 0.0 || *p1 != 1.0 {
        return ordered;
    }
    let start: Vec<f64> = (0..3)
        .map(|i| channel(&first.hex, i).powf(LINEAR_LIGHT_GAMMA))
        .collect();
    let end: Vec<f64> = (0..3).map(|i| channel(&last.hex, i).powf(LINEAR_LIGHT_GAMMA)).collect();
    (0..=LINEAR_LIGHT_STOPS)
        .map(|index| {
            let t = f64::from(index) / f64::from(LINEAR_LIGHT_STOPS);
            let eased = (1.0 - (std::f64::consts::PI * t).cos()) / 2.0;
            let hex: String = start
                .iter()
                .zip(&end)
                .map(|(a, b)| {
                    let c = (a + (b - a) * eased).powf(1.0 / LINEAR_LIGHT_GAMMA);
                    format!("{:02x}", crate::units::byte(c * 255.0))
                })
                .collect();
            (t, Colour::opaque(&format!("#{hex}")))
        })
        .collect()
}

fn office_gradient(g: &GradientFill, painter: &mut Painter<'_>, bounds: Box4, frame: Option<&ShapeFrame>) -> String {
    let stops = office_stops(&g.stops);
    let (x, y, width, height) = bounds;
    let path = (g.gradient_type == "radial").then(|| g.path.clone().unwrap_or_else(|| "circle".into()));
    if path.as_deref() == Some("shape") && frame.is_some_and(|f| !f.rectangular) {
        return shape_rings(g, &stops, painter);
    }
    if matches!(path.as_deref(), Some("rect" | "shape")) {
        return rectangular_gradient(g, &stops, painter, bounds);
    }
    let id = painter.new_id("grad");
    let markup: Vec<Element> = stops.iter().map(|(p, c)| stop(*p, c)).collect();
    if g.gradient_type == "radial" {
        let [left, top, right, bottom] = g.focus.unwrap_or([0.5; 4]);
        let fx = x + width * (left + (1.0 - right)) / 2.0;
        let fy = y + height * (top + (1.0 - bottom)) / 2.0;
        let (cx, cy) = (x + width / 2.0, y + height / 2.0);
        let radius = (width / 2.0)
            .hypot(height / 2.0)
            .max((fx - cx).hypot(fy - cy) * CIRCLE_CLEARANCE);
        painter.add_def(
            Element::open(Tag::RadialGradient)
                .word("id", &id)
                .word("gradientUnits", "userSpaceOnUse")
                .num("cx", cx)
                .num("cy", cy)
                .num("r", radius)
                .num("fx", fx)
                .num("fy", fy)
                .holding(markup),
        );
        return id;
    }
    let radians = page_angle(g, frame).to_radians();
    let (dx, dy) = if g.scaled == Some(true) {
        (height * radians.cos(), width * radians.sin())
    } else {
        (radians.cos(), radians.sin())
    };
    let hypot = dx.hypot(dy);
    let length = if hypot == 0.0 { 1.0 } else { hypot };
    let (dx, dy) = (dx / length, dy / length);
    let half = (width * dx.abs() + height * dy.abs()) / 2.0;
    let (cx, cy) = (x + width / 2.0, y + height / 2.0);
    painter.add_def(
        Element::open(Tag::LinearGradient)
            .word("id", &id)
            .word("gradientUnits", "userSpaceOnUse")
            .num("x1", cx - dx * half)
            .num("y1", cy - dy * half)
            .num("x2", cx + dx * half)
            .num("y2", cy + dy * half)
            .holding(markup),
    );
    id
}

/// A gradient's angle in the element's user space: its own, or, held to the page
/// (`rotWithShape="0"`), with the element's rotation undone and then its flips (the element
/// rotates after flipping, so its inverse flips after unrotating).
fn page_angle(g: &GradientFill, frame: Option<&ShapeFrame>) -> f64 {
    let mut angle = g.angle;
    if let (Some(false), Some(frame)) = (g.rotate_with_shape, frame) {
        angle -= frame.rotation;
        if frame.flip_h {
            angle = 180.0 - angle;
        }
        if frame.flip_v {
            angle = -angle;
        }
    }
    angle
}

/// A `shape` path gradient on an outline that is not its box: rings of the box's
/// inscribed ellipses (measured on an ellipse), a radial gradient over the box.
fn shape_rings(g: &GradientFill, stops: &[(f64, Colour)], painter: &mut Painter<'_>) -> String {
    let id = painter.new_id("grad");
    let [left, top, right, bottom] = g.focus.unwrap_or([0.5; 4]);
    painter.add_def(
        Element::open(Tag::RadialGradient)
            .word("id", &id)
            .word("cx", "0.5")
            .word("cy", "0.5")
            .word("r", "0.5")
            .num("fx", (left + 1.0 - right) / 2.0)
            .num("fy", (top + 1.0 - bottom) / 2.0)
            .holding(stops.iter().map(|(p, c)| stop(*p, c)).collect()),
    );
    id
}

fn rectangular_gradient(g: &GradientFill, stops: &[(f64, Colour)], painter: &mut Painter<'_>, bounds: Box4) -> String {
    let (x, y, width, height) = bounds;
    let [left, top, right, bottom] = g.focus.unwrap_or([0.5; 4]);
    let (mut fl, mut ft) = (width * left, height * top);
    let (mut fr, mut fb) = (width * (1.0 - right), height * (1.0 - bottom));
    if fr < fl {
        fl = (fl + fr) / 2.0;
        fr = fl;
    }
    if fb < ft {
        ft = (ft + fb) / 2.0;
        fb = ft;
    }
    let id = painter.new_id("grad");
    let middle = (fl + fr) / 2.0;
    let pieces = [
        (
            "l",
            [(0.0, 0.0), (middle, 0.0), (middle, height), (0.0, height)],
            (fl, 0.0, 0.0, 0.0),
        ),
        (
            "r",
            [(middle, 0.0), (width, 0.0), (width, height), (middle, height)],
            (fr, 0.0, width, 0.0),
        ),
        ("t", [(0.0, 0.0), (width, 0.0), (fr, ft), (fl, ft)], (0.0, ft, 0.0, 0.0)),
        (
            "b",
            [(0.0, height), (width, height), (fr, fb), (fl, fb)],
            (0.0, fb, 0.0, height),
        ),
    ];
    let mut parts = Vec::new();
    for (name, polygon, (x1, y1, x2, y2)) in pieces {
        if (x1, y1) == (x2, y2) {
            continue;
        }
        let gid = format!("{id}-{name}");
        parts.push(
            Element::open(Tag::LinearGradient)
                .word("id", &gid)
                .word("gradientUnits", "userSpaceOnUse")
                .num("x1", x1)
                .num("y1", y1)
                .num("x2", x2)
                .num("y2", y2)
                .holding(stops.iter().map(|(p, c)| stop(*p, c)).collect()),
        );
        parts.push(
            Element::empty(Tag::Polygon)
                .with("points", Value::Points(polygon.to_vec()))
                .with("fill", Value::Url(gid)),
        );
    }
    if let Some((_, first)) = stops.first() {
        if fr > fl && fb > ft {
            let mut r = Element::empty(Tag::Rect)
                .num("x", fl)
                .num("y", ft)
                .num("width", fr - fl)
                .num("height", fb - ft)
                .word("fill", &first.hex);
            r.attrs.extend(opacity("fill-opacity", first));
            parts.push(r);
        }
    }
    painter.add_def(
        Element::open(Tag::Pattern)
            .word("id", &id)
            .word("patternUnits", "userSpaceOnUse")
            .num("x", x)
            .num("y", y)
            .num("width", width)
            .num("height", height)
            .holding(parts),
    );
    id
}

fn image_fill(
    data: &str,
    mime: &str,
    tile: Option<&ImageTile>,
    painter: &mut Painter<'_>,
    bounds: Option<Box4>,
) -> String {
    let id = painter.new_id("imgfill");
    let href = format!("data:{mime};base64,{data}");
    if let Some(tile) = tile {
        if let Some(def) = tile_pattern(&id, &href, data, tile, bounds, &[], painter.dpi) {
            painter.add_def(def);
            return id;
        }
    }
    painter.add_def(
        Element::open(Tag::Pattern)
            .word("id", &id)
            .word("patternContentUnits", "objectBoundingBox")
            .word("width", "1")
            .word("height", "1")
            .holding(vec![Element::empty(Tag::Image)
                .with("href", Value::Text(href))
                .word("width", "1")
                .word("height", "1")
                .word("preserveAspectRatio", "none")]),
    );
    id
}

/// One `pattern` for `a:tile`: the picture's own size scaled, placed by the alignment,
/// mirrored copies in a doubled cell for a flip, at `dpi`; `None` where the picture's size
/// is unknown.
pub fn tile_pattern(
    id: &str,
    href: &str,
    data: &str,
    tile: &ImageTile,
    bounds: Option<Box4>,
    image_attrs: &[Attr],
    dpi: f64,
) -> Option<Element> {
    let natural = natural_size_pt(&base64_decode(data)?)?;
    let width = natural.0 * px_per_pt(dpi) * tile.sx;
    let height = natural.1 * px_per_pt(dpi) * tile.sy;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let (mirror_x, mirror_y) = match tile.flip.as_str() {
        "x" => (true, false),
        "y" => (false, true),
        "xy" => (true, true),
        _ => (false, false),
    };
    let (mut x, mut y) = tile_origin(&tile.align, bounds, width, height);
    x += emu_to_px_at(tile.tx, dpi);
    y += emu_to_px_at(tile.ty, dpi);
    let mut copies = vec![(0.0, 0.0, 1.0, 1.0)];
    if mirror_x {
        copies.push((2.0 * width, 0.0, -1.0, 1.0));
    }
    if mirror_y {
        copies.push((0.0, 2.0 * height, 1.0, -1.0));
    }
    if mirror_x && mirror_y {
        copies.push((2.0 * width, 2.0 * height, -1.0, -1.0));
    }
    let images = copies
        .into_iter()
        .map(|(ox, oy, sx, sy)| {
            let mut image = Element::empty(Tag::Image)
                .with("href", Value::Text(href.to_owned()))
                .num("width", width)
                .num("height", height)
                .word("preserveAspectRatio", "none");
            image.attrs.extend(image_attrs.iter().cloned());
            if (sx, sy) != (1.0, 1.0) {
                image.set(
                    "transform",
                    Value::Transform(vec![
                        Op::Translate(ox, oy, Sep::CommaSpace),
                        Op::Scale(sx, sy, Sep::CommaSpace),
                    ]),
                );
            }
            image
        })
        .collect();
    Some(
        Element::open(Tag::Pattern)
            .word("id", id)
            .word("patternUnits", "userSpaceOnUse")
            .num("x", x)
            .num("y", y)
            .num("width", width * if mirror_x { 2.0 } else { 1.0 })
            .num("height", height * if mirror_y { 2.0 } else { 1.0 })
            .holding(images),
    )
}

fn tile_origin(align: &str, bounds: Option<Box4>, width: f64, height: f64) -> (f64, f64) {
    let Some((left, top, bw, bh)) = bounds else {
        return (0.0, 0.0);
    };
    let x = match align {
        "tr" | "r" | "br" => left + bw - width,
        "t" | "ctr" | "b" => left + (bw - width) / 2.0,
        _ => left,
    };
    let y = match align {
        "bl" | "b" | "br" => top + bh - height,
        "l" | "ctr" | "r" => top + (bh - height) / 2.0,
        _ => top,
    };
    (x, y)
}

fn pattern_fill(
    preset: &str,
    fg: &Colour,
    bg: &Colour,
    painter: &mut Painter<'_>,
    page: Option<&ShapeFrame>,
) -> Vec<Attr> {
    let Some(rects) = cell_rectangles(preset) else {
        let mut out = vec![word("fill", &fg.hex)];
        out.extend(opacity("fill-opacity", fg));
        return out;
    };
    // Anchored to the shape, or (Word's) laid in the page's coordinates, carried into the
    // element's by its frame's page transform where the painter knows it.
    let cell = PATTERN_CELL_PT * px_per_pt(painter.dpi);
    let unit = cell / f64::from(PATTERN_CELL_BITS);
    let id = painter.new_id("patt");
    let mut background = Element::empty(Tag::Rect)
        .num("width", cell)
        .num("height", cell)
        .word("fill", &bg.hex);
    background.attrs.extend(opacity("fill-opacity", bg));
    let mut children = vec![background];
    for (x, y, w, h) in rects {
        let mut r = Element::empty(Tag::Rect)
            .num("x", f64::from(x) * unit)
            .num("y", f64::from(y) * unit)
            .num("width", f64::from(w) * unit)
            .num("height", f64::from(h) * unit)
            .word("fill", &fg.hex);
        r.attrs.extend(opacity("fill-opacity", fg));
        children.push(r);
    }
    let mut pattern = Element::open(Tag::Pattern)
        .word("id", &id)
        .word("patternUnits", "userSpaceOnUse")
        .num("width", cell)
        .num("height", cell);
    if let Some(ops) = page
        .and_then(|f| f.page_transform.as_ref())
        .filter(|ops| !ops.is_empty())
    {
        pattern.set("patternTransform", Value::Transform(ops.clone()));
    }
    painter.add_def(pattern.holding(children));
    vec![attr("fill", Value::Url(id))]
}

/// A preset dash as multiples of the width.
fn dash_pattern(name: &str) -> Option<&'static [f64]> {
    Some(match name {
        "dash" => &[4.0, 3.0],
        "dot" => &[1.0, 3.0],
        "dashDot" => &[4.0, 3.0, 1.0, 3.0],
        "lgDash" => &[8.0, 3.0],
        "lgDashDot" => &[8.0, 3.0, 1.0, 3.0],
        "lgDashDotDot" => &[8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
        "sysDash" => &[3.0, 1.0],
        "sysDot" => &[1.0, 1.0],
        "sysDashDot" => &[3.0, 1.0, 1.0, 1.0],
        "sysDashDotDot" => &[3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        _ => return None,
    })
}

fn dash_lengths(outline: &Outline, width: f64, dashes: Dashes) -> Option<Vec<f64>> {
    let units: Vec<f64> = match &outline.custom_dash {
        Some(custom) if !custom.is_empty() => custom.clone(),
        _ if outline.dash_style != "solid" => dash_pattern(&outline.dash_style)?.to_vec(),
        _ => return None,
    };
    let lengths = units.iter().map(|v| v * width);
    if dashes == Dashes::Office && outline.line_cap.as_deref() == Some("round") {
        return Some(
            lengths
                .enumerate()
                .map(|(i, v)| if i % 2 == 0 { (v - width).max(0.0) } else { v + width })
                .collect(),
        );
    }
    Some(lengths.collect())
}

/// `stroke`, `stroke-width`, dashes, cap and join for an outline over `bounds`, on an
/// element placed on its page by `frame` where the caller knows it.
pub fn outline_attrs(
    outline: Option<&Outline>,
    painter: &mut Painter<'_>,
    bounds: Option<Box4>,
    frame: Option<&ShapeFrame>,
) -> Vec<Attr> {
    let Some(outline) = outline else {
        return vec![word("stroke", "none")];
    };
    let width = emu_to_px_at(outline.width, painter.dpi);
    let mut out = vec![attr("stroke-width", Value::Num(width))];
    match &outline.fill {
        None => out.push(word("stroke", "none")),
        Some(Fill::Solid(c)) => {
            out.push(word("stroke", &c.hex));
            out.extend(opacity("stroke-opacity", c));
        }
        Some(Fill::Gradient(g)) => {
            let id = match (painter.rules.gradients, bounds) {
                (Gradients::Office, Some(b)) => {
                    let half = width / 2.0;
                    office_gradient(g, painter, (b.0 - half, b.1 - half, b.2 + width, b.3 + width), frame)
                }
                _ => box_gradient(g, painter),
            };
            out.push(attr("stroke", Value::Url(id)));
        }
        Some(_) => {}
    }
    let mut cap = outline.line_cap.clone();
    if let Some(dashes) = dash_lengths(outline, width, painter.rules.dashes) {
        out.push(attr("stroke-dasharray", Value::Nums(dashes)));
        let custom = outline.custom_dash.as_ref().is_some_and(|d| !d.is_empty());
        if painter.rules.dashes == Dashes::Office && cap.as_deref() == Some("square") && !custom {
            cap = None;
        }
    }
    if let Some(cap) = cap.filter(|c| !c.is_empty()) {
        out.push(word("stroke-linecap", &cap));
    }
    let join = outline
        .line_join
        .clone()
        .filter(|j| !j.is_empty())
        .or_else(|| painter.rules.default_join.map(str::to_owned));
    if let Some(join) = join {
        out.push(word("stroke-linejoin", &join));
    }
    out
}

const fn arrow_size(size: &str) -> Option<f64> {
    match size.as_bytes() {
        b"sm" => Some(5.0),
        b"med" => Some(8.0),
        b"lg" => Some(12.0),
        _ => None,
    }
}

/// `marker-start` and `marker-end` for a connector's arrowheads.
pub fn markers(outline: Option<&Outline>, painter: &mut Painter<'_>) -> Vec<Attr> {
    let Some(outline) = outline else { return Vec::new() };
    if outline.head_end.is_none() && outline.tail_end.is_none() {
        return Vec::new();
    }
    let (colour, alpha) = match &outline.fill {
        Some(Fill::Solid(c)) => (c.hex.clone(), c.alpha),
        Some(Fill::Gradient(g)) if !g.stops.is_empty() => g
            .stops
            .first()
            .map_or(("#000000".into(), 1.0), |s| (s.colour.hex.clone(), s.colour.alpha)),
        _ => ("#000000".into(), 1.0),
    };
    let mut out = Vec::new();
    for (end, name) in [(&outline.head_end, "marker-start"), (&outline.tail_end, "marker-end")] {
        let Some(end) = end else { continue };
        let id = painter.new_id("marker");
        if let Some(def) = marker(&id, end, &colour, alpha) {
            painter.add_def(def);
            out.push(attr(name, Value::Url(id)));
        }
    }
    out
}

fn marker(id: &str, end: &ArrowEnd, colour: &str, alpha: f64) -> Option<Element> {
    let (mw, mh) = (arrow_size(&end.length)?, arrow_size(&end.width)?);
    let fade = |e: Element| if alpha < 1.0 { e.num("opacity", alpha) } else { e };
    let open = |ref_x: f64| {
        Element::open(Tag::Marker)
            .word("id", id)
            .num("markerWidth", mw)
            .num("markerHeight", mh)
            .num("refX", ref_x)
            .num("refY", mh / 2.0)
            .word("orient", "auto")
            .word("markerUnits", "userSpaceOnUse")
    };
    let p = |letter: char, x: f64, y: f64| ooxml_scene::markup::Command::new(letter, vec![x, y]);
    let z = ooxml_scene::markup::Command::new('Z', vec![]);
    let (data, stroked) = match end.kind.as_str() {
        "oval" => {
            let e = Element::empty(Tag::Ellipse)
                .num("cx", mw / 2.0)
                .num("cy", mh / 2.0)
                .num("rx", mw / 2.0)
                .num("ry", mh / 2.0)
                .word("fill", colour);
            return Some(open(mw / 2.0).holding(vec![fade(e)]));
        }
        "triangle" => (vec![p('M', 0.0, 0.0), p('L', mw, mh / 2.0), p('L', 0.0, mh), z], false),
        "stealth" => (
            vec![
                p('M', 0.0, 0.0),
                p('L', mw, mh / 2.0),
                p('L', 0.0, mh),
                p('L', mw * 0.3, mh / 2.0),
                z,
            ],
            false,
        ),
        "diamond" => (
            vec![
                p('M', 0.0, mh / 2.0),
                p('L', mw / 2.0, 0.0),
                p('L', mw, mh / 2.0),
                p('L', mw / 2.0, mh),
                z,
            ],
            false,
        ),
        "arrow" => (vec![p('M', 0.0, 0.0), p('L', mw, mh / 2.0), p('L', 0.0, mh)], true),
        _ => return None,
    };
    let mut path = Element::empty(Tag::Path).with("d", Value::Path(data));
    path = if stroked {
        path.word("fill", "none")
            .word("stroke", colour)
            .word("stroke-width", "1")
    } else {
        path.word("fill", colour)
    };
    Some(open(mw).holding(vec![fade(path)]))
}
