//! Owns: a geometry as one element to paint (ooxml-common's `geometry.render_geometry`):
//! PowerPoint's hand-drawn presets (a `rect` as a rectangle, a `diamond` as a polygon), the
//! other presets evaluated from the specification's guides and paths, a custom geometry's
//! paths scaled onto the box; and the rectangle a geometry lays its text out in
//! (`text_rect`, `text_area`).
//!
//! Must not: paint (the caller puts fill and stroke on the element), or hold a preset's
//! definition (`ooxml_drawingml::presets` does).

use crate::rules::DrawingRules;
use ooxml_drawingml::drawable::{CustomPath, Geometry, TextRect};
use ooxml_drawingml::guides::{arc_segments, evaluate_guides, resolve_value, Variables};
use ooxml_drawingml::presets::{preset, Preset};
use ooxml_drawingml::source::{SpecCommand, SpecPath};
use ooxml_scene::markup::{Command, Element, Op, Sep, Tag, Value};

/// The presets PowerPoint's renderer draws by hand (pptx2svg's choice, measured against
/// PowerPoint); every other known preset is drawn from the specification.
const HAND_DRAWN: [&str; 32] = [
    "rect",
    "ellipse",
    "roundRect",
    "rtTriangle",
    "diamond",
    "line",
    "upArrow",
    "homePlate",
    "straightConnector1",
    "flowChartProcess",
    "flowChartAlternateProcess",
    "flowChartDecision",
    "flowChartInputOutput",
    "flowChartPredefinedProcess",
    "flowChartInternalStorage",
    "flowChartPreparation",
    "flowChartManualInput",
    "flowChartManualOperation",
    "flowChartConnector",
    "flowChartOffpageConnector",
    "flowChartPunchedCard",
    "flowChartCollate",
    "flowChartSort",
    "flowChartExtract",
    "flowChartMerge",
    "wedgeEllipseCallout",
    "borderCallout1",
    "borderCallout2",
    "round1Rect",
    "round2SameRect",
    "leftBrace",
    "rightBrace",
];

/// Misspellings real files carry, and the preset they mean.
pub(crate) fn alias(name: &str) -> &str {
    if name == "bendUpArrow" {
        "bentUpArrow"
    } else {
        name
    }
}

/// A geometry over a box of `width` by `height` px as one element.
pub fn render_geometry(geometry: &Geometry, width: f64, height: f64, scale_paths: bool) -> Element {
    match geometry {
        Geometry::Preset { name, adjust } => preset_element(name, width, height, adjust),
        Geometry::Custom(paths) if !paths.is_empty() => custom(paths, width, height, scale_paths),
        Geometry::Custom(_) => rect(width, height),
    }
}

fn rect(w: f64, h: f64) -> Element {
    Element::empty(Tag::Rect).num("width", w).num("height", h)
}

fn adjust_ratio(adjust: &[(String, f64)], name: &str, default: f64) -> f64 {
    adjust.iter().find(|(k, _)| k == name).map_or(default, |(_, v)| *v) / 100_000.0
}

fn clamp_half(value: f64) -> f64 {
    0.5_f64.min(0.0_f64.max(value))
}

/// One preset as an element: hand-drawn, from the specification, or a rectangle.
pub fn preset_element(name: &str, w: f64, h: f64, adjust: &[(String, f64)]) -> Element {
    let name = alias(name);
    if HAND_DRAWN.contains(&name) {
        return hand_drawn(name, w, h, adjust);
    }
    match preset(name) {
        Some(spec) => spec_geometry(spec, w, h, adjust),
        None => rect(w, h),
    }
}

fn polygon(points: &[(f64, f64)]) -> Element {
    Element::empty(Tag::Polygon).with("points", Value::Points(points.to_vec()))
}

fn path(commands: Vec<Command>) -> Element {
    Element::empty(Tag::Path).with("d", Value::Path(commands))
}

fn c(letter: char, operands: &[f64]) -> Command {
    Command::new(letter, operands.to_vec())
}

fn arc(rx: f64, ry: f64, large: f64, sweep: f64, x: f64, y: f64) -> Command {
    c('A', &[rx, ry, 0.0, large, sweep, x, y])
}

