//! Owns: what a check reads of one painted page ([`PageRead`]): its lines' text
//! ([`LineText`]) and the boxes of what it draws beside them ([`PageBoxes`]) -- each rule's
//! path and horizontal extent, and the body's inline pictures and drawings as placed --
//! taken from one paint, so that a check paints a page once for all it reads of it
//! (docx-agent's `layout_facts` reading `page.rules`, `page.pictures` and `page.floats`).
//!
//! Must not: paint (the engine does), or judge what it reads.

use crate::line_text::LineText;
use ooxml_word_layout::paint::PagePaint;
use ooxml_word_layout::px::Px;

/// A box drawn on a page: what it is drawn for and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Boxed {
    /// Its path (a picture's run's, a drawing's `wp:anchor` or run's).
    pub path: String,
    /// Its left edge, px from the page's left.
    pub x: Px,
    /// Its top.
    pub y: Px,
    /// Its width.
    pub width: Px,
    /// Its height.
    pub height: Px,
}

/// A rule's path and horizontal extent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleExtent {
    /// The path of what it is drawn for.
    pub path: String,
    /// Its left edge, px.
    pub x: Px,
    /// Its width.
    pub width: Px,
}

/// What a page draws beside its lines' text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageBoxes {
    /// Every rule, in paint order.
    pub rules: Vec<RuleExtent>,
    /// The body's inline pictures (the document's own, `page.pictures`).
    pub pictures: Vec<Boxed>,
    /// The body's drawings, inline shapes and floating ones (`page.floats`).
    pub floats: Vec<Boxed>,
}

/// One painted page as a check reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageRead {
    /// Its lines' text, in paint order.
    pub texts: Vec<LineText>,
    /// What it draws beside them.
    pub boxes: PageBoxes,
}

impl PageRead {
    /// What a check reads of `paint`.
    #[must_use]
    pub fn of(paint: &PagePaint) -> PageRead {
        let body = |path: &str| path.starts_with("w:body");
        PageRead {
            texts: paint.lines.iter().map(LineText::of).collect(),
            boxes: PageBoxes {
                rules: paint
                    .rules
                    .iter()
                    .map(|r| RuleExtent {
                        path: r.path.clone(),
                        x: r.x,
                        width: r.width,
                    })
                    .collect(),
                pictures: paint
                    .pictures
                    .iter()
                    .filter(|p| p.part.is_none() && body(&p.path))
                    .map(|p| Boxed {
                        path: p.path.clone(),
                        x: p.x,
                        y: p.y,
                        width: p.width,
                        height: p.height,
                    })
                    .collect(),
                floats: paint
                    .drawings
                    .iter()
                    .filter(|d| d.story.is_none() && body(&d.path))
                    .map(|d| Boxed {
                        path: d.path.clone(),
                        x: d.x,
                        y: d.y,
                        width: d.width,
                        height: d.height,
                    })
                    .collect(),
            },
        }
    }
}
