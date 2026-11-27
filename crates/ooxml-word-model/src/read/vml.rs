//! Owns: a run's VML object (`w:pict`, `w:object`, an `mc:AlternateContent`'s VML where
//! its branch the readers read holds one)
//! as far as layout takes it -- docx2svg's `parse.vml`: an absolutely positioned shape is a
//! floating drawing ([`Anchor`]: its CSS `width`/`height`, `mso-position-*`, `margin-*`,
//! `z-index`, `w10:wrap`); anything else a character of its line (`drawing:cx:cy:0:0:0:0`);
//! what it shows is a picture, a preset shape (with a text box's blocks and insets), a text
//! path, or something else. A preset shape or a text path keeps the run's object it was
//! read from (`Graphic.source`) for its painter, which reads its paint ([`vml_shape`],
//! [`style_of`], [`length_emu`], [`preset_of`]).
//!
//! Must not: read how a shape is painted (fill, stroke, a text path's font and colour: its
//! painter's).

use super::fields::Fields;
use super::Reader;
use crate::model::{Anchor, AnchorPosition, BodyValue, Graphic, InlineObject};
use crate::value::{text, Text};
use crate::xmlread::{attr, child, elements, local, py_int};
use ooxml_xml::Element;
use std::collections::BTreeMap;
use std::sync::Arc;

const VML: &str = "urn:schemas-microsoft-com:vml";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const SHAPES: &[&str] = &[
    "shape",
    "rect",
    "roundrect",
    "oval",
    "line",
    "polyline",
    "arc",
    "curve",
    "image",
    "group",
];
const INSETS: [i64; 4] = [91440, 45720, 91440, 45720];

/// A VML object read: floating, or a character of its line with its mark and what it shows.
pub(crate) enum Vml {
    /// Absolutely positioned.
    Anchor(Box<Anchor>),
    /// In the line.
    Inline(String, InlineObject),
}

/// A VML `style`'s declarations by lower-cased property.
pub fn style_of(element: &Element) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for declaration in attr(element, "style").unwrap_or("").split(';') {
        let (name, value) = declaration.split_once(':').unwrap_or((declaration, ""));
        let (name, value) = (name.trim(), value.trim());
        if !name.is_empty() && !value.is_empty() {
            let value = value.trim_matches('"').replace("&quot;", "");
            out.insert(name.to_lowercase(), value);
        }
    }
    out
}

fn emu_per(unit: &str) -> Option<f64> {
    Some(match unit {
        "pt" => 12700.0,
        "in" => 914_400.0,
        "cm" => 360_000.0,
        "mm" => 36000.0,
        "px" | "" => 9525.0,
        "pc" => 152_400.0,
        "emu" => 1.0,
        _ => return None,
    })
}

/// A CSS length in EMU (`412.4pt`, `1in`, `0`; a bare number is px).
pub fn length_emu(value: Option<&str>) -> Option<i64> {
    let v = value?.to_lowercase();
    let v = v.trim();
    let split = v.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(v.len());
    let (number, unit) = (v.get(..split)?.trim(), v.get(split..)?.trim());
    let digits = number.strip_prefix('-').unwrap_or(number);
    let shape_ok = !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
        && digits.matches('.').count() <= 1
        && !digits.ends_with('.')
        && unit.chars().all(|c| c.is_ascii_lowercase());
    if !shape_ok {
        return None;
    }
    let n: f64 = number.parse().ok()?;
    let product = n * emu_per(unit)?;
    #[allow(clippy::cast_possible_truncation)]
    Some(product.round_ties_even() as i64)
}

fn rotation(style: &BTreeMap<String, String>) -> i64 {
    let raw = style
        .get("rotation")
        .map_or("0", String::as_str)
        .trim_end_matches(['f', 'd']);
    match raw.trim().parse::<f64>() {
        #[allow(clippy::cast_possible_truncation)]
        Ok(f) if f.is_finite() => (f * 60000.0).round_ties_even() as i64,
        _ => 0,
    }
}