fn boxed() -> Vec<Command> {
    Vec::new()
}

fn frame(w: f64, h: f64, out: &mut Vec<Command>) {
    out.extend([
        c('M', &[0.0, 0.0]),
        c('L', &[w, 0.0]),
        c('L', &[w, h]),
        c('L', &[0.0, h]),
        c('Z', &[]),
    ]);
}

fn hand_drawn(name: &str, w: f64, h: f64, adj: &[(String, f64)]) -> Element {
    match name {
        "rect" | "flowChartProcess" => rect(w, h),
        "ellipse" | "flowChartConnector" => Element::empty(Tag::Ellipse)
            .num("cx", w / 2.0)
            .num("cy", h / 2.0)
            .num("rx", w / 2.0)
            .num("ry", h / 2.0),
        "roundRect" => {
            let r = clamp_half(adjust_ratio(adj, "adj", 16667.0)) * w.min(h);
            rect(w, h).num("rx", r).num("ry", r)
        }
        "flowChartAlternateProcess" => {
            let r = w.min(h) / 6.0;
            rect(w, h).num("rx", r).num("ry", r)
        }
        "rtTriangle" => polygon(&[(0.0, 0.0), (w, h), (0.0, h)]),
        "diamond" | "flowChartDecision" => polygon(&[(w / 2.0, 0.0), (w, h / 2.0), (w / 2.0, h), (0.0, h / 2.0)]),
        "line" => Element::empty(Tag::Line)
            .num("x1", 0.0)
            .num("y1", 0.0)
            .num("x2", w)
            .num("y2", h),
        "straightConnector1" => path(vec![c('M', &[0.0, 0.0]), c('L', &[w, h])]),
        _ => arrows_and_flows(name, w, h, adj),
    }
}

fn arrows_and_flows(name: &str, w: f64, h: f64, adj: &[(String, f64)]) -> Element {
    match name {
        "upArrow" => {
            let head_w = adjust_ratio(adj, "adj1", 50000.0) * w;
            let head_l = adjust_ratio(adj, "adj2", 50000.0) * h;
            let left = (w - head_w) / 2.0;
            let right = w - left;
            polygon(&[
                (left, head_l),
                (0.0, head_l),
                (w / 2.0, 0.0),
                (w, head_l),
                (right, head_l),
                (right, h),
                (left, h),
            ])
        }
        "homePlate" => {
            let offset = adjust_ratio(adj, "adj", 50000.0) * w.min(h);
            polygon(&[(0.0, 0.0), (w - offset, 0.0), (w, h / 2.0), (w - offset, h), (0.0, h)])
        }
        "flowChartInputOutput" => {
            let o = w / 5.0;
            polygon(&[(o, 0.0), (w, 0.0), (w - o, h), (0.0, h)])
        }
        "flowChartPreparation" => {
            let o = w / 5.0;
            polygon(&[(o, 0.0), (w - o, 0.0), (w, h / 2.0), (w - o, h), (o, h), (0.0, h / 2.0)])
        }
        "flowChartManualInput" => polygon(&[(0.0, h / 5.0), (w, 0.0), (w, h), (0.0, h)]),
        "flowChartManualOperation" => {
            let o = w / 5.0;
            polygon(&[(0.0, 0.0), (w, 0.0), (w - o, h), (o, h)])
        }
        "flowChartOffpageConnector" => {
            let a = h * 0.2;
            polygon(&[(0.0, 0.0), (w, 0.0), (w, h - a), (w / 2.0, h), (0.0, h - a)])
        }
        "flowChartPunchedCard" => {
            let cut = w.min(h) * 0.2;
            polygon(&[(cut, 0.0), (w, 0.0), (w, h), (0.0, h), (0.0, cut)])
        }
        "flowChartCollate" => polygon(&[
            (0.0, 0.0),
            (w, 0.0),
            (w / 2.0, h / 2.0),
            (w, h),
            (0.0, h),
            (w / 2.0, h / 2.0),
        ]),
        "flowChartExtract" => polygon(&[(w / 2.0, 0.0), (w, h), (0.0, h)]),
        "flowChartMerge" => polygon(&[(0.0, 0.0), (w, 0.0), (w / 2.0, h)]),
        _ => flow_paths(name, w, h, adj),
    }
}

