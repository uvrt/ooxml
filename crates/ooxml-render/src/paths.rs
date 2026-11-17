//! Owns: a geometry as path data over a box, for a painter that draws each `a:path` itself
//! (ooxml-common's `geometry.GeometryPath` and the functions that make it: `spec_path_data`,
//! `preset_path_data`, `geometry_path_data`) -- each path with its fill mode and stroke flag,
//! every preset from the specification's table -- and an open path's ends and its cutting
//! back under arrowheads (`path_ends`, `trim_path`).
//!
//! A path's ends are read from the path as written: its numbers as the writer writes them
//! ([`ooxml_scene::written`]), as docx2svg reads them back from the path data it was given.
//! A cut path is written as ooxml-common writes it: each command's numbers in one run, an
//! arc's flags as integers.
//!
//! Must not: draw the paths (the caller paints them), or evaluate a guide itself
//! ([`ooxml_drawingml::guides`] does).

use crate::geometry::{alias, spec_commands, tokens};
use ooxml_drawingml::drawable::Geometry;
use ooxml_drawingml::guides::{evaluate_guides, Variables};
use ooxml_drawingml::presets::preset;
use ooxml_drawingml::source::SpecPath;
use ooxml_scene::markup::Command;
use ooxml_scene::written;

/// One `a:path` of a geometry, as path data in the box's coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct GeometryPath {
    /// The path data.
    pub d: Vec<Command>,
    /// `@fill`: `norm`, `none`, or a shading mode (`lighten`, `lightenLess`, `darken`,
    /// `darkenLess`).
    pub fill: String,
    /// `@stroke`.
    pub stroke: bool,
}

/// A geometry in the preset table's form, as path data over the box `x, y, width,
/// height`: `adjust` overrides adjustments by name (a formula, `val 25000`, as `a:avLst`
/// states it); a path with its own `@w`/`@h` is scaled onto the box; a path that draws
/// nothing is left out.
pub fn spec_path_data(
    adjustments: &[(String, String)],
    guides: &[(String, String)],
    paths: &[SpecPath],
    adjust: &[(String, String)],
    (x, y, width, height): (f64, f64, f64, f64),
) -> Vec<GeometryPath> {
    let evaluated: Vec<(String, String)> = adjustments
        .iter()
        .map(|(name, default)| {
            let stated = adjust.iter().find(|(key, _)| key == name).map(|(_, v)| v);
            (name.clone(), stated.unwrap_or(default).clone())
        })
        .collect();
    let variables = evaluate_guides(&[&evaluated, guides], width, height, true);
    paths_over(paths, &variables, (x, y, width, height))
}

fn paths_over(
    paths: &[SpecPath],
    variables: &Variables,
    (x, y, width, height): (f64, f64, f64, f64),
) -> Vec<GeometryPath> {
    paths
        .iter()
        .filter_map(|path| {
            let scale = path.space.map_or((1.0, 1.0), |(sw, sh)| {
                (
                    if sw != 0.0 { width / sw } else { 1.0 },
                    if sh != 0.0 { height / sh } else { 1.0 },
                )
            });
            let d = spec_commands(&path.commands, variables, scale, (x, y));
            (!d.is_empty()).then(|| GeometryPath {
                d,
                fill: path.fill.clone(),
                stroke: path.stroke,
            })
        })
        .collect()
}

/// A preset's paths from the specification's table over the box, or `None` for a name
/// `ST_ShapeType` does not have; `adjust` as [`spec_path_data`] takes it.
pub fn preset_path_data(
    name: &str,
    adjust: &[(String, String)],
    frame: (f64, f64, f64, f64),
) -> Option<Vec<GeometryPath>> {
    let spec = preset(alias(name))?;
    let defaults: Vec<(String, String)> = spec
        .adjustments
        .iter()
        .map(|(key, value)| (key.clone(), format!("val {value}")))
        .collect();
    Some(spec_path_data(&defaults, &spec.guides, &spec.paths, adjust, frame))
}

/// A drawable's geometry as path data over the box: a preset from the table (`None` for an
/// unknown name), a custom geometry's paths scaled from their own `@w`/`@h` onto the box,
/// each `norm` and stroked (the model keeps no `@fill` or `@stroke`).
pub fn geometry_path_data(geometry: &Geometry, frame: (f64, f64, f64, f64)) -> Option<Vec<GeometryPath>> {
    match geometry {
        Geometry::Preset { name, adjust } => {
            let adjust: Vec<(String, String)> = adjust.iter().map(|(k, v)| (k.clone(), format!("val {v}"))).collect();
            preset_path_data(name, &adjust, frame)
        }
        Geometry::Custom(paths) => {
            let (x, y, width, height) = frame;
            Some(
                paths
                    .iter()
                    .filter_map(|path| {
                        let sx = if path.width > 0.0 { width / path.width } else { 1.0 };
                        let sy = if path.height > 0.0 { height / path.height } else { 1.0 };
                        let d = scale_commands(&path.commands, (sx, sy), (x, y));
                        (!d.is_empty()).then(|| GeometryPath {
                            d,
                            fill: "norm".to_owned(),
                            stroke: true,
                        })
                    })
                    .collect(),
            )
        }
    }
}

