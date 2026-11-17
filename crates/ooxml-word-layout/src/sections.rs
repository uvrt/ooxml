//! Owns: a document's sections as pagination reads them ([`Sections`]): each section's
//! page size and margins, its text columns ([`ColumnBox`]), whether it starts a page, the
//! number its first page takes and the blank pages Word puts before it, the header and
//! footer kind a page shows ([`StoryKind`]), and where a page's text starts and ends given
//! its header's and footer's heights -- docx2svg's `linebreak.column_width_twips`,
//! `column_boxes`, `is_multi_column` and `paginate._geometry`, `_new_page_section`,
//! `section_start_number`, `story_kind`, `_section_of`, `ends_section`.
//!
//! Must not: lay out a header or footer (a [`crate::stories::StoryHeights`] gives their
//! heights), or read XML.

use crate::px::Px;
use ooxml_word_model::model::{Section, SectionProps};
use ooxml_word_model::WordDocument;
use std::sync::Arc;

/// One text column: its offset from the text area's left edge, its width and the space
/// after it, twips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColumnBox {
    /// From the text area's left edge (the left margin and gutter).
    pub offset: i64,
    /// Its width.
    pub width: i64,
    /// The space after it.
    pub space: i64,
}

/// The header and footer a page shows (`w:headerReference/@w:type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StoryKind {
    /// `default`.
    Default,
    /// `first`: a section's first page under `w:titlePg`.
    First,
    /// `even`: an even-numbered page under `w:evenAndOddHeaders`.
    Even,
}

impl StoryKind {
    /// Every kind, in docx2svg's order.
    pub const ALL: [StoryKind; 3] = [StoryKind::Default, StoryKind::First, StoryKind::Even];

    /// The kind's name as the file writes it.
    pub fn name(self) -> &'static str {
        match self {
            StoryKind::Default => "default",
            StoryKind::First => "first",
            StoryKind::Even => "even",
        }
    }
}

/// One section's page geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionGeometry {
    /// Its properties.
    pub props: Arc<SectionProps>,
    /// Its first top-level paragraph.
    pub first_paragraph: usize,
    /// One past its last.
    pub last_paragraph: usize,
}

impl SectionGeometry {
    /// The page's text width: the page less the side margins and the gutter, twips.
    pub fn text_width(&self) -> i64 {
        let m = &self.props.margins;
        self.props.page_size.width - m.left - m.right - m.gutter
    }

    /// The width of text column `column`, twips: equal columns share the text width less
    /// the spaces between them, truncated to a whole twip; unequal ones are as stated.
    pub fn column_width(&self, column: usize) -> i64 {
        let page = self.text_width();
        let columns = &self.props.columns;
        if !columns.is_empty() {
            let k = column.min(columns.len() - 1);
            return columns.get(k).map_or(page, |c| c.0);
        }
        let count = self.props.column_count;
        if count <= 1 {
            return page;
        }
        (page - (count - 1) * self.props.column_space).div_euclid(count)
    }

    /// Whether the section lays its text out in more than one column.
    pub fn multi_column(&self) -> bool {
        self.props.column_count > 1 || self.props.columns.len() > 1
    }

    /// Every text column, left to right.
    pub fn column_boxes(&self) -> Vec<ColumnBox> {
        let widths: Vec<(i64, i64)> = if !self.props.columns.is_empty() {
            self.props.columns.to_vec()
        } else if self.props.column_count > 1 {
            let count = usize::try_from(self.props.column_count).unwrap_or(1);
            vec![(self.column_width(0), self.props.column_space); count]
        } else {
            return vec![ColumnBox {
                offset: 0,
                width: self.column_width(0),
                space: 0,
            }];
        };
        let mut offset = 0;
        widths
            .into_iter()
            .map(|(width, space)| {
                let b = ColumnBox { offset, width, space };
                offset += width + space;
                b
            })
            .collect()
    }

    /// Whether its columns differ in width.
    pub fn unequal_columns(&self) -> bool {
        let boxes = self.column_boxes();
        boxes.iter().any(|b| boxes.first().is_some_and(|f| f.width != b.width))
    }