fn flow_paths(name: &str, w: f64, h: f64, adj: &[(String, f64)]) -> Element {
    match name {
        "flowChartPredefinedProcess" => {
            let d = w / 8.0;
            let mut out = boxed();
            frame(w, h, &mut out);
            out.extend([
                c('M', &[d, 0.0]),
                c('L', &[d, h]),
                c('M', &[w - d, 0.0]),
                c('L', &[w - d, h]),
            ]);
            path(out)
        }
        "flowChartInternalStorage" => {
            let (dx, dy) = (w / 8.0, h / 8.0);
            let mut out = boxed();
            frame(w, h, &mut out);
            out.extend([
                c('M', &[dx, 0.0]),
                c('L', &[dx, h]),
                c('M', &[0.0, dy]),
                c('L', &[w, dy]),
            ]);
            path(out)
        }
        "flowChartSort" => path(vec![
            c('M', &[w / 2.0, 0.0]),
            c('L', &[w, h / 2.0]),
            c('L', &[0.0, h / 2.0]),
            c('Z', &[]),
            c('M', &[0.0, h / 2.0]),
            c('L', &[w, h / 2.0]),
            c('M', &[w / 2.0, h]),
            c('L', &[w, h / 2.0]),
            c('L', &[0.0, h / 2.0]),
            c('Z', &[]),
        ]),
        _ => callouts_and_braces(name, w, h, adj),
    }
}

fn callouts_and_braces(name: &str, w: f64, h: f64, adj: &[(String, f64)]) -> Element {
    let a = |key: &str, default: f64| adjust_ratio(adj, key, default);
    match name {
        "wedgeEllipseCallout" => {
            let tip_x = w / 2.0 + a("adj1", -20833.0) * w;
            let tip_y = h / 2.0 + a("adj2", 62500.0) * h;
            let (cx, cy, rx, ry) = (w / 2.0, h / 2.0, w / 2.0, h / 2.0);
            let angle = (tip_y - cy).atan2(tip_x - cx);
            let (x1, y1) = (cx + rx * (angle - 0.15).cos(), cy + ry * (angle - 0.15).sin());
            let (x2, y2) = (cx + rx * (angle + 0.15).cos(), cy + ry * (angle + 0.15).sin());
            path(vec![
                c('M', &[x1, y1]),
                c('L', &[tip_x, tip_y]),
                c('L', &[x2, y2]),
                arc(rx, ry, 1.0, 1.0, x1, y1),
                c('Z', &[]),
            ])
        }
        "borderCallout1" | "borderCallout2" => {
            let mut out = boxed();
            frame(w, h, &mut out);
            let y1 = a("adj1", 18750.0) * h;
            let x1 = a("adj2", -8333.0) * w;
            out.push(c('M', &[x1, y1]));
            if name == "borderCallout1" {
                out.push(c('L', &[a("adj4", -38333.0) * w, a("adj3", 112_500.0) * h]));
            } else {
                out.push(c('L', &[a("adj4", -16667.0) * w, a("adj3", 18750.0) * h]));
                out.push(c('L', &[a("adj6", -46667.0) * w, a("adj5", 112_500.0) * h]));
            }
            path(out)
        }
        "round1Rect" => {
            let r = clamp_half(a("adj", 16667.0)) * w.min(h);
            path(vec![
                c('M', &[0.0, 0.0]),
                c('L', &[w - r, 0.0]),
                arc(r, r, 0.0, 1.0, w, r),
                c('L', &[w, h]),
                c('L', &[0.0, h]),
                c('Z', &[]),
            ])
        }
        "round2SameRect" => {
            let r1 = clamp_half(a("adj1", 16667.0)) * w.min(h);
            let r2 = clamp_half(a("adj2", 0.0)) * w.min(h);
            path(vec![
                c('M', &[r1, 0.0]),
                c('L', &[w - r1, 0.0]),
                arc(r1, r1, 0.0, 1.0, w, r1),
                c('L', &[w, h - r2]),
                arc(r2, r2, 0.0, 1.0, w - r2, h),
                c('L', &[r2, h]),
                arc(r2, r2, 0.0, 1.0, 0.0, h - r2),
                c('L', &[0.0, r1]),
                arc(r1, r1, 0.0, 1.0, r1, 0.0),
                c('Z', &[]),
            ])
        }
        "leftBrace" | "rightBrace" => brace(name == "leftBrace", w, h, a("adj1", 8333.0) * h, a("adj2", 50000.0) * h),
        _ => rect(w, h),
    }
}

