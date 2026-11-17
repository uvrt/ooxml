//! Owns: a `w:sectPr` as [`SectionProps`] and section discovery ([`sections`]): every
//! section but the last is ended by the `w:sectPr` in its last paragraph's properties (that
//! paragraph belongs to it), the last by the body's own -- docx2svg's `_section` and
//! `_sections`. A stated 0 is 0 (a header at `w:header="0"` is at the page's edge).
//!
//! Must not: lay out pages.

use crate::model::{Block, LineNumbering, NoteNumbering, PageMargins, PageSize, Section, SectionProps};
use crate::value::{text, Text};
use crate::w;
use ooxml_xml::Element;
use std::sync::Arc;

fn page_size(sect: w::SectPr<'_>) -> PageSize {
    let size = sect.page_size();
    PageSize {
        width: size.and_then(w::PgSz::w).filter(|v| *v != 0).unwrap_or(12240),
        height: size.and_then(w::PgSz::h).filter(|v| *v != 0).unwrap_or(15840),
        orientation: size.and_then(w::PgSz::orient).map(text),
    }
}

fn margins(sect: w::SectPr<'_>) -> PageMargins {
    let Some(m) = sect.margins() else {
        return PageMargins::default();
    };
    let d = PageMargins::default();
    PageMargins {
        top: m.top().unwrap_or(d.top),
        right: m.right().unwrap_or(d.right),
        bottom: m.bottom().unwrap_or(d.bottom),
        left: m.left().unwrap_or(d.left),
        header: m.header().unwrap_or(d.header),
        footer: m.footer().unwrap_or(d.footer),
        gutter: m.gutter().unwrap_or(d.gutter),
    }
}

fn notes(pr: Option<w::NotePr<'_>>) -> NoteNumbering {
    let val = |v: Option<w::Val<'_>>| v.and_then(w::Val::val).map(text);
    NoteNumbering {
        format: val(pr.and_then(w::NotePr::format)),
        start: pr.and_then(w::NotePr::start).and_then(w::Val::int),
        restart: val(pr.and_then(w::NotePr::restart)),
        position: val(pr.and_then(w::NotePr::position)),
    }
}

fn line_numbering(element: Option<w::LnNumType<'_>>) -> Option<LineNumbering> {
    let e = element?;
    let count_by = e.count_by();
    if count_by.is_some_and(|c| c < 1) {
        return None;
    }
    Some(LineNumbering {
        count_by: count_by.unwrap_or(1),
        start: e.start().unwrap_or(0),
        distance: e.distance(),
        restart: text(e.restart().filter(|r| !r.is_empty()).unwrap_or("newPage")),
    })
}

/// A section's text columns.
struct Columns {
    count: i64,
    space: i64,
    widths: Arc<[(i64, i64)]>,
    separator: bool,
}

fn columns(sect: w::SectPr<'_>) -> Columns {
    let Some(cols) = sect.columns() else {
        return Columns {
            count: 1,
            space: 720,
            widths: Arc::from(Vec::new()),
            separator: false,
        };
    };
    let unequal = matches!(cols.equal_width(), Some("0" | "false"));
    let widths: Vec<(i64, i64)> = if unequal {
        cols.columns()
            .map(|c| (c.w().unwrap_or(0), c.space().unwrap_or(0)))
            .collect()
    } else {
        Vec::new()
    };
    let count = cols.num().filter(|n| *n != 0).unwrap_or(1).max(1);
    let space = if cols.space_raw().is_some_and(|s| !s.is_empty()) {
        cols.space()
    } else {
        None
    };
    let separator = matches!(cols.sep(), Some("1" | "true" | "on"));
    Columns {
        count,
        space: space.unwrap_or(720),
        widths: Arc::from(widths),
        separator,
    }
}

/// A `w:sectPr`'s properties.
pub fn section_props(element: &Element) -> SectionProps {
    let sect = w::SectPr::new(element);
    let cols = columns(sect);
    let headers: Vec<(Text, Text)> = sect.headers().filter_map(reference).collect();
    let footers: Vec<(Text, Text)> = sect.footers().filter_map(reference).collect();
    let pg = sect.page_numbers();
    SectionProps {
        page_size: page_size(sect),
        margins: margins(sect),
        start_type: sect.kind().and_then(w::Val::val).map(text),
        column_count: cols.count,
        column_space: cols.space,
        columns: cols.widths,
        column_separator: cols.separator,
        headers: Arc::from(headers),
        footers: Arc::from(footers),
        title_page: sect
            .title_page()
            .is_some_and(|t| crate::xmlread::attr_on(t.element(), "val")),
        page_number_start: pg.and_then(w::PgNumType::start),
        page_number_format: pg.and_then(w::PgNumType::fmt).map(text),
        vertical_alignment: sect.vertical_alignment().and_then(w::Val::val).map(text),
        endnotes: notes(sect.endnotes()),
        footnotes: notes(sect.footnotes()),
        line_numbering: line_numbering(sect.line_numbers()),
    }
}

fn reference(r: w::StoryReference<'_>) -> Option<(Text, Text)> {
    let id = r.id().filter(|i| !i.is_empty())?;
    Some((text(r.kind().filter(|k| !k.is_empty()).unwrap_or("default")), text(id)))
}

/// The sections of a story's top-level paragraphs, the last ended by `last` (the body's
/// own `w:sectPr`).
pub fn sections(body: &[Block], last: Option<&Element>) -> Vec<Section> {
    let mut out = Vec::new();
    let mut first = 0;
    let mut count = 0;
    for block in body {
        let Block::Paragraph(p) = block else { continue };
        count += 1;
        if let Some(props) = &p.section {
            out.push(Section {
                properties: Arc::clone(props),
                first_paragraph: first,
                last_paragraph: count,
            });
            first = count;
        }
    }
    if let Some(last) = last {
        out.push(Section {
            properties: Arc::new(section_props(last)),
            first_paragraph: first,
            last_paragraph: count,
        });
    } else if out.is_empty() {
        let props = SectionProps {
            page_size: PageSize {
                width: 12240,
                height: 15840,
                orientation: None,
            },
            margins: PageMargins::default(),
            start_type: None,
            column_count: 1,
            column_space: 720,
            columns: Arc::from(Vec::new()),
            column_separator: false,
            headers: Arc::from(Vec::new()),
            footers: Arc::from(Vec::new()),
            title_page: false,
            page_number_start: None,
            page_number_format: None,
            vertical_alignment: None,
            endnotes: NoteNumbering::default(),
            footnotes: NoteNumbering::default(),
            line_numbering: None,
        };
        out.push(Section {
            properties: Arc::new(props),
            first_paragraph: 0,
            last_paragraph: count,
        });
    }
    out
}