/// The first VML element a run's object (`w:pict`, `w:object`, or an
/// `mc:AlternateContent` whose branch the readers read holds one, [`crate::mc::chosen`]:
/// its Fallback where they do not understand its Choice) draws -- `v:shape`, `v:rect`, ...;
/// a `v:shapetype` only defines one -- and its path from the object (`v:shape[1]`).
pub fn vml_shape(node: &Element) -> Option<(&Element, String)> {
    match local(node) {
        "AlternateContent" => {
            let branch = crate::mc::chosen(node)?;
            let inner = elements(branch).find(|c| matches!(local(c), "pict" | "object"))?;
            let (shape, path) = vml_shape(inner)?;
            Some((shape, format!("mc:{}/w:{}[1]/{path}", local(branch), local(inner))))
        }
        "pict" | "object" => {
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for element in elements(node) {
                let l = local(element);
                let count = counts.entry(l).or_insert(0);
                *count += 1;
                if element.name().namespace() == Some(VML) && SHAPES.contains(&l) {
                    return Some((element, format!("v:{l}[{count}]")));
                }
            }
            None
        }
        _ => None,
    }
}

/// The DrawingML preset a VML shape draws as (`v:rect`, `v:roundrect`, `v:oval`, or the
/// shape type Word writes for one), `None` for another.
pub fn preset_of(shape: &Element) -> Option<&'static str> {
    match local(shape) {
        "rect" => return Some("rect"),
        "roundrect" => return Some("roundRect"),
        "oval" => return Some("ellipse"),
        _ => {}
    }
    match attr(shape, "type").unwrap_or("").trim_start_matches('#') {
        "_x0000_t1" | "_x0000_t202" => Some("rect"),
        "_x0000_t2" => Some("roundRect"),
        "_x0000_t3" => Some("ellipse"),
        _ => None,
    }
}

impl Reader<'_> {
    fn vml_shape_graphic(&self, shape: &Element, path: &str, base: Graphic, fields: &mut Fields) -> Graphic {
        let boxed = child(shape, "textbox");
        let Some(content) = boxed.and_then(|b| child(b, "txbxContent")) else {
            return Graphic {
                kind: text("shape"),
                ..base
            };
        };
        let blocks = self.text_box_story(content, &format!("{path}/v:textbox/w:txbxContent"), fields);
        let stated: Vec<Option<i64>> = boxed
            .and_then(|b| attr(b, "inset"))
            .unwrap_or("")
            .split(',')
            .map(|p| length_emu(Some(p)))
            .collect();
        let body: Vec<(Text, BodyValue)> = ["lIns", "tIns", "rIns", "bIns"]
            .iter()
            .zip(INSETS)
            .enumerate()
            .map(|(k, (name, default))| {
                let value = stated.get(k).copied().flatten().unwrap_or(default);
                (text(name), BodyValue::Int(Some(value)))
            })
            .collect();
        Graphic {
            kind: text("shape"),
            text: Arc::from(blocks),
            body: Arc::from(body),
            ..base
        }
    }

    fn vml_graphic(&self, (holder, shape): (&Element, &Element), path: &str, fields: &mut Fields) -> Graphic {
        let style = style_of(shape);
        let flip = style.get("flip").map_or("", String::as_str);
        let base = Graphic {
            rotation: rotation(&style),
            flip_h: flip.contains('x'),
            flip_v: flip.contains('y'),
            path: text(path),
            source: Some(holder.clone()),
            ..Graphic::default()
        };
        if let Some(image) = child(shape, "imagedata") {
            let id = image
                .attribute(Some(R), "id")
                .filter(|i| !i.is_empty())
                .or_else(|| attr(image, "id"));
            if let Some(id) = id.filter(|i| !i.is_empty()) {
                return Graphic {
                    kind: text("picture"),
                    relationship: Some(text(id)),
                    source: None,
                    ..base
                };
            }
        }
        let textpath = child(shape, "textpath").filter(|t| attr(t, "string").is_some_and(|s| !s.is_empty()));
        if preset_of(shape).is_some() && textpath.is_none() {
            return self.vml_shape_graphic(shape, path, base, fields);
        }
        if textpath.is_some() {
            return Graphic {
                kind: text("textpath"),
                ..base
            };
        }
        Graphic {
            kind: text("other"),
            uri: Some(text(&format!("vml:{}", local(shape)))),
            path: text(path),
            ..Graphic::default()
        }
    }

    /// A run's VML object; `None` where it is not VML or states no size.
    pub(crate) fn vml(&self, node: &Element, path: &str, fields: &mut Fields) -> Option<Vml> {
        let (shape, inner) = vml_shape(node)?;
        let path = format!("{path}/{inner}");
        let style = style_of(shape);
        let width = length_emu(style.get("width").map(String::as_str)).filter(|w| *w > 0)?;
        let height = length_emu(style.get("height").map(String::as_str)).filter(|h| *h > 0)?;
        let graphic = self.vml_graphic((node, shape), &path, fields);
        if style.get("position").map(String::as_str) != Some("absolute") {
            let shown = match &*graphic.kind {
                "picture" => graphic
                    .relationship
                    .clone()
                    .map_or(InlineObject::None, InlineObject::Picture),
                "shape" | "textpath" => InlineObject::Graphic(Arc::new(graphic)),
                _ => InlineObject::None,
            };
            return Some(Vml::Inline(format!("drawing:{width}:{height}:0:0:0:0"), shown));
        }
        Some(Vml::Anchor(Box::new(anchor(shape, &style, (width, height), graphic))))
    }
}