fn brace(left: bool, w: f64, h: f64, r: f64, mid: f64) -> Element {
    let half = w / 2.0;
    let (start, outer, s1, s2) = if left { (w, 0.0, 0.0, 1.0) } else { (0.0, w, 1.0, 0.0) };
    path(vec![
        c('M', &[start, 0.0]),
        arc(half, r, 0.0, s1, half, r),
        c('L', &[half, mid - r]),
        arc(half, r, 0.0, s2, outer, mid),
        arc(half, r, 0.0, s2, half, mid + r),
        c('L', &[half, h - r]),
        arc(half, r, 0.0, s1, start, h),
    ])
}

fn shade_overlay(fill: &str) -> Option<(&'static str, &'static str)> {
    match fill {
        "lighten" => Some(("#ffffff", "0.4")),
        "lightenLess" => Some(("#ffffff", "0.2")),
        "darken" => Some(("#000000", "0.4")),
        "darkenLess" => Some(("#000000", "0.2")),
        _ => None,
    }
}

fn path_overrides(p: &SpecPath, element: Element) -> Element {
    let mut e = element;
    if p.fill == "none" {
        e = e.word("fill", "none");
    }
    if !p.stroke {
        e = e.word("stroke", "none");
    }
    e
}

fn has_overrides(p: &SpecPath) -> bool {
    p.fill == "none" || !p.stroke
}

fn spec_geometry(spec: &Preset, w: f64, h: f64, adjust: &[(String, f64)]) -> Element {
    let adjustments: Vec<(String, String)> = spec
        .adjustments
        .iter()
        .map(|(name, default)| {
            let value = adjust.iter().find(|(k, _)| k == name).map_or(*default, |(_, v)| *v);
            (name.clone(), format!("val {value}"))
        })
        .collect();
    let variables = evaluate_guides(&[&adjustments, &spec.guides], w, h, true);
    let mut rendered: Vec<Element> = Vec::new();
    for p in &spec.paths {
        let data = spec_commands(&p.commands, &variables, path_scale(p, w, h), (0.0, 0.0));
        if data.is_empty() {
            continue;
        }
        rendered.push(path_overrides(p, path(data.clone())));
        if let Some((colour, opacity)) = shade_overlay(&p.fill) {
            rendered.push(
                path(data)
                    .word("fill", colour)
                    .word("fill-opacity", opacity)
                    .word("stroke", "none"),
            );
        }
    }
    if rendered.is_empty() {
        return rect(w, h);
    }
    let first_plain = spec.paths.first().is_some_and(|p| !has_overrides(p));
    if rendered.len() == 1 && first_plain {
        return rendered.remove(0);
    }
    Element::open(Tag::G).holding(rendered)
}

fn path_scale(p: &SpecPath, w: f64, h: f64) -> (f64, f64) {
    match p.space {
        None => (1.0, 1.0),
        Some((sw, sh)) => (
            if sw != 0.0 { w / sw } else { 1.0 },
            if sh != 0.0 { h / sh } else { 1.0 },
        ),
    }
}

