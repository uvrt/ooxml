//! Owns: the golden harness' SVG document (ooxml-common's `tests/goldens.py`,
//! `svg_document` and `stable`): drawable elements painted by `ooxml-render` under an
//! application's rules, one element a line, the notes as comments, the resources in a
//! `defs` element a line each, and every embedded PNG written as its pixels' fingerprint.
//!
//! Must not: be used by any library; it is a test instrument.

use crate::pixels::stable;
use ooxml_drawingml::drawable::Element;
use ooxml_render::{render_element, table_measurer, Painter, POWERPOINT, WORD};
use ooxml_scene::python_g;

/// `elements` painted into one document of `width_emu` by `height_emu`.
pub fn document(elements: &[Element], width_emu: f64, height_emu: f64, rules: &str, notes: &[String]) -> String {
    let rules = if rules == "word" { WORD } else { POWERPOINT };
    let measurer = table_measurer(&rules);
    let mut painter = Painter::new(rules, &measurer);
    let body: Vec<String> = elements
        .iter()
        .map(|e| render_element(e, &mut painter).to_svg())
        .collect();
    let (width, height) = (width_emu / 914_400.0 * 96.0, height_emu / 914_400.0 * 96.0);
    let (w, h) = (python_g(width), python_g(height));
    let mut lines = vec![format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"0 0 {w} {h}\" width=\"{w}\" height=\"{h}\">"
    )];
    lines.extend(notes.iter().map(|n| format!("<!-- {n} -->")));
    if !painter.defs.is_empty() {
        lines.push("<defs>".into());
        lines.extend(painter.defs.iter().map(ooxml_scene::markup::Element::to_svg));
        lines.push("</defs>".into());
    }
    lines.extend(body.into_iter().filter(|b| !b.is_empty()));
    lines.push("</svg>".into());
    stable(&(lines.join("\n") + "\n"))
}
