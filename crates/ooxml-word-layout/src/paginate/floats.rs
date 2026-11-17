//! Owns: what a page's floating drawings do to its lines (docx2svg's `paginate`,
//! `page_bands`, `_stacked_bands`, `clear_of_bands`, `_frames`): each drawing text wraps
//! top and bottom of positioned from the page as laid out without it ([`Band`]), and the
//! lines that would reach into its band moved below it; several such drawings on a page
//! each positioned from the page laid out with the bands of those before it.
//!
//! Must not: position a drawing (`crate::floating` does) or stack lines (`scan` does).

use super::PageContext;
use crate::floating::{emu_twips, vertical, Frames};
use crate::flow::{FlowItem, ParaItem};
use crate::px::Px;
use ooxml_base::Q;

/// Device px per twip, exactly (a drawing's place is not rounded to the layout unit).
fn twip_px() -> Q {
    Q::new(300, 1440)
}

/// An exact twip length in device px.
fn exact(twips: Q) -> Px {
    Px::from(twips * twip_px())
}

/// A drawing by where it is anchored: its paragraph's block, its index in the
/// paragraph's text, its run and its index among the run's anchors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DrawingKey {
    /// The paragraph's block.
    pub block: usize,
    /// The anchor's index in the paragraph's text.
    pub source: i64,
    /// Its run.
    pub run: usize,
    /// Its index among the run's anchors.
    pub index: usize,
}

/// What a drawing text wraps top and bottom of keeps lines off: device px from the
/// page's top, its box widened by `distT` above and `distB` below; and the drawing's own
/// top as positioned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Band {
    /// The band's top.
    pub top: Px,
    /// Its foot.
    pub bottom: Px,
    /// The drawing's top.
    pub y: Px,
    /// The drawing.
    pub key: DrawingKey,
    /// Its left and right edges, where it is in a section of several columns (`None`:
    /// across the page's text).
    pub span: Option<(Px, Px)>,
}

/// Where a line that would start at `y` and reach `reach` down starts: below every band
/// it would reach into (a line touching a band's edge does not), until none.
pub fn clear_of_bands(mut y: Px, reach: Px, bands: &[Band]) -> Px {
    let mut moved = true;
    while moved {
        moved = false;
        for band in bands {
            if y < band.bottom && y + reach > band.top {
                y = band.bottom;
                moved = true;
            }
        }
    }
    y
}

/// A line a scan placed, as the drawings anchored on it are positioned against: its
/// top and pitch, its paragraph's top, and in a section of several columns its column's
/// offset and width, twips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Top {
    pub item: usize,
    pub line: usize,
    pub top: Px,
    pub pitch: Px,
    pub paragraph_top: Px,
    pub column: Option<(i64, i64)>,
}

/// The lines a page's scans placed, in the order first placed, each with what its last
/// placement found (`Tops`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Tops(Vec<Top>);

impl Tops {
    /// A line placed (again).
    pub fn record(&mut self, top: Top) {
        match self.0.iter_mut().find(|t| t.item == top.item && t.line == top.line) {
            Some(found) => *found = top,
            None => self.0.push(top),
        }
    }

    /// Every line of `other`, recorded in turn.
    pub fn extend(&mut self, other: Tops) {
        for top in other.0 {
            self.record(top);
        }
    }

    /// The lines in order.
    pub fn iter(&self) -> impl Iterator<Item = &Top> {
        self.0.iter()
    }
}

/// What the lines of one layout of a page keep off: the bands of its drawings.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Floats<'f> {
    pub bands: &'f [Band],
}

impl PageContext<'_> {
    /// What a drawing anchored on line `top` of `item` is positioned against
    /// (`_frames`): below mode 15 the paragraph's and the line's tops taken in whole
    /// twips, the nearest; `character` in px.
    pub(super) fn frames(&self, item: &ParaItem, top: &Top, page: usize, character: Px) -> Option<Frames> {
        let section = self.sections.get(item.section)?;
        let props = &section.props;
        let m = &props.margins;
        let whole = |v: Px| -> Q {
            let twips = v.value() / twip_px();
            if self.mode15 {
                twips
            } else {
                (twips + Q::new(1, 2)).floor()
            }
        };
        let line_top = if top.line == 0 { top.paragraph_top } else { top.top };
        Some(Frames {
            page_width: props.page_size.width,
            page_height: props.page_size.height,
            left: m.left + m.gutter,
            right: m.right,
            top: Q::from_integer(i128::from(m.top)),
            bottom: Q::from_integer(i128::from(m.bottom)),
            header: m.header,
            footer: m.footer,
            page: page + 1,
            mode15: self.mode15,
            paragraph_top: whole(top.paragraph_top),
            line_top: whole(line_top),
            line_bottom: whole(top.top + top.pitch),
            character: character.value() / twip_px(),
            column: top.column.map(|(offset, width)| (m.left + m.gutter + offset, width)),
        })
    }

    /// The bands of the drawings text wraps top and bottom of anchored on the lines
    /// `tops` placed, each positioned on page `page` (0-based), as tall as its extent in
    /// whole twips (`page_bands`).
    pub(super) fn page_bands(&self, tops: &Tops, page: usize) -> Vec<Band> {
        let mut out = Vec::new();
        for top in tops.iter() {
            let Some(FlowItem::Para(item)) = self.items.get(top.item) else {
                continue;
            };
            let lines = self.lines_of(top.item, item);
            for anchored in lines.anchors().iter().filter(|a| a.line == top.line) {
                let anchor = &anchored.anchor;
                if anchor.wrap.as_deref() != Some("wrapTopAndBottom") {
                    continue;
                }
                let Some(frames) = self.frames(item, top, page, Px::ZERO) else {
                    continue;
                };
                let Some(y) = vertical(anchor, &frames) else {
                    continue;
                };
                // In a column of several: the band keeps lines off the columns the
                // drawing is across, not the page's.
                let span = top.column.and_then(|_| {
                    let x = crate::floating::horizontal(anchor, &frames)?;
                    let width = Q::from_integer(i128::from(emu_twips(anchor.extent.0)));
                    Some((exact(x), exact(x + width)))
                });
                let [above, below, _, _] = anchor.distance.map(|d| Q::from_integer(i128::from(emu_twips(d))));
                let height = Q::from_integer(i128::from(emu_twips(anchor.extent.1)));
                out.push(Band {
                    top: exact(y - above),
                    bottom: exact(y + height + below),
                    y: exact(y),
                    key: DrawingKey {
                        block: item.block,
                        source: anchored.source,
                        run: anchored.run,
                        index: anchored.index,
                    },
                    span,
                });
            }
        }
        out
    }
}
