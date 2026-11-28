//! Owns: a `w:sectPr` as the layout reads it ([`SectionProps`]: what it states, else the
//! layout's defaults -- a Letter page, 1-inch margins, one column), and the sections of a
//! story's top-level paragraphs ([`story_sections`]): every section but the last is ended
//! by the `w:sectPr` in its last paragraph's properties (that paragraph belongs to it), the
//! last by the body's own -- docx2svg's `_section` and `_sections`. A stated 0 is 0 (a
//! header at `w:header="0"` is at the page's edge). Names in exactly their namespace
//! (`Ns::W`), measures in twips or universal measures (`crate::w`).
//!
//! Must not: lay out pages.

use crate::model::{Block, LineNumbering, NoteNumbering, PageMargins, PageSize, Section, SectionProps};
use crate::value::{text, Text};
use crate::w;
use crate::xmlread::{attr_on_in, Ns};
use ooxml_xml::Element;
use std::sync::Arc;

fn page_size(sect: Option<w::SectPr<'_>>) -> PageSize {
    let size = sect.and_then(w::SectPr::page_size);
    PageSize {
        width: size.and_then(w::PgSz::w).filter(|v| *v != 0).unwrap_or(12240),
        height: size.and_then(w::PgSz::h).filter(|v| *v != 0).unwrap_or(15840),
        orientation: size.and_then(w::PgSz::orient).map(text),
    }
}

fn margins(sect: Option<w::SectPr<'_>>) -> PageMargins {
    let Some(m) = sect.and_then(w::SectPr::margins) else {
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

fn columns(sect: Option<w::SectPr<'_>>) -> Columns {
    let Some(cols) = sect.and_then(w::SectPr::columns) else {
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
#[must_use]
pub fn section_props(element: &Element) -> SectionProps {
    props(Some(w::SectPr::new(element)))
}

/// The properties of a story's one section where it states none: the defaults.
#[must_use]
pub fn unstated() -> SectionProps {
    props(None)
}

fn props(sect: Option<w::SectPr<'_>>) -> SectionProps {
    let cols = columns(sect);
    let headers: Vec<(Text, Text)> = sect
        .into_iter()
        .flat_map(w::SectPr::headers)
        .filter_map(reference)
        .collect();
    let footers: Vec<(Text, Text)> = sect
        .into_iter()
        .flat_map(w::SectPr::footers)
        .filter_map(reference)
        .collect();
    let pg = sect.and_then(w::SectPr::page_numbers);
    let val = |v: Option<w::Val<'_>>| v.and_then(w::Val::val).map(text);
    SectionProps {
        page_size: page_size(sect),
        margins: margins(sect),
        start_type: val(sect.and_then(w::SectPr::kind)),
        column_count: cols.count,
        column_space: cols.space,
        columns: cols.widths,
        column_separator: cols.separator,
        headers: Arc::from(headers),
        footers: Arc::from(footers),
        title_page: sect
            .and_then(w::SectPr::title_page)
            .is_some_and(|t| attr_on_in(t.element(), Some(Ns::W), "val")),
        page_number_start: pg.and_then(w::PgNumType::start),
        page_number_format: pg.and_then(w::PgNumType::fmt).map(text),
        vertical_alignment: val(sect.and_then(w::SectPr::vertical_alignment)),
        endnotes: notes(sect.and_then(w::SectPr::endnotes)),
        footnotes: notes(sect.and_then(w::SectPr::footnotes)),
        line_numbering: line_numbering(sect.and_then(w::SectPr::line_numbers)),
    }
}

fn reference(r: w::StoryReference<'_>) -> Option<(Text, Text)> {
    let id = r.id().filter(|i| !i.is_empty())?;
    Some((text(r.kind().filter(|k| !k.is_empty()).unwrap_or("default")), text(id)))
}

/// The sections of a story's top-level paragraphs, the last ended by `last` (the body's
/// own `w:sectPr`; where neither it nor a paragraph states one, the story is one section
/// with the defaults, [`unstated`]).
#[must_use]
pub fn story_sections(body: &[Block], last: Option<&Element>) -> Vec<Section> {
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
    let last = match last {
        Some(last) => Some(section_props(last)),
        None => out.is_empty().then(unstated),
    };
    if let Some(props) = last {
        out.push(Section {
            properties: Arc::new(props),
            first_paragraph: first,
            last_paragraph: count,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    //! A `w:sectPr` read alike in transitional and strict WordprocessingML, its measures in
    //! twips or as universal measures (cut toward zero, as Word 16.106 reads them:
    //! `crates/ooxml-oracle/word/observations/strict-word.json`).
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::{section_props, unstated};
    use ooxml_base::XmlLimits;

    const TRANSITIONAL: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    const STRICT: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";

    fn read(namespace: &str, body: &str) -> crate::model::SectionProps {
        let xml = format!(r#"<w:sectPr xmlns:w="{namespace}">{body}</w:sectPr>"#);
        let tree = ooxml_xml::parse(xml.as_bytes(), &XmlLimits::default()).unwrap_or_else(|e| panic!("{e}"));
        section_props(tree.root())
    }

    #[test]
    fn universal_measures_read_as_twips_in_either_namespace() {
        let twips = r#"<w:pgSz w:w="12240" w:h="15840" w:orient="portrait"/>
            <w:pgMar w:top="1440" w:right="720" w:bottom="569" w:left="1889" w:header="708" w:footer="246" w:gutter="0"/>
            <w:cols w:num="2" w:space="360" w:equalWidth="0"><w:col w:w="4320" w:space="360"/><w:col w:w="1440"/></w:cols>
            <w:titlePg w:val="1"/>"#;
        let measures = r#"<w:pgSz w:w="8.5in" w:h="11in" w:orient="portrait"/>
            <w:pgMar w:top="2.54cm" w:right="36pt" w:bottom="10.05mm" w:left="3.333cm" w:header="35.4pt" w:footer="12.33pt" w:gutter="0pt"/>
            <w:cols w:num="2" w:space="1.5pc" w:equalWidth="0"><w:col w:w="3in" w:space="0.25in"/><w:col w:w="6pc"/></w:cols>
            <w:titlePg w:val="1"/>"#;
        let expected = read(TRANSITIONAL, twips);
        assert_eq!(expected.page_size.width, 12240);
        assert_eq!(expected.margins.bottom, 569);
        assert_eq!(&*expected.columns, &[(4320, 360), (1440, 0)]);
        assert!(expected.title_page);
        for namespace in [TRANSITIONAL, STRICT] {
            assert_eq!(read(namespace, twips), expected, "{namespace}");
            assert_eq!(read(namespace, measures), expected, "{namespace}");
        }
    }

    #[test]
    fn a_name_in_another_namespace_is_not_read() {
        let foreign = r#"<x:pgSz xmlns:x="urn:other" x:w="1000" x:h="1000"/><x:titlePg xmlns:x="urn:other"/>"#;
        assert_eq!(read(TRANSITIONAL, foreign), unstated());
        assert_eq!(read(STRICT, foreign), unstated());
    }
}