/// How many operands each command letter of this path form takes, and which are x and y.
fn roles(letter: char) -> Option<&'static str> {
    Some(match letter {
        'M' | 'L' => "xy",
        'Q' => "xyxy",
        'C' => "xyxyxy",
        'A' => "rr---xy",
        'Z' => "",
        _ => return None,
    })
}

/// Absolute path data in the form `M L Q C A Z` read into commands: a token before a
/// command, or a command cut short, ends or skips as ooxml-common's `_commands` does.
fn read_commands(data: &str) -> Vec<(char, Vec<&str>)> {
    let tokens: Vec<&str> = tokens(data)
        .into_iter()
        .filter(|t| !t.starts_with(|c: char| c.is_ascii_alphabetic()) || roles_of(t).is_some())
        .collect();
    let mut out = Vec::new();
    let mut index = 0;
    while let Some(token) = tokens.get(index) {
        index += 1;
        let Some((letter, roles)) = roles_of(token) else {
            continue;
        };
        let operands = tokens.get(index..index + roles.len()).unwrap_or_default();
        index += roles.len();
        if operands.len() < roles.len() {
            break;
        }
        out.push((letter, operands.to_vec()));
    }
    out
}

fn roles_of(token: &str) -> Option<(char, &'static str)> {
    let mut chars = token.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), None) => roles(letter).map(|r| (letter, r)),
        _ => None,
    }
}

/// `_scale_path_data`: coordinates scaled about the origin and moved by `offset`, radii
/// scaled, an arc's rotation and flags kept.
fn scale_commands(data: &str, (sx, sy): (f64, f64), (x, y): (f64, f64)) -> Vec<Command> {
    read_commands(data)
        .into_iter()
        .map(|(letter, operands)| {
            let roles = roles(letter).unwrap_or("");
            let mut values = Vec::with_capacity(operands.len());
            for (role, token) in roles.bytes().zip(operands) {
                let value: f64 = token.parse().unwrap_or(0.0);
                values.push(match role {
                    b'x' => value * sx + x,
                    b'y' => value * sy + y,
                    b'r' if values.is_empty() => value * sx,
                    b'r' => value * sy,
                    _ => value,
                });
            }
            Command::new(letter, values)
        })
        .collect()
}

/// An open path's end: the point, and the unit direction pointing out of the path there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathEnd {
    /// The point's x.
    pub x: f64,
    /// Its y.
    pub y: f64,
    /// The direction's x.
    pub dx: f64,
    /// Its y.
    pub dy: f64,
}

/// The path's commands with its numbers as written.
fn as_written(d: &[Command]) -> Vec<(char, Vec<f64>)> {
    d.iter()
        .map(|c| (c.letter, c.groups.iter().flatten().map(|v| written(*v)).collect()))
        .collect()
}

fn unit(dx: f64, dy: f64) -> Option<(f64, f64)> {
    let length = dx.hypot(dy);
    (length > 1e-9).then(|| (dx / length, dy / length))
}

/// The last two numbers of a command (its end point), or of the command before.
fn tail(values: &[f64], from_end: usize) -> Option<(f64, f64)> {
    let n = values.len();
    Some((
        *values.get(n.checked_sub(from_end + 2)?)?,
        *values.get(n.checked_sub(from_end + 1)?)?,
    ))
}

/// Where the first segment heads: its first point for a line or curve, its end for an arc.
fn toward(command: &(char, Vec<f64>)) -> Option<(f64, f64)> {
    let (letter, values) = command;
    if matches!(letter, 'L' | 'Q' | 'C') {
        Some((*values.first()?, *values.get(1)?))
    } else {
        tail(values, 0)
    }
}

/// Where the last segment comes from: a curve's last control point, else the point before.
fn previous(before: &(char, Vec<f64>), last: &(char, Vec<f64>)) -> Option<(f64, f64)> {
    if matches!(last.0, 'Q' | 'C') {
        tail(&last.1, 2)
    } else {
        tail(&before.1, 0)
    }
}

