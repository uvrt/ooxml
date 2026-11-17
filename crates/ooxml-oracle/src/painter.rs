//! Owns: the painter pieces' differential's Rust side -- one call docx2svg made to
//! ooxml-common's DrawingML pieces, as `python/painter_record.py` wrote it, made again
//! through `ooxml-render` ([`replay`]) and written in the Python's form for comparison:
//! attribute text, the resources declared, path data, numbers.
//!
//! Must not: change what is drawn: every argument is the call's own.

use crate::scene_data::{self, pairs, spec, text, tuple};
use ooxml_drawingml::drawable::TextRect;
use ooxml_render::arrowhead::render_arrowheads;
use ooxml_render::effect::render_effects;
use ooxml_render::fill::{fill_attrs, outline_attrs, Box4, ShapeFrame};
use ooxml_render::geometry::{text_rect, TextGeometry};
use ooxml_render::paths::{path_ends, preset_path_data, spec_path_data, trim_path, GeometryPath, PathEnd};
use ooxml_render::{table_measurer, Painter, POWERPOINT, WORD};
use ooxml_scene::markup::{path_text, Attr, Command, Element, Op, Sep, Tag};
use serde_json::{json, Value as J};

/// What a call answered, in the recorder's form.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    /// The result, as `painter_record.py`'s `answer` writes it.
    pub result: J,
    /// The resources the call declared, as markup.
    pub defs: Vec<String>,
}

fn number(v: Option<&J>, default: f64) -> f64 {
    v.and_then(J::as_f64).unwrap_or(default)
}

/// A transform list as docx2svg writes one (`translate(a b) scale(x y) rotate(r) ...`).
fn ops(text: &str) -> Vec<Op> {
    text.split(')')
        .filter_map(|part| {
            let (name, args) = part.trim().split_once('(')?;
            let n: Vec<f64> = args.split_whitespace().filter_map(|a| a.parse().ok()).collect();
            Some(match (name, n.as_slice()) {
                ("translate", [x, y]) => Op::Translate(*x, *y, Sep::Space),
                ("scale", [x, y]) => Op::Scale(*x, *y, Sep::Space),
                ("rotate", [a]) => Op::Rotate(*a),
                _ => return None,
            })
        })
        .collect()
}

fn frame(v: Option<&J>) -> Option<ShapeFrame> {
    let v = v.filter(|v| !v.is_null())?;
    Some(ShapeFrame {
        rotation: number(v.get("rotation"), 0.0),
        flip_h: v.get("flip_h").and_then(J::as_bool).unwrap_or(false),
        flip_v: v.get("flip_v").and_then(J::as_bool).unwrap_or(false),
        page_transform: v.get("page_transform").and_then(J::as_str).map(ops),
        rectangular: v.get("rectangular").and_then(J::as_bool).unwrap_or(true),
    })
}

fn four(v: Option<&J>) -> Option<Box4> {
    match tuple(v.filter(|v| !v.is_null())?) {
        [a, b, c, d] => Some((a.as_f64()?, b.as_f64()?, c.as_f64()?, d.as_f64()?)),
        _ => None,
    }
}

/// Path data as the recorder wrote it, read back into commands (groups split at commas).
pub fn commands(d: &str) -> Vec<Command> {
    let mut out = Vec::new();
    let mut rest = d.trim();
    while let Some(letter) = rest.chars().next() {
        let body_end = rest
            .char_indices()
            .skip(1)
            .find(|(_, c)| c.is_ascii_alphabetic() && *c != 'e' && *c != 'E')
            .map_or(rest.len(), |(i, _)| i);
        let body = rest.get(letter.len_utf8()..body_end).unwrap_or("");
        let groups: Vec<Vec<f64>> = body
            .split(',')
            .map(|g| {
                g.split_whitespace()
                    .filter_map(|n| n.parse().ok())
                    .collect::<Vec<f64>>()
            })
            .filter(|g| !g.is_empty())
            .collect();
        out.push(Command { letter, groups });
        rest = rest.get(body_end..).unwrap_or("").trim_start();
    }
    out
}

fn attrs_text(attrs: &[Attr]) -> String {
    let mut element = Element::empty(Tag::G);
    element.attrs = attrs.to_vec();
    let svg = element.to_svg();
    svg.strip_prefix("<g ")
        .and_then(|s| s.strip_suffix("/>"))
        .unwrap_or("")
        .to_owned()
}

fn paths_answer(paths: Option<Vec<GeometryPath>>) -> J {
    paths.map_or(J::Null, |paths| {
        J::Array(
            paths
                .into_iter()
                .map(|p| json!([path_text(&p.d), p.fill, p.stroke]))
                .collect(),
        )
    })
}

fn ends(v: &J) -> Option<[PathEnd; 2]> {
    let read = |e: &J| match tuple(e) {
        [x, y, dx, dy] => Some(PathEnd {
            x: x.as_f64()?,
            y: y.as_f64()?,
            dx: dx.as_f64()?,
            dy: dy.as_f64()?,
        }),
        _ => None,
    };
    match tuple(v) {
        [a, b] => Some([read(a)?, read(b)?]),
        _ => None,
    }
}