/// A specification path's commands evaluated, scaled and offset, as path data
/// (`_spec_path_data`): `arcTo` converted from the pen's position.
pub fn spec_commands(
    commands: &[SpecCommand],
    variables: &Variables,
    scale: (f64, f64),
    offset: (f64, f64),
) -> Vec<Command> {
    let (sx, sy) = scale;
    let (ox, oy) = offset;
    let value = |i: usize, cmd: &SpecCommand| cmd.operands.get(i).map_or(0.0, |t| resolve_value(t, variables));
    let mut out = Vec::new();
    let (mut x, mut y, mut start_x, mut start_y) = (0.0, 0.0, 0.0, 0.0);
    for cmd in commands {
        match cmd.letter {
            'M' | 'L' => {
                x = value(0, cmd);
                y = value(1, cmd);
                out.push(c(cmd.letter, &[x * sx + ox, y * sy + oy]));
                if cmd.letter == 'M' {
                    start_x = x;
                    start_y = y;
                }
            }
            'Z' => {
                out.push(c('Z', &[]));
                x = start_x;
                y = start_y;
            }
            'A' => {
                let (wr, hr) = (value(0, cmd), value(1, cmd));
                let (start, swing) = (value(2, cmd), value(3, cmd));
                if swing == 0.0 || (wr == 0.0 && hr == 0.0) {
                    continue;
                }
                for seg in arc_segments(x, y, wr, hr, start, swing) {
                    x = seg.x;
                    y = seg.y;
                    out.push(arc(
                        wr * sx,
                        hr * sy,
                        f64::from(seg.large),
                        f64::from(seg.sweep),
                        x * sx + ox,
                        y * sy + oy,
                    ));
                }
            }
            'Q' | 'C' => {
                let mut groups = Vec::new();
                let mut i = 0;
                while i + 1 < cmd.operands.len() {
                    let (px, py) = (value(i, cmd), value(i + 1, cmd));
                    groups.push(vec![px * sx + ox, py * sy + oy]);
                    x = px;
                    y = py;
                    i += 2;
                }
                out.push(Command {
                    letter: cmd.letter,
                    groups,
                });
            }
            _ => {}
        }
    }
    out
}

fn custom(paths: &[CustomPath], w: f64, h: f64, scale_paths: bool) -> Element {
    if let [only] = paths {
        return custom_path(only, w, h, scale_paths);
    }
    Element::open(Tag::G).holding(paths.iter().map(|p| custom_path(p, w, h, scale_paths)).collect())
}

fn custom_path(p: &CustomPath, w: f64, h: f64, scale_paths: bool) -> Element {
    let sx = if p.width > 0.0 { w / p.width } else { 1.0 };
    let sy = if p.height > 0.0 { h / p.height } else { 1.0 };
    if scale_paths {
        return path(scaled_path_data(&p.commands, sx, sy));
    }
    Element::empty(Tag::Path).word("d", &p.commands).with(
        "transform",
        Value::Transform(vec![Op::ScaleFactor(sx, sy, Sep::CommaSpace)]),
    )
}

/// The tokens of SVG path data: command letters and numbers.
pub(crate) fn tokens(data: &str) -> Vec<&str> {
    let bytes = data.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b.is_ascii_alphabetic() {
            out.push(data.get(i..=i).unwrap_or(""));
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i;
        if matches!(b, b'+' | b'-') {
            j += 1;
        }
        let digits = |k: &mut usize| {
            let from = *k;
            while bytes.get(*k).is_some_and(u8::is_ascii_digit) {
                *k += 1;
            }
            *k > from
        };
        let mut k = j;
        let whole = digits(&mut k);
        let mut ok = whole;
        if whole {
            if bytes.get(k) == Some(&b'.') {
                k += 1;
                digits(&mut k);
            }
        } else if bytes.get(k) == Some(&b'.') {
            k += 1;
            ok = digits(&mut k);
        }
        if !ok {
            i = start + 1;
            continue;
        }
        if matches!(bytes.get(k), Some(b'e' | b'E')) {
            let mut e = k + 1;
            if matches!(bytes.get(e), Some(b'+' | b'-')) {
                e += 1;
            }
            if digits(&mut e) {
                k = e;
            }
        }
        out.push(data.get(start..k).unwrap_or(""));
        i = k;
    }
    out
}

fn operand_roles(letter: char) -> &'static str {
    match letter.to_ascii_uppercase() {
        'M' | 'L' | 'T' => "xy",
        'H' => "x",
        'V' => "y",
        'C' => "xyxyxy",
        'S' | 'Q' => "xyxy",
        'A' => "xy---xy",
        _ => "",
    }
}