/// An open path's first and last points, each with the direction out of the path there, or
/// `None` for a closed or degenerate path. A curve's end points along its last control
/// leg; an arc's along its chord.
pub fn path_ends(d: &[Command]) -> Option<[PathEnd; 2]> {
    let commands = as_written(d);
    let first = commands.first()?;
    if first.0 != 'M' || commands.iter().any(|c| c.0 == 'Z') {
        return None;
    }
    if commands.len() < 2 || commands.iter().skip(1).any(|c| c.0 == 'M') {
        return None;
    }
    let start = (*first.1.first()?, *first.1.get(1)?);
    let heading = toward(commands.get(1)?)?;
    let (sdx, sdy) = unit(start.0 - heading.0, start.1 - heading.1)?;
    let last = commands.last()?;
    let end = tail(&last.1, 0)?;
    let from = previous(commands.get(commands.len() - 2)?, last)?;
    let (edx, edy) = unit(end.0 - from.0, end.1 - from.1)?;
    Some([
        PathEnd {
            x: start.0,
            y: start.1,
            dx: sdx,
            dy: sdy,
        },
        PathEnd {
            x: end.0,
            y: end.1,
            dx: edx,
            dy: edy,
        },
    ])
}

/// How far `point` moves toward `other` to cut `by` off, never past `other`.
fn cut(point: (f64, f64), other: (f64, f64), by: f64) -> Option<(f64, f64)> {
    let length = (other.0 - point.0).hypot(other.1 - point.1);
    (length > 1e-9).then(|| {
        let t = by.min(length * 0.999) / length;
        (point.0 + (other.0 - point.0) * t, point.1 + (other.1 - point.1) * t)
    })
}

fn set_tail(values: &mut [f64], (x, y): (f64, f64)) {
    if let [.., a, b] = values {
        *a = x;
        *b = y;
    }
}

/// The open path cut back by `start` at its first point and `end` at its last, along its
/// end segments (a straight segment exactly; a curve's end point moved along its tangent,
/// never past the segment's other end). Unchanged where nothing is cut or there is no
/// segment.
pub fn trim_path(d: &[Command], start: f64, end: f64) -> Vec<Command> {
    if start == 0.0 && end == 0.0 {
        return d.to_vec();
    }
    let mut commands = as_written(d);
    if commands.len() < 2 {
        return d.to_vec();
    }
    if start != 0.0 {
        let heading = commands.get(1).and_then(toward);
        if let (Some(first), Some(heading)) = (commands.first_mut(), heading) {
            let point = (first.1.first().copied(), first.1.get(1).copied());
            if let (Some(px), Some(py)) = point {
                if let Some(moved) = cut((px, py), heading, start) {
                    if let [x, y, ..] = first.1.as_mut_slice() {
                        *x = moved.0;
                        *y = moved.1;
                    }
                }
            }
        }
    }
    if end != 0.0 {
        let n = commands.len();
        let from = commands
            .get(n - 2)
            .zip(commands.last())
            .and_then(|(b, l)| previous(b, l));
        if let (Some(last), Some(from)) = (commands.last_mut(), from) {
            if let Some(point) = tail(&last.1, 0) {
                if let Some(moved) = cut(point, from, end) {
                    set_tail(&mut last.1, moved);
                }
            }
        }
    }
    commands
        .into_iter()
        .map(|(letter, values)| Command::new(letter, values))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> Vec<Command> {
        vec![Command::new('M', vec![0.0, 0.0]), Command::new('L', vec![10.0, 0.0])]
    }

    #[test]
    fn an_open_line_has_ends_pointing_out() {
        let [a, b] = path_ends(&line()).unwrap();
        assert_eq!((a.x, a.y, a.dx, a.dy), (0.0, 0.0, -1.0, 0.0));
        assert_eq!((b.x, b.y, b.dx, b.dy), (10.0, 0.0, 1.0, 0.0));
        let closed = [line(), vec![Command::new('Z', vec![])]].concat();
        assert!(path_ends(&closed).is_none());
    }

    #[test]
    fn a_trimmed_line_stops_short_at_both_ends() {
        let cut = trim_path(&line(), 2.0, 3.0);
        assert_eq!(
            cut,
            vec![Command::new('M', vec![2.0, 0.0]), Command::new('L', vec![7.0, 0.0])]
        );
        assert_eq!(trim_path(&line(), 0.0, 0.0), line());
    }

    #[test]
    fn a_preset_is_drawn_from_the_table_over_its_box() {
        let paths = preset_path_data("rect", &[], (10.0, 20.0, 30.0, 40.0)).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].d.first(), Some(&Command::new('M', vec![10.0, 20.0])));
        assert!(preset_path_data("noSuchPreset", &[], (0.0, 0.0, 1.0, 1.0)).is_none());
    }
}