/// The call `record` made again through `ooxml-render`; `None` for a function not replayed.
pub fn replay(record: &J) -> Option<Answer> {
    let name = record.get("fn")?.as_str()?;
    let args = record.get("args").and_then(J::as_array).map_or(&[][..], Vec::as_slice);
    let kwargs = record.get("kwargs").unwrap_or(&J::Null);
    let arg = |i: usize, key: &str| args.get(i).filter(|v| !v.is_null()).or_else(|| kwargs.get(key));
    let rules = if kwargs.get("rules").and_then(J::as_str) == Some("word") {
        WORD
    } else {
        POWERPOINT
    };
    let measurer = table_measurer(&rules);
    let mut painter = Painter::new(rules, &measurer);
    painter.dpi = number(kwargs.get("dpi"), 96.0);
    painter.resume_ids(
        record
            .get("ids_before")
            .and_then(J::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0),
    );
    let result = match name {
        "render_fill_attrs" => {
            let fill = arg(0, "fill").and_then(scene_data::fill);
            let frame = frame(kwargs.get("frame"));
            J::String(attrs_text(&fill_attrs(
                fill.as_ref(),
                &mut painter,
                four(arg(2, "box")),
                frame.as_ref(),
            )))
        }
        "render_outline_attrs" => {
            let outline = arg(0, "outline").map(scene_data::outline);
            let frame = frame(kwargs.get("frame"));
            let attrs = outline_attrs(outline.as_ref(), &mut painter, four(kwargs.get("box")), frame.as_ref());
            J::String(attrs_text(&attrs))
        }
        "render_effects" => {
            let effects = arg(0, "effects").map(scene_data::effects);
            let attr = render_effects(effects.as_ref(), &mut painter, four(kwargs.get("bounds")));
            J::String(attr.map(|a| attrs_text(&[a])).unwrap_or_default())
        }
        "render_arrowheads" => {
            let outline = arg(0, "outline").map(scene_data::outline);
            let heads = render_arrowheads(outline.as_ref(), &ends(arg(1, "ends")?)?, painter.dpi);
            let elements: Vec<String> = heads.elements.iter().map(Element::to_svg).collect();
            json!([elements, heads.start, heads.end])
        }
        "to_unicode" => {
            let face = arg(0, "face")?.as_str()?;
            let ch = arg(1, "char")?.as_str()?.chars().next()?;
            ooxml_fonts::symbols::to_unicode(face, ch).map_or(J::Null, J::String)
        }
        _ => return replay_geometry(name, &arg),
    };
    let defs = painter.defs.iter().map(Element::to_svg).collect();
    Some(Answer { result, defs })
}

fn replay_geometry<'a>(name: &str, arg: &dyn Fn(usize, &str) -> Option<&'a J>) -> Option<Answer> {
    let f = |i: usize, key: &str, default: f64| number(arg(i, key), default);
    let frame = (f(9, "x", 0.0), f(9, "y", 0.0), f(1, "width", 0.0), f(2, "height", 0.0));
    let result = match name {
        "preset_path_data" => {
            let adjust = arg(3, "adjust").map(pairs).unwrap_or_default();
            paths_answer(preset_path_data(arg(0, "preset")?.as_str()?, &adjust, frame))
        }
        "spec_path_data" => {
            let (adjustments, guides, paths) = spec(arg(0, "spec")?);
            let adjust = arg(3, "adjust").map(pairs).unwrap_or_default();
            paths_answer(Some(spec_path_data(&adjustments, &guides, &paths, &adjust, frame)))
        }
        "path_ends" => match path_ends(&commands(arg(0, "data")?.as_str()?)) {
            Some([a, b]) => json!([[a.x, a.y, a.dx, a.dy], [b.x, b.y, b.dx, b.dy]]),
            None => J::Null,
        },
        "trim_path" => {
            let d = commands(arg(0, "data")?.as_str()?);
            let cut = trim_path(&d, f(1, "start", 0.0), f(2, "end", 0.0));
            J::String(path_text(&cut))
        }
        "text_rect" => {
            let (x, y) = (frame.0, frame.1);
            let geometry = arg(0, "geometry").map(tuple).unwrap_or_default();
            let rect = arg(9, "rect").map(tuple).unwrap_or_default();
            let (l, t, r, b) = text_rect_of(geometry, rect, frame.2, frame.3);
            json!([x + l, y + t, x + r, y + b])
        }
        _ => return None,
    };
    Some(Answer {
        result,
        defs: Vec::new(),
    })
}

fn text_rect_of(geometry: &[J], rect: &[J], width: f64, height: f64) -> (f64, f64, f64, f64) {
    let kind = geometry.first().and_then(J::as_str);
    match (kind, geometry.get(1), geometry.get(2)) {
        (Some("preset"), Some(name), adjust) => {
            let adjust = adjust.map(pairs).unwrap_or_default();
            let name = name.as_str().unwrap_or("rect");
            text_rect(Some(TextGeometry::PresetWritten(name, &adjust)), width, height)
        }
        (Some("custom"), Some(found), _) if rect.len() == 4 => {
            let (adjustments, guides, _) = spec(found);
            let names: Vec<String> = rect.iter().map(text).collect();
            let rect = TextRect {
                adjustments,
                guides,
                rect: [0, 1, 2, 3].map(|i| names.get(i).cloned().unwrap_or_default()),
            };
            text_rect(Some(TextGeometry::Custom(&rect)), width, height)
        }
        _ => (0.0, 0.0, width, height),
    }
}