/// SVG path data with every coordinate multiplied by the scale, written as a coordinate
/// is (`scaled_path_data`). Numbers before any command letter are left out.
pub fn scaled_path_data(data: &str, sx: f64, sy: f64) -> Vec<Command> {
    let mut out: Vec<Command> = Vec::new();
    let mut index = 0usize;
    for token in tokens(data) {
        let mut chars = token.chars();
        if let (Some(letter), None) = (chars.next(), chars.next()) {
            if letter.is_ascii_alphabetic() {
                out.push(Command::new(letter, Vec::new()));
                index = 0;
                continue;
            }
        }
        let Some(current) = out.last_mut() else { continue };
        let roles = operand_roles(current.letter);
        let mut value: f64 = token.parse().unwrap_or(0.0);
        if let Some(role) = roles.as_bytes().get(index % roles.len().max(1)) {
            if *role == b'x' {
                value *= sx;
            } else if *role == b'y' {
                value *= sy;
            }
            index += 1;
        }
        match current.groups.first_mut() {
            Some(group) => group.push(value),
            None => current.groups.push(vec![value]),
        }
    }
    out
}

/// What a shape lays its text out in, as `text_rect` takes it.
#[derive(Debug, Clone, Copy)]
pub enum TextGeometry<'a> {
    /// A preset by name, with the shape's adjustments.
    Preset(&'a str, &'a [(String, f64)]),
    /// A preset by name, with the shape's adjustments as `a:avLst` writes them (formulas,
    /// `val 25000`).
    PresetWritten(&'a str, &'a [(String, String)]),
    /// A custom geometry's own rectangle.
    Custom(&'a TextRect),
}

/// The rectangle a geometry lays its text out in, `(left, top, right, bottom)` over the box
/// `width` by `height` at the origin; the whole box where nothing says otherwise.
pub fn text_rect(geometry: Option<TextGeometry<'_>>, width: f64, height: f64) -> (f64, f64, f64, f64) {
    let whole = (0.0, 0.0, width, height);
    let preset_rect = |name: &str, stated: &dyn Fn(&str) -> Option<String>| {
        let spec = preset(alias(name))?;
        let adjustments: Vec<(String, String)> = spec
            .adjustments
            .iter()
            .map(|(key, default)| (key.clone(), stated(key).unwrap_or_else(|| format!("val {default}"))))
            .collect();
        Some((
            &spec.text_rect,
            evaluate_guides(&[&adjustments, &spec.guides], width, height, true),
        ))
    };
    let (names, variables) = match geometry {
        Some(TextGeometry::Preset(name, adjust)) => {
            let stated = |key: &str| adjust.iter().find(|(k, _)| k == key).map(|(_, v)| format!("val {v}"));
            let Some(found) = preset_rect(name, &stated) else {
                return whole;
            };
            found
        }
        Some(TextGeometry::PresetWritten(name, adjust)) => {
            let stated = |key: &str| adjust.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
            let Some(found) = preset_rect(name, &stated) else {
                return whole;
            };
            found
        }
        Some(TextGeometry::Custom(rect)) => (
            &rect.rect,
            evaluate_guides(&[&rect.adjustments, &rect.guides], width, height, true),
        ),
        None => return whole,
    };
    let [l, t, r, b] = names;
    (
        resolve_value(l, &variables),
        resolve_value(t, &variables),
        resolve_value(r, &variables),
        resolve_value(b, &variables),
    )
}

/// [`text_rect`] drawn in by the rules' share of the outline's width.
pub fn text_area(
    geometry: Option<TextGeometry<'_>>,
    width: f64,
    height: f64,
    outline_width: f64,
    rules: &DrawingRules,
) -> (f64, f64, f64, f64) {
    let (l, t, r, b) = text_rect(geometry, width, height);
    let inset = rules.text_outline_inset * outline_width;
    if inset == 0.0 {
        return (l, t, r, b);
    }
    (l + inset, t + inset, r - inset, b - inset)
}
