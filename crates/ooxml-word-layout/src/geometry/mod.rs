//! Owns: a page's geometry, made lazily from a layout (docs/architecture/README.md,
//! section 8: paginate, then geometry per page): its frame -- its size in device px and
//! points, its text area ([`PageFrame`], docx2svg's `_new_page` and `paginate._geometry`) --
//! and every line it draws of the body's flow, on whole device pixels: its baseline and the
//! rows its text fills ([`DrawnLine`], docx2svg's `_walk`, `_table_drawn`, `_drop_cap` and
//! `_continuation` placing lines with `baseline_in_box` and `text_band`). A line's box is
//! its pitch with what of its paragraph's spacing and borders is in it ([`linebox`]).
//!
//! Must not: decide where a page breaks or where a line is (the paginator has), draw (the
//! paint is [`crate::paint`]'s), or hold anything between calls.

pub mod footnotes;
pub mod linebox;
mod lines;
pub mod stories;
pub mod walk;

pub use lines::{DrawnLine, LineSource};

use crate::engine::DocumentLayout;
use crate::paginate::{LaidPage, PageInfo};
use crate::px::Px;
use crate::sections::SectionGeometry;
use crate::stories::StoryHeights;
use ooxml_base::Q;

/// A page as docx2svg makes it before anything is drawn on it: its size and its text
/// area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageFrame {
    /// Its width in whole device px (Word's export rounds the page to them).
    pub width_px: i64,
    /// Its height in whole device px.
    pub height_px: i64,
    /// Its width in points, as exported (the whole px in points).
    pub width_pt: Q,
    /// Its height in points.
    pub height_pt: Q,
    /// The text column's left edge: the margin and gutter on a whole pixel.
    pub left: Px,
    /// Its right edge.
    pub right: Px,
    /// Where the text area starts, px from the page's top.
    pub top: Px,
    /// Where it ends.
    pub bottom: Px,
    /// The section whose geometry it has.
    pub section: usize,
}

/// One page's geometry: its frame and the body's lines on it, in drawing order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageGeometry {
    /// Its 0-based place in the document, blank pages counted.
    pub page: usize,
    /// What the paginator knows of it.
    pub info: PageInfo,
    /// Its frame.
    pub frame: PageFrame,
    /// The body's lines it draws (none on a blank page).
    pub lines: Vec<DrawnLine>,
}

/// A page extent of `twips` in whole device px, as Word exports it (`round(pt * 300 / 72)`
/// of the authored size: half to the even px).
pub fn device_px(twips: i64) -> i64 {
    let exact = Q::new(i128::from(twips) * 5, 24);
    let floor = exact.floor();
    let rest = exact - floor;
    let half = Q::new(1, 2);
    let whole = floor.to_integer();
    let up = rest > half || (rest == half && whole % 2 != 0);
    i64::try_from(if up { whole + 1 } else { whole }).unwrap_or(i64::MAX)
}

/// The frame of a page of `section`, showing headers and footers of `info`'s kind.
pub fn page_frame(layout: &DocumentLayout, section_index: usize, info: &PageInfo) -> Option<PageFrame> {
    let section: &SectionGeometry = layout.sections.get(section_index)?;
    let props = &section.props;
    let (header, footer) = layout.stories.heights(section_index, info.story);
    let (top, bottom) = section.edges(header, footer);
    let width_px = device_px(props.page_size.width);
    let height_px = device_px(props.page_size.height);
    let m = &props.margins;
    let exact = Px::twips(m.left + m.gutter);
    let left = Px::int(exact.round_half_up()).quantise();
    let right = Px::twips(props.page_size.width - m.right) + (left - exact);
    Some(PageFrame {
        width_px,
        height_px,
        width_pt: Q::new(i128::from(width_px) * 6, 25),
        height_pt: Q::new(i128::from(height_px) * 6, 25),
        left,
        right,
        top,
        bottom,
        section: section_index,
    })
}

impl DocumentLayout {
    /// How many pages there are, blank ones included.
    pub fn page_count(&self) -> usize {
        self.pagination.page_count()
    }

    /// The page at 0-based place `page` laid out (`None` for a blank page or none).
    pub fn laid_page(&self, page: usize) -> Option<&LaidPage> {
        let pages = &self.pagination.pages;
        let k = pages.partition_point(|p| p.page < page);
        pages.get(k).filter(|p| p.page == page)
    }

    /// Page `page`'s geometry (0-based, blank pages counted); `None` past the last.
    pub fn page_geometry(&self, page: usize) -> Option<PageGeometry> {
        let info = *self.pagination.info.get(page)?;
        let frame = page_frame(self, info.section, &info)?;
        let lines = match self.laid_page(page) {
            Some(laid) => lines::body_lines(self, laid, &frame),
            None => Vec::new(),
        };
        Some(PageGeometry {
            page,
            info,
            frame,
            lines,
        })
    }
}
