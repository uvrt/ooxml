//! Owns: a chart drawn as ooxml-common's golden harness draws one (`tests/goldens.py`,
//! `chart_element` and `svg_document`): laid out by `ooxml-chart` in a frame of the case's
//! size, its elements put in a group mapping the frame onto itself, and painted by
//! `ooxml-render` under the case's rules into one SVG document.
//!
//! Must not: be used by any library; it is a test instrument.

use super::{chart_of, laid, locale_of};
use crate::drawing::document;
use ooxml_drawingml::drawable::{Element, Group, Transform};
use serde_json::Value as J;

/// EMU a point.
const EMU_PER_POINT: f64 = 12_700.0;

/// One golden case (a JSON object: `file`, `width_pt`, `height_pt`, `rules`, `locale`)
/// drawn as the golden harness writes it.
///
/// # Errors
/// When the case cannot be read.
pub fn golden(spec: &J) -> Result<String, String> {
    let Some(chart) = chart_of(spec)? else {
        return Ok("<!-- no c:chart -->\n".into());
    };
    let number = |key: &str| spec.get(key).and_then(J::as_f64).ok_or(format!("a case states {key}"));
    let rules = spec.get("rules").and_then(J::as_str).unwrap_or("powerpoint");
    let case = laid::Case {
        chart: &chart,
        width: number("width_pt")?,
        height: number("height_pt")?,
        rules,
        locale: locale_of(spec)?,
        variant: None,
    };
    let Some(laid_out) = laid::lay(&case) else {
        let mut kinds: Vec<&str> = chart.plots.iter().map(|p| p.kind.as_str()).collect();
        kinds.sort_unstable();
        return Ok(format!("<!-- not drawn: {} -->\n", kinds.join(",")));
    };
    let notes = laid::notes(&laid_out);
    let frame = Transform::of(0.0, 0.0, case.width * EMU_PER_POINT, case.height * EMU_PER_POINT);
    let group = Element::Group(Box::new(Group {
        transform: frame,
        child_transform: frame,
        children: laid_out.elements,
        ..Group::default()
    }));
    Ok(document(
        &[group],
        frame.extent_width,
        frame.extent_height,
        rules,
        &notes,
    ))
}