fn position(style: &BTreeMap<String, String>, horizontal: bool) -> AnchorPosition {
    let kind = if horizontal { "horizontal" } else { "vertical" };
    let relative_of = |name: &str| -> Option<&'static str> {
        Some(match (horizontal, name) {
            (_, "margin") => "margin",
            (_, "page") => "page",
            (true, "text") => "column",
            (false, "text") => "paragraph",
            (true, "char") => "character",
            (false, "line") => "line",
            (true, "left-margin-area") => "leftMargin",
            (true, "right-margin-area") => "rightMargin",
            (false, "top-margin-area") => "topMargin",
            (false, "bottom-margin-area") => "bottomMargin",
            (_, "inner-margin-area") => "insideMargin",
            (_, "outer-margin-area") => "outsideMargin",
            _ => return None,
        })
    };
    let stated = style
        .get(&format!("mso-position-{kind}-relative"))
        .map_or("text", String::as_str);
    let relative = relative_of(stated).or_else(|| relative_of("text")).map(text);
    let aligns: &[&str] = if horizontal {
        &["left", "center", "right", "inside", "outside"]
    } else {
        &["top", "center", "bottom", "inside", "outside"]
    };
    if let Some(align) = style
        .get(&format!("mso-position-{kind}"))
        .filter(|a| aligns.contains(&a.as_str()))
    {
        return AnchorPosition {
            relative,
            offset: None,
            align: Some(text(align)),
        };
    }
    let (margin, edge) = if horizontal {
        ("margin-left", "left")
    } else {
        ("margin-top", "top")
    };
    let offset = length_emu(style.get(margin).map(String::as_str)).unwrap_or(0)
        + length_emu(style.get(edge).map(String::as_str)).unwrap_or(0);
    AnchorPosition {
        relative,
        offset: Some(offset),
        align: None,
    }
}

fn anchor(shape: &Element, style: &BTreeMap<String, String>, extent: (i64, i64), graphic: Graphic) -> Anchor {
    let wrap_type = elements(shape)
        .find(|c| local(c) == "wrap")
        .and_then(|w| attr(w, "type"))
        .filter(|t| !t.is_empty());
    let wrap = match wrap_type.unwrap_or("none") {
        "square" => "wrapSquare",
        "tight" => "wrapTight",
        "through" => "wrapThrough",
        "topAndBottom" => "wrapTopAndBottom",
        _ => "wrapNone",
    };
    let z = style.get("z-index").and_then(|z| py_int(z)).unwrap_or(0);
    Anchor {
        extent,
        h: position(style, true),
        v: position(style, false),
        wrap: Some(text(wrap)),
        behind: z < 0,
        relative_height: z.unsigned_abs().rem_euclid(1 << 31).try_into().unwrap_or(0),
        allow_overlap: true,
        layout_in_cell: true,
        graphic: Some(Arc::new(graphic)),
        identifier: attr(shape, "id").map(text),
        vml: true,
        ..Anchor::default()
    }
}
