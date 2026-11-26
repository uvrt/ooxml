//! Owns: what a check reads of one painted page ([`PageRead`]): its lines' text
//! ([`LineText`]), its text boxes' apart, and the boxes of what it draws beside them
//! ([`PageBoxes`]) -- each rule's anchor, row and horizontal extent, and the body's inline
//! pictures and drawings as placed, each named by key ([`ItemPlace`]) --
//! taken from one paint, so that a check paints a page once for all it reads of it
//! (docx-agent's `layout_facts` reading `page.rules`, `page.pictures` and `page.floats`).
//!
//! Must not: paint (the engine does), or judge what it reads.

use crate::line_text::{table_row, ItemPlace, LineText, TableRow};
use ooxml_word_layout::paint::names::PageNames;
use ooxml_word_layout::paint::{ItemName, PagePaint};
use ooxml_word_layout::px::Px;
use ooxml_word_model::docx::VersionNames;
use ooxml_word_model::ids::{Anchor, StoryIx};

/// A box drawn on a page: what it is drawn for and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Boxed {
    /// What it is drawn for: a picture's run, a drawing's `wp:anchor` or run, below their
    /// paragraph.
    pub place: ItemPlace,
    /// Its left edge, px from the page's left.
    pub x: Px,
    /// Its top.
    pub y: Px,
    /// Its width.
    pub width: Px,
    /// Its height.
    pub height: Px,
}

/// A rule's horizontal extent, and the body table's row it is drawn in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleExtent {
    /// What it is drawn for (`None`: what no story's index holds, a separator the layout
    /// makes).
    pub anchor: Option<Anchor>,
    /// The body table's row it is drawn in.
    pub row: Option<TableRow>,
    /// Its left edge, px.
    pub x: Px,
    /// Its width.
    pub width: Px,
}

/// What a page draws beside its lines.
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
    /// Its lines' text, in paint order (docx2svg's `Page.lines`).
    pub texts: Vec<LineText>,
    /// Its text boxes' lines' text, box by box (with [`PageRead::texts`], docx2svg's
    /// `Page.text_lines`: every line of text the page draws).
    pub box_texts: Vec<LineText>,
    /// What it draws beside them.
    pub boxes: PageBoxes,
}

impl PageRead {
    /// What a check reads of `paint`, what it is drawn for named by key through the
    /// version's `names`.
    #[must_use]
    pub fn of(paint: &PagePaint, names: &VersionNames) -> PageRead {
        let page = PageNames::of_page(names, paint);
        let in_body = |anchor: Option<Anchor>, path: &str| match anchor {
            Some(anchor) => anchor.block.story() == StoryIx::BODY,
            None => path.starts_with("w:body"),
        };
        // an item below a paragraph is placed below it by its name's steps
        let below = |name: &ItemName| {
            let path = page.written(name);
            let host = name.anchor().map(|a| Anchor::whole(a.block));
            (in_body(host, &path)).then(|| ItemPlace::of(names, None, &path, host))
        };
        let host = |slot: usize| paint.drawings.get(slot).and_then(|d| d.name.anchor());
        PageRead {
            texts: paint.lines.iter().map(|l| LineText::of(l, names, None)).collect(),
            box_texts: paint
                .text_boxes
                .iter()
                .flat_map(|b| b.lines.iter().map(move |l| (l, host(b.slot))))
                .map(|(l, host)| LineText::of(l, names, host))
                .collect(),
            boxes: PageBoxes {
                rules: paint
                    .rules
                    .iter()
                    .map(|r| RuleExtent {
                        anchor: r.name.anchor(),
                        row: r.name.anchor().and_then(|a| table_row(names, a)),
                        x: r.x,
                        width: r.width,
                    })
                    .collect(),
                pictures: paint
                    .pictures
                    .iter()
                    .filter(|p| p.part.is_none())
                    .filter_map(|p| Some((p, below(&p.name)?)))
                    .map(|(p, place)| Boxed {
                        place,
                        x: p.x,
                        y: p.y,
                        width: p.width,
                        height: p.height,
                    })
                    .collect(),
                floats: paint
                    .drawings
                    .iter()
                    .filter(|d| d.story.is_none())
                    .filter_map(|d| {
                        let path = page.slot(d);
                        let host = d.name.anchor().map(|a| Anchor::whole(a.block));
                        in_body(host, &path).then(|| (d, ItemPlace::of(names, None, &path, host)))
                    })
                    .map(|(d, place)| Boxed {
                        place,
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
