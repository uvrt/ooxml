//! Owns: where a drawing's text boxes lay their text out, as docx2svg's painter finds them
//! (`drawing.draw`'s `text_boxes`, `text_area_edges`, `automatic_text_colour`): each shape
//! holding text, in its box as the drawing is painted (a group's members on its child
//! coordinates); how far in from each side its text area starts -- its geometry's text
//! rectangle less half the width its outline states, drawn or not (`make_text_box_probe.py`)
//! -- and what its automatic text is drawn in: its style's `a:fontRef` colour, else white
//! on a solid fill darker than [`AUTOMATIC_WHITE_BELOW`] (`make_dml_probe.py`).
//!
//! Must not: lay the text out (the layout does, from each [`TextBoxFrame`]), or paint.

use super::colours::{Said, WordColours};
use super::shapes::Paint;
use super::write::PxBox;
use ooxml_base::Q;
use ooxml_drawingml::drawable::{Fill, TextRect};
use ooxml_drawingml::read::geometry as read_geometry;
use ooxml_drawingml::source::GeometrySpec;
use ooxml_render::geometry::{text_rect, TextGeometry};
use ooxml_schema::a;
use ooxml_word_layout::geometry::text_box::TextBoxFrame;
use ooxml_word_model::model::Graphic;
use ooxml_xml::view::child;
use std::sync::Arc;

/// Automatic text on a fill darker than this is drawn white (the fill's Rec. 601 luma,
/// 0-255; measured between 72 and 75.1).
pub const AUTOMATIC_WHITE_BELOW: f64 = 74.0;

/// EMU per device px.
const EMU_PER_PX: i128 = 3048;

/// Whether a drawing (or a member of its group) holds a text box.
pub fn holds_text(graphic: &Graphic) -> bool {
    !graphic.text.is_empty() || graphic.children.iter().any(|g| holds_text(g))
}

/// A float exactly as a fraction (Python's `Fraction(float)`), its denominator capped at
/// 2^80 (far below the EMU a text rectangle is measured in).
fn exact(value: f64) -> Q {
    if !value.is_finite() || value == 0.0 {
        return Q::from_integer(0);
    }
    let bits = value.to_bits();
    let sign: i128 = if bits >> 63 == 0 { 1 } else { -1 };
    let exponent = i32::try_from((bits >> 52) & 0x7ff).unwrap_or(0);
    let fraction = i128::from(bits & ((1 << 52) - 1));
    let (mantissa, exponent) = if exponent == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), exponent - 1075)
    };
    if exponent >= 0 {
        let shift = u32::try_from(exponent.min(60)).unwrap_or(0);
        return Q::from_integer(sign * (mantissa << shift));
    }
    let down = u32::try_from((-exponent).min(80)).unwrap_or(80);
    let kept = if -exponent > 80 {
        mantissa >> u32::try_from(-exponent - 80).unwrap_or(127).min(127)
    } else {
        mantissa
    };
    Q::new(sign * kept, 1i128 << down)
}

/// The width a shape's outline states, EMU -- its own `a:ln@w` over its style's line
/// reference's -- drawn or not; 0 where neither states one (`outline_width`).
fn outline_width(paint: &Paint, colours: &WordColours<'_>) -> f64 {
    let mut width = None;
    let reference = paint.style.as_ref().and_then(|s| s.line.as_ref());
    if let (Some(reference), Some(formats)) = (reference, colours.formats) {
        let index = usize::try_from(reference.idx - 1).ok();
        if reference.idx > 0 {
            width = index.and_then(|i| formats.line_styles.get(i)).and_then(|o| o.width);
        }
    }
    if let Some(stated) = paint.line.as_ref().and_then(|l| l.width) {
        width = Some(stated);
    }
    width.unwrap_or(0.0)
}

/// A custom geometry's `a:rect`, with the geometry's adjustments and guides.
fn custom_rect(graphic: &Graphic, spec: &GeometrySpec) -> Option<TextRect> {
    let GeometrySpec::Custom {
        adjustments, guides, ..
    } = spec
    else {
        return None;
    };
    let properties = graphic.source.as_ref().and_then(|e| child(e, "spPr"));
    let rect = read_geometry::text_rect(properties.map(a::ShapeProperties::new))?;
    Some(TextRect {
        adjustments: adjustments.clone(),
        guides: guides.clone(),
        rect,
    })
}

