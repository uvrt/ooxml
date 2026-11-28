//! Owns: glyphs painted with Word 2010 text effects (`w14:textFill`, `w14:textOutline`), as
//! docx2svg's `svg._paint` paints a span: the effects' DrawingML read by its local names, its
//! colours resolved against the theme, then filled and outlined by the shared renderers
//! under Word's rules at 300 dpi -- a gradient spanning the span's text -- their resources
//! added to the page's.
//!
//! Must not: decide where the glyphs go (the layout has), or paint anything but their fill
//! and outline.

use super::colours::{Said, WordColours};
use super::write::attrs_text;
use ooxml_drawingml::drawable::Fill;
use ooxml_drawingml::read::fill as read_fill;
use ooxml_render::fill::{fill_attrs, outline_attrs};
use ooxml_render::Painter;
use ooxml_schema::a;
use ooxml_word_layout::paint::svg::PageDefs;
use ooxml_word_layout::paint::TextEffects;

/// The paint of glyphs in `bounds` with `effects`, `colour` their own colour where the
/// effects state no fill: attributes as written, resources added to `defs`.
pub fn paint(
    colours: &WordColours<'_>,
    effects: &TextEffects,
    (bounds, colour): ((f64, f64, f64, f64), Option<&str>),
    defs: &mut PageDefs,
) -> String {
    let mut said = Said::new();
    let fill = effects
        .fill
        .as_ref()
        .and_then(|e| read_fill::fill(Some(&e.element)))
        .and_then(|f| colours.fill(&f, None, None, &mut said));
    let outline = effects
        .outline
        .as_ref()
        .and_then(|e| read_fill::line(Some(a::Ln::new(&e.element))))
        .and_then(|o| colours.outline(Some(&o), None, &mut said));
    let measurer = ooxml_render::table_measurer(&ooxml_render::WORD);
    let mut painter = Painter::new(ooxml_render::WORD, &measurer);
    painter.dpi = super::shapes::DPI;
    painter.scope_ids(defs.scope());
    painter.resume_ids(defs.ids);
    let mut out = match &fill {
        None => format!("fill=\"#{}\"", colour.unwrap_or("000000").to_ascii_uppercase()),
        Some(found) => attrs_text(&fill_attrs(Some(found), &mut painter, Some(bounds), None)),
    };
    if let Some(outline) = &outline {
        out.push(' ');
        out.push_str(&attrs_text(&outline_attrs(
            Some(outline),
            &mut painter,
            Some(bounds),
            None,
        )));
    }
    defs.ids = painter.ids();
    defs.defs
        .extend(painter.defs.iter().map(ooxml_scene::markup::Element::to_svg));
    if matches!(fill, Some(Fill::None)) && outline.is_none() {
        return "fill=\"none\"".to_owned();
    }
    out
}