    /// Where the page's text starts and ends, px from the page's top, given the heights
    /// its header and footer reach from their edge distances (`paginate._geometry`): the
    /// margins, pushed in where a story reaches past them; a negative margin is kept.
    pub fn edges(&self, header: Px, footer: Px) -> (Px, Px) {
        let m = &self.props.margins;
        let height = self.props.page_size.height;
        let mut top = Px::twips(m.top.abs());
        if m.top >= 0 && !header.is_zero() {
            top = top.max(Px::twips(m.header) + header);
        }
        let mut bottom = Px::twips(height - m.bottom.abs());
        if m.bottom >= 0 && !footer.is_zero() {
            bottom = bottom.min(Px::twips(height - m.footer) - footer);
        }
        (top, bottom)
    }

    /// The text area's left edge, px from the page's left (the left margin and gutter).
    pub fn text_left(&self) -> Px {
        let m = &self.props.margins;
        Px::twips(m.left + m.gutter)
    }
}

/// A document's sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sections {
    /// Each section, in order.
    pub list: Vec<SectionGeometry>,
    /// `w:evenAndOddHeaders`.
    pub even_and_odd: bool,
}

impl Sections {
    /// The sections of a document.
    pub fn of(document: &WordDocument) -> Sections {
        Sections::new(&document.sections, document.context.settings.even_and_odd_headers)
    }

    /// The sections given.
    pub fn new(sections: &[Section], even_and_odd: bool) -> Sections {
        Sections {
            list: sections
                .iter()
                .map(|s| SectionGeometry {
                    props: Arc::clone(&s.properties),
                    first_paragraph: s.first_paragraph,
                    last_paragraph: s.last_paragraph,
                })
                .collect(),
            even_and_odd,
        }
    }

    /// Section `k`; the last for an index past the end.
    pub fn get(&self, k: usize) -> Option<&SectionGeometry> {
        self.list.get(k).or_else(|| self.list.last())
    }

    /// The section that holds top-level paragraph `paragraph` (`_section_of`).
    pub fn section_of(&self, paragraph: usize) -> usize {
        self.list
            .iter()
            .position(|s| s.first_paragraph <= paragraph && paragraph < s.last_paragraph)
            .unwrap_or(self.list.len().saturating_sub(1))
    }

    /// Whether top-level paragraph `paragraph` holds a section break: the `w:sectPr` of a
    /// section other than the last (`ends_section`).
    pub fn ends_section(&self, paragraph: usize) -> bool {
        let n = self.list.len();
        self.list
            .iter()
            .take(n.saturating_sub(1))
            .any(|s| s.last_paragraph >= 1 && s.last_paragraph - 1 == paragraph && paragraph + 1 > s.first_paragraph)
    }

    /// Whether section `k` starts a new page: every kind but `continuous` (and
    /// `nextColumn`), and a continuous one whose page size differs from the one before's.
    pub fn new_page(&self, k: usize) -> bool {
        let Some(here) = self.get(k) else { return true };
        match here.props.start_type.as_deref().unwrap_or("nextPage") {
            "nextPage" | "oddPage" | "evenPage" => true,
            _ => {
                k > 0
                    && self
                        .get(k - 1)
                        .is_some_and(|p| p.props.page_size != here.props.page_size)
            }
        }
    }

    /// Whether any section has several columns.
    pub fn any_multi_column(&self) -> bool {
        self.list.iter().any(SectionGeometry::multi_column)
    }

    /// The story kind a page shows (`story_kind`).
    pub fn story_kind(&self, section: usize, section_first: bool, number: i64) -> StoryKind {
        if section_first && self.get(section).is_some_and(|s| s.props.title_page) {
            StoryKind::First
        } else if self.even_and_odd && number % 2 == 0 {
            StoryKind::Even
        } else {
            StoryKind::Default
        }
    }

    /// A page-starting section's first page number and the blank pages Word puts before
    /// it, given the number of the page before and the 0-based index its page would have
    /// (`section_start_number`).
    pub fn start_number(&self, section: usize, previous: i64, physical: i64) -> (i64, i64) {
        let Some(here) = self.get(section) else {
            return (previous + 1, 0);
        };
        let start = here.props.page_number_start;
        let mut number = start.unwrap_or(previous + 1);
        let mut blanks = 0;
        let kind = here.props.start_type.as_deref();
        let wanted = if kind == Some("oddPage") { 1 } else { 0 };
        if matches!(kind, Some("oddPage" | "evenPage")) && number.rem_euclid(2) != wanted {
            if start.is_none() {
                blanks = 1;
            }
            number += 1;
        }
        if self.even_and_odd && (physical + blanks + 1).rem_euclid(2) != number.rem_euclid(2) {
            blanks += 1;
        }
        (number, blanks)
    }
}