/// How far in from the left, top, right and bottom of a text box's shape (`width` by
/// `height` EMU) its text area starts, before `wps:bodyPr`'s insets, EMU
/// (`text_area_edges`): the geometry's text rectangle less half the outline's width, the
/// half in whole EMU, truncated.
fn text_area_edges(graphic: &Graphic, paint: &Paint, colours: &WordColours<'_>, (w, h): (f64, f64)) -> [Q; 4] {
    let custom = paint.geometry.as_ref().and_then(|g| custom_rect(graphic, g));
    let geometry = match (&paint.geometry, &custom) {
        (Some(GeometrySpec::Preset { name, adjust }), _) => Some(TextGeometry::PresetWritten(name, adjust)),
        (Some(GeometrySpec::Custom { .. }), Some(rect)) => Some(TextGeometry::Custom(rect)),
        _ => None,
    };
    let (left, top, right, bottom) = text_rect(geometry, w, h);
    #[allow(clippy::cast_possible_truncation)]
    let half = Q::from_integer(i128::from(outline_width(paint, colours).round_ties_even() as i64).div_euclid(2));
    [
        exact(left) + half,
        exact(top) + half,
        exact(w - right) + half,
        exact(h - bottom) + half,
    ]
}

/// What a shape's text whose colour is automatic is drawn in, `RRGGBB` (`None`: black):
/// its style's `a:fontRef` colour where it names one, else white on a solid fill darker
/// than [`AUTOMATIC_WHITE_BELOW`] (`automatic_text_colour`).
fn automatic_colour(paint: &Paint, colours: &WordColours<'_>) -> Option<String> {
    let mut said = Said::new();
    let style = paint.style.as_ref();
    if let Some(choice) = style.and_then(|s| s.font.as_ref()).and_then(|f| f.colour.as_ref()) {
        return colours
            .resolve(choice, None, &mut said)
            .map(|c| c.hex.trim_start_matches('#').to_owned());
    }
    let fill = match &paint.fill {
        Some(f) => colours.fill(f, None, None, &mut said),
        None => colours.style_fill(style.and_then(|s| s.fill.as_ref()), &mut said),
    };
    let Some(Fill::Solid(colour)) = fill else {
        return None;
    };
    let hex = colour.hex.trim_start_matches('#');
    let channel = |k: usize| {
        hex.get(k..k + 2)
            .and_then(|c| u8::from_str_radix(c, 16).ok())
            .map_or(0.0, f64::from)
    };
    let luma = 0.299 * channel(0) + 0.587 * channel(2) + 0.114 * channel(4);
    (luma < AUTOMATIC_WHITE_BELOW).then(|| "FFFFFF".to_owned())
}

/// A shape holding text in its box `at` (as the drawing is painted), as the layout lays
/// its text out.
pub fn frame(graphic: &Arc<Graphic>, at: &PxBox, colours: &WordColours<'_>) -> TextBoxFrame {
    let paint = Paint::of(graphic);
    let emu = |px: Q| -> f64 {
        let v = px * Q::from_integer(EMU_PER_PX);
        #[allow(clippy::cast_precision_loss)]
        let out = *v.numer() as f64 / *v.denom() as f64;
        out
    };
    let size = (emu(at.width.value()), emu(at.height.value()));
    TextBoxFrame {
        graphic: Arc::clone(graphic),
        x: at.x,
        y: at.y,
        width: at.width,
        height: at.height,
        edges: text_area_edges(graphic, &paint, colours, size),
        colour: automatic_colour(&paint, colours),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_are_taken_exactly() {
        assert_eq!(exact(0.5), Q::new(1, 2));
        assert_eq!(exact(-3.25), Q::new(-13, 4));
        assert_eq!(exact(12_700.0), Q::from_integer(12_700));
        assert_eq!(exact(0.1), Q::new(3_602_879_701_896_397, 36_028_797_018_963_968));
    }

    #[test]
    fn automatic_text_turns_white_on_a_dark_fill() {
        let theme = std::collections::BTreeMap::new();
        let colours = WordColours {
            theme: &theme,
            slots: std::collections::BTreeMap::new(),
            formats: None,
        };
        let solid = |hex: &str| Paint {
            fill: Some(ooxml_drawingml::source::Fill::Solid(ooxml_colour::SourceColour::Srgb {
                hex: hex.to_owned(),
                linear: None,
                transforms: Vec::new(),
            })),
            ..Paint::default()
        };
        assert_eq!(automatic_colour(&solid("484848"), &colours).as_deref(), Some("FFFFFF"));
        assert_eq!(automatic_colour(&solid("4C4C4C"), &colours), None);
        assert_eq!(automatic_colour(&Paint::default(), &colours), None);
    }
}
