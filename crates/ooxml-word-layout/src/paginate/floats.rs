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
use crate::sections::SectionGeometry;
use crate::wrap::{Column, Wrap};
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

/// What the lines of one layout of a page keep off: the bands of its drawings, and the
/// drawings text wraps beside.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Floats<'f> {
    pub bands: &'f [Band],
    pub wraps: &'f [Wrap],
}

/// The text column of a section's page as `crate::wrap` measures from it; one of several
/// `offset` twips from the text area's left and `width` wide, its start rounded as the
/// layout rounds a column's (`wrap_column`).
pub(super) fn wrap_column(section: &SectionGeometry, mode15: bool, column: Option<(i64, i64)>) -> Column {
    let m = &section.props.margins;
    let (offset, width) = column.unwrap_or((0, section.column_width(0)));
    let total = m.left + m.gutter + offset;
    let exact = Q::new(i128::from(total) * 300, 1440);
    let half_up = |x: Q| (x + Q::new(1, 2)).floor();
    let total_units = ooxml_text::twips(total);
    let (n, d) = total_units.parts();
    let start = if offset != 0 && mode15 {
        half_up(Q::new(n, d) * Px::layout_unit())
    } else {
        half_up(exact)
    };
    let (wn, wd) = ooxml_text::twips(width).parts();
    Column {
        exact,
        start,
        width: Q::new(wn, wd),
        mode15,
        width_px: Q::new(i128::from(width) * 300, 1440),
    }
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

    /// The drawings text wraps beside anchored on the lines `tops` placed, each
    /// positioned on page `page` from the page laid out without them (`page_wraps`).
    pub(super) fn page_wraps(&self, tops: &Tops, page: usize) -> Vec<Wrap> {
        let mut out = Vec::new();
        for top in tops.iter() {
            if let Some(FlowItem::Table(t)) = self.items.get(top.item) {
                out.extend(self.table_wrap(t, top));
                continue;
            }
            let Some(FlowItem::Para(item)) = self.items.get(top.item) else {
                continue;
            };
            if let Some(drop) = item.drop_cap.as_ref().filter(|d| top.line == 0 && d.beside()) {
                if let Some(wrap) = self.drop_cap_wrap(item, top, drop) {
                    out.push(wrap);
                }
            }
            let lines = self.lines_of(top.item, item);
            for anchored in lines.anchors().iter().filter(|a| a.line == top.line) {
                let side = anchored
                    .anchor
                    .wrap
                    .as_deref()
                    .is_some_and(|w| crate::floating::SIDE_WRAPS.contains(&w));
                if side {
                    if let Some(wrap) = self.side_wrap(item, top, page, &anchored.anchor) {
                        out.push(wrap);
                    }
                }
            }
        }
        out
    }

    /// A floating table as text wraps beside it on its page (`table_wrap`), positioned
    /// against the top of the paragraph after it (`top.paragraph_top`): its box -- its
    /// outer grid lines and its edges past them, from its top to below its bottom border --
    /// widened by `topFromText` and `bottomFromText` and, across, by `leftFromText` and
    /// `rightFromText`, each at least 10 twips; text goes beside it on both sides.
    fn table_wrap(&self, t: &crate::flow::TableItem, top: &Top) -> Option<Wrap> {
        let floating = t.flow.floating()?;
        let section = self.sections.get(t.section)?;
        let height = table_height(t, self.mode15);
        let f = &floating.position;
        let y = f.top(&section.props, top.paragraph_top, height);
        let least = crate::table::MIN_FROM_TEXT;
        let zero = Q::from_integer(0);
        Some(Wrap {
            top: (y - Px::twips(f.top)).value(),
            bottom: (y + height + Px::twips(f.bottom)).value(),
            left: floating.first - floating.edges.0 * twip_px(),
            right: floating.last + floating.edges.1 * twip_px(),
            dist_left: Q::from_integer(i128::from(f.left.max(least))) * twip_px(),
            dist_right: Q::from_integer(i128::from(f.right.max(least))) * twip_px(),
            side: "bothSides".to_owned(),
            y: y.value(),
            polygon: None,
            dist_top: zero,
            dist_bottom: zero,
            half_up: true,
            text_shift: None,
            drop_cap: None,
            table: Some(top.item),
        })
    }

    /// A drop cap's frame as text wraps beside it: from its paragraph's top down its
    /// height, from the column's exact edge to where the text beside it starts.
    fn drop_cap_wrap(&self, item: &ParaItem, top: &Top, drop: &crate::flow::DropCap) -> Option<Wrap> {
        let section = self.sections.get(item.section)?;
        let column = wrap_column(section, self.mode15, top.column);
        let (start, shift) = drop.text_start(column.start, column.exact, column.mode15);
        let paragraph_top = top.paragraph_top.value();
        let zero = Q::from_integer(0);
        Some(Wrap {
            top: paragraph_top,
            bottom: paragraph_top + drop.height().value(),
            left: column.exact,
            right: column.exact + start - column.start,
            dist_left: zero,
            dist_right: zero,
            side: "bothSides".to_owned(),
            y: paragraph_top,
            polygon: None,
            dist_top: zero,
            dist_bottom: zero,
            half_up: false,
            text_shift: Some(shift),
            drop_cap: Some(top.item),
            table: None,
        })
    }

    /// One drawing text wraps beside, positioned on its page.
    fn side_wrap(
        &self,
        item: &ParaItem,
        top: &Top,
        page: usize,
        anchor: &ooxml_word_model::model::Anchor,
    ) -> Option<Wrap> {
        let mut frames = self.frames(item, top, page, Px::ZERO)?;
        let in_column = anchor.h.relative.as_deref().is_none_or(|r| r == "column");
        if !self.mode15 && anchor.simple.is_none() && in_column && anchor.h.offset.is_none() {
            if let Some(source) = &item.source {
                // Below mode 15 a drawing aligned in the column is aligned between the
                // paragraph's indents.
                let indent = |key: &str| {
                    source
                        .resolved
                        .paragraph
                        .get(key)
                        .and_then(ooxml_word_model::value::Value::int)
                        .unwrap_or(0)
                };
                frames.left += indent("ind.left");
                frames.right += indent("ind.right");
            }
        }
        let mut x = crate::floating::horizontal(anchor, &frames)?;
        let y = vertical(anchor, &frames)?;
        if !self.mode15 {
            // ... and on a whole twip, the nearest.
            x = (x + Q::new(1, 2)).floor();
        }
        let twips = |v: i64| Q::from_integer(i128::from(emu_twips(v)));
        let (cx, cy) = (emu_twips(anchor.extent.0), emu_twips(anchor.extent.1));
        let [el, et, er, eb] = anchor.effect.map(twips);
        let [mut dt, mut db, dl, dr] = anchor.distance.map(|d| twips(d) * twip_px());
        let tight = matches!(anchor.wrap.as_deref(), Some("wrapTight" | "wrapThrough"));
        let polygon: Option<Vec<(Q, Q)>> = anchor
            .polygon
            .as_ref()
            .filter(|p| tight && !p.is_empty())
            .map(|points| {
                points
                    .iter()
                    .map(|(a, b)| {
                        let px = (x + Q::from_integer(i128::from(cx) * i128::from(*a) / 21600)) * twip_px();
                        let py = (y + Q::from_integer(i128::from(cy) * i128::from(*b) / 21600)) * twip_px();
                        (px, py)
                    })
                    .collect()
            });
        if polygon.is_some() && !self.mode15 {
            // Below mode 15 a wrap polygon keeps text off its own height only.
            dt = Q::from_integer(0);
            db = Q::from_integer(0);
        }
        let (top_px, bottom_px) = match &polygon {
            Some(points) => (
                points.iter().map(|p| p.1).min()? - dt,
                points.iter().map(|p| p.1).max()? + db,
            ),
            None => (
                (y - et) * twip_px() - dt,
                (y + Q::from_integer(i128::from(cy)) + eb) * twip_px() + db,
            ),
        };
        Some(Wrap {
            top: top_px,
            bottom: bottom_px,
            left: (x - el) * twip_px(),
            right: (x + Q::from_integer(i128::from(cx)) + er) * twip_px(),
            dist_left: dl,
            dist_right: dr,
            side: anchor.wrap_text.as_deref().unwrap_or("bothSides").to_owned(),
            y: y * twip_px(),
            polygon,
            dist_top: dt,
            dist_bottom: db,
            half_up: false,
            text_shift: None,
            drop_cap: None,
            table: None,
        })
    }
}

/// A table's height placed whole from 0, borders included (a floating table's).
pub(super) fn table_height(t: &crate::flow::TableItem, mode15: bool) -> Px {
    let far = Px::int(10_000_000);
    let request = crate::table::TableRequest {
        y: Px::ZERO,
        bottom: far,
        row: 0,
        part: Vec::new(),
        first_on_page: false,
        mode15,
        split_kept: false,
        stop: None,
    };
    t.flow.place(&request).map_or(Px::ZERO, |laid| laid.end)
}
