//! Owns: reading transforms (`a:xfrm`, a group's child space, `dsp:txXfrm`) and geometry --
//! `a:prstGeom` with its adjustments, `a:custGeom` evaluated into path data at its own size,
//! the same unevaluated (a geometry spec), and a custom geometry's text rectangle -- as
//! ooxml-common's `drawingml/read.py` reads them.
//!
//! Must not: evaluate a preset (the preset table's consumers do) or draw a path.

use super::{is_true, or, or_str};
use crate::guides::{arc_segments, evaluate_guides, py_words, resolve_value, Variables};
use crate::pyfmt::coordinate;
use crate::source::{CustomPath, Geometry, GeometrySpec, SpecCommand, SpecPath, Xfrm};
use ooxml_schema::a;
use ooxml_xml::view::py_float;

/// `a:xfrm` (and kin) as a transform; `None` unless its offset and extent are both stated.
pub fn xfrm(xfrm: Option<a::Xfrm<'_>>) -> Option<Xfrm> {
    let xfrm = xfrm?;
    let offset = xfrm.offset();
    let extent = xfrm.extent();
    Some(Xfrm {
        x: offset.and_then(a::Point::x)?,
        y: offset.and_then(a::Point::y)?,
        width: extent.and_then(a::Size::cx)?,
        height: extent.and_then(a::Size::cy)?,
        rotation: or(xfrm.rotation(), 0.0),
        flip_h: is_true(xfrm.flip_h()),
        flip_v: is_true(xfrm.flip_v()),
    })
}

/// A group's `a:xfrm`: its placement, and its children's space where both `chOff` and
/// `chExt` are stated.
pub fn group_xfrms(xfrm_view: Option<a::Xfrm<'_>>) -> (Option<Xfrm>, Option<Xfrm>) {
    let Some(view) = xfrm_view else {
        return (None, None);
    };
    let outer = xfrm(Some(view));
    let (Some(offset), Some(extent)) = (view.child_offset(), view.child_extent()) else {
        return (outer, None);
    };
    let inner = Xfrm {
        x: or(offset.x(), 0.0),
        y: or(offset.y(), 0.0),
        width: or(extent.cx(), 0.0),
        height: or(extent.cy(), 0.0),
        ..Xfrm::default()
    };
    (outer, Some(inner))
}

/// A guide list as `(name, formula)`, each with both stated and not empty.
fn guide_list(list: Option<a::GuideList<'_>>) -> Vec<(String, String)> {
    list.into_iter()
        .flat_map(a::GuideList::guides)
        .filter_map(|g| {
            let name = g.name().filter(|n| !n.is_empty())?;
            let formula = g.formula().filter(|f| !f.is_empty())?;
            Some((name.to_owned(), formula.to_owned()))
        })
        .collect()
}

/// An adjustment's value: only `val N` is read.
fn adjust_value(formula: &str) -> Option<f64> {
    let mut tokens = py_words(formula);
    match (tokens.next(), tokens.next(), tokens.next()) {
        (Some("val"), Some(value), None) => py_float(value),
        _ => None,
    }
}

/// `name -> value` kept in first-stated order, a later duplicate replacing the value.
fn insert<T>(pairs: &mut Vec<(String, T)>, name: String, value: T) {
    match pairs.iter_mut().find(|(k, _)| *k == name) {
        Some(slot) => slot.1 = value,
        None => pairs.push((name, value)),
    }
}

/// A shape's geometry: its preset, or its custom geometry with at least one drawn path.
pub fn geometry(properties: Option<a::ShapeProperties<'_>>) -> Option<Geometry> {
    let properties = properties?;
    if let Some(preset) = properties.preset() {
        let mut adjust = Vec::new();
        for guide in preset.adjustments().into_iter().flat_map(a::GuideList::guides) {
            if let (Some(name), Some(value)) = (guide.name(), guide.formula().and_then(adjust_value)) {
                insert(&mut adjust, name.to_owned(), value);
            }
        }
        return Some(Geometry::Preset {
            name: or_str(preset.preset(), "rect"),
            adjust,
        });
    }
    let paths = custom_paths(properties.custom()?);
    (!paths.is_empty()).then_some(Geometry::Custom(paths))
}

/// `a:custGeom`'s paths evaluated at their own sizes into path data.
fn custom_paths(custom: a::CustomGeometry<'_>) -> Vec<CustomPath> {
    let adjustments = guide_list(custom.adjustments());
    let guides = guide_list(custom.guides());
    let mut out = Vec::new();
    for path in custom.paths().into_iter().flat_map(a::PathList::paths) {
        let (width, height) = (or(path.w(), 0.0), or(path.h(), 0.0));
        if width == 0.0 && height == 0.0 {
            continue;
        }
        let variables = evaluate_guides(&[&adjustments, &guides], width, height, false);
        let commands = path_commands(path, &variables);
        if !commands.is_empty() {
            out.push(CustomPath {
                width,
                height,
                commands,
            });
        }
    }
    out
}

fn points(command: a::PathCommand<'_>, variables: &Variables) -> Vec<(f64, f64)> {
    command
        .points()
        .map(|p| {
            (
                resolve_value(&or_str(p.x(), "0"), variables),
                resolve_value(&or_str(p.y(), "0"), variables),
            )
        })
        .collect()
}

