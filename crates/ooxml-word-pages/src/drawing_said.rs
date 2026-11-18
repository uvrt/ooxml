//! Owns: what a version's drawings say as its warnings are gathered ([`DrawingWarnings`],
//! the warnings' `PageDrawings` port): each page that holds drawings painted, its painters'
//! warnings and approximations as docx2svg says them while it draws.
//!
//! Must not: keep a page's paint (the engine's paint memo does), or say anything else.

use crate::drawings::WordDrawings;
use crate::fonts::DocumentFonts;
use crate::warnings::{Approximation, DrawingSaid, LayoutWarning};
use ooxml_word_layout::paint::{paint_page, GlyphSize, PaintContext};
use ooxml_word_layout::WordLayout;
use ooxml_word_model::Model;

/// What a version's drawings say, painted page by page as its warnings are gathered
/// (docx2svg says them as it draws them, laying out).
pub(crate) struct DrawingWarnings<'a> {
    pub(crate) layout: &'a mut WordLayout,
    pub(crate) fonts: &'a DocumentFonts,
    pub(crate) drawings: Option<&'a WordDrawings>,
    pub(crate) glyph_size: GlyphSize,
    pub(crate) decimal: char,
    /// The text paths said to be approximated, by part and path: once each, on the first
    /// page that draws them.
    pub(crate) text_paths: Vec<(Option<String>, String)>,
}

impl crate::warnings::PageDrawings for DrawingWarnings<'_> {
    fn said(&mut self, model: &mut Model, laid: &ooxml_word_layout::DocumentLayout, page: usize) -> Vec<DrawingSaid> {
        let Some(drawings) = self.drawings else {
            return Vec::new();
        };
        let parts = self.layout.page_parts(model, laid, page);
        let cx = PaintContext {
            faces: self.fonts.glyphs.as_ref(),
            names: self.fonts.names.as_ref(),
            glyph_size: self.glyph_size,
            decimal: self.decimal,
        };
        let Some(paint) = paint_page(laid, model, page, &parts, &cx) else {
            return Vec::new();
        };
        let number = Some(page + 1);
        let warning = |code: &str, message: String| {
            DrawingSaid::Warning(LayoutWarning {
                code: code.to_owned(),
                message,
                page: number,
            })
        };
        let mut out = Vec::new();
        for path in &paint.unplaced {
            out.push(warning(
                "drawing-position-unknown",
                format!(
                    "a floating drawing ({path}) is positioned against a frame that is not modelled: it is not drawn"
                ),
            ));
        }
        let mut defs = ooxml_word_layout::paint::svg::PageDefs::default();
        for slot in &paint.drawings {
            if slot.no_overlap {
                out.push(warning(
                    "drawing-overlap-not-modelled",
                    "a floating drawing that may not overlap another (allowOverlap=\"0\") is drawn where it is \
                     positioned; Word moves it clear of the drawings before it"
                        .into(),
                ));
            }
            let painted = drawings.paint(slot, &mut defs);
            let key = (
                slot.story.as_ref().map(|(_, part)| part.to_string()),
                slot.path.to_string(),
            );
            if painted.approximated && !self.text_paths.contains(&key) {
                self.text_paths.push(key);
                out.push(DrawingSaid::Approximation(Approximation {
                    code: "layout-approximate:text-path".into(),
                    message: format!(
                        "a VML text path ({}: a watermark's text) is drawn stretched to its box, in its face, \
                         colour, opacity and rotation, here and on every page after it that shows it; the glyphs' \
                         size and place in the box are not measured",
                        slot.path
                    ),
                    page: number,
                    path: Some(slot.path.to_string()),
                }));
            }
            for (code, message) in painted.warnings {
                out.push(warning(&code, message));
            }
        }
        out
    }
}