fn pair((x, y): (f64, f64)) -> String {
    format!("{} {}", coordinate(x), coordinate(y))
}

/// The SVG path data of one `a:path`.
fn path_commands(path: a::Path<'_>, variables: &Variables) -> String {
    let mut parts: Vec<String> = Vec::new();
    let (mut current, mut start) = ((0.0, 0.0), (0.0, 0.0));
    for command in path.commands() {
        let found = points(command, variables);
        match command.element().name().local() {
            "moveTo" | "lnTo" => {
                if let Some(first) = found.first().copied() {
                    let moving = command.element().name().local() == "moveTo";
                    parts.push(format!("{} {}", if moving { "M" } else { "L" }, pair(first)));
                    current = first;
                    if moving {
                        start = first;
                    }
                }
            }
            name @ ("cubicBezTo" | "quadBezTo") => {
                let (letter, needed) = if name == "cubicBezTo" { ("C", 3) } else { ("Q", 2) };
                if let (true, Some(last)) = (found.len() >= needed, found.last().copied()) {
                    let joined: Vec<String> = found.iter().copied().map(pair).collect();
                    parts.push(format!("{letter} {}", joined.join(", ")));
                    current = last;
                }
            }
            "arcTo" => {
                if let Some((arc, end)) = arc_to(command, current, variables) {
                    parts.push(arc);
                    current = end;
                }
            }
            "close" => {
                parts.push("Z".into());
                current = start;
            }
            _ => {}
        }
    }
    parts.join(" ")
}

fn arc_to(command: a::PathCommand<'_>, (x, y): (f64, f64), variables: &Variables) -> Option<(String, (f64, f64))> {
    let value = |v: Option<&str>| resolve_value(&or_str(v, "0"), variables);
    let (wr, hr) = (value(command.width_radius()), value(command.height_radius()));
    let (start, swing) = (value(command.start_angle()), value(command.swing_angle()));
    if (wr == 0.0 && hr == 0.0) || swing == 0.0 {
        return None;
    }
    let segments = arc_segments(x, y, wr, hr, start, swing);
    let last = segments.last()?;
    let written: Vec<String> = segments
        .iter()
        .map(|s| {
            format!(
                "A {} {} 0 {} {} {} {}",
                coordinate(wr),
                coordinate(hr),
                s.large,
                s.sweep,
                coordinate(s.x),
                coordinate(s.y)
            )
        })
        .collect();
    Some((written.join(" "), (last.x, last.y)))
}

/// The geometry as written, unevaluated (`None` where the shape states neither kind).
pub fn geometry_spec(properties: Option<a::ShapeProperties<'_>>) -> Option<GeometrySpec> {
    let properties = properties?;
    if let Some(preset) = properties.preset() {
        let mut adjust = Vec::new();
        for g in preset.adjustments().into_iter().flat_map(a::GuideList::guides) {
            insert(&mut adjust, or_str(g.name(), ""), or_str(g.formula(), ""));
        }
        return Some(GeometrySpec::Preset {
            name: or_str(preset.preset(), "rect"),
            adjust,
        });
    }
    let custom = properties.custom()?;
    let paths = custom
        .paths()
        .into_iter()
        .flat_map(a::PathList::paths)
        .map(spec_path)
        .collect();
    Some(GeometrySpec::Custom {
        adjustments: guide_list(custom.adjustments()),
        guides: guide_list(custom.guides()),
        paths,
    })
}

fn spec_path(path: a::Path<'_>) -> SpecPath {
    let mut commands = Vec::new();
    for command in path.commands() {
        let operand = |v: Option<&str>| or_str(v, "0");
        let (letter, operands) = match command.element().name().local() {
            "moveTo" => ('M', None),
            "lnTo" => ('L', None),
            "quadBezTo" => ('Q', None),
            "cubicBezTo" => ('C', None),
            "arcTo" => (
                'A',
                Some(vec![
                    operand(command.width_radius()),
                    operand(command.height_radius()),
                    operand(command.start_angle()),
                    operand(command.swing_angle()),
                ]),
            ),
            "close" => ('Z', Some(Vec::new())),
            _ => continue,
        };
        let operands = operands.unwrap_or_else(|| {
            command
                .points()
                .flat_map(|p| [operand(p.x()), operand(p.y())])
                .collect()
        });
        commands.push(SpecCommand { letter, operands });
    }
    let space = (or(path.w(), 0.0), or(path.h(), 0.0));
    SpecPath {
        fill: or_str(path.fill(), "norm"),
        stroke: !matches!(path.stroke(), Some("0" | "false")),
        space: (space.0 != 0.0 || space.1 != 0.0).then_some(space),
        commands,
    }
}

/// A custom geometry's `a:rect`: `l`, `t`, `r`, `b` as written (their names where unstated).
pub fn text_rect(properties: Option<a::ShapeProperties<'_>>) -> Option<[String; 4]> {
    let rect = properties?.custom()?.rect()?;
    Some([
        or_str(rect.l(), "l"),
        or_str(rect.t(), "t"),
        or_str(rect.r(), "r"),
        or_str(rect.b(), "b"),
    ])
}
