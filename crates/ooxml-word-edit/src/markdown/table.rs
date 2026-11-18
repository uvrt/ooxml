//! Owns: a GFM table as insert_markdown writes it (docx-agent's `Writer.table`,
//! `cell_look`, `text_width`, `table_style_in_use`, `Styles.table_default`): in the
//! document's table style, `w:tblW` auto, `tblLook` 04A0 (0480 without a header row), the
//! section's text width shared among the columns, the header row repeated, a column's
//! alignment its cells' style where the map names one, else their direct `w:jc` (said once).
//!
//! Must not: write but through the writer and the sink.

use super::writer::Writer;
use crate::names::W;
use crate::review::xml::make;
use crate::sink::EditSink;
use crate::work::{Id, Work};
use crate::Result;
use ooxml_word_model::docx::{child_val, find_path, style_id, style_kind, Docx};
use ooxml_word_model::markdown::model::{Inline, Table};
use ooxml_xml::Element;

/// The text width a table shares when the section says none (A4, one-inch margins).
const DEFAULT_TEXT_WIDTH: i64 = 9026;
/// The settings part's relationship type.
const REL_SETTINGS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings";

impl Writer<'_> {
    /// `table` as a `w:tbl`.
    pub(crate) fn table(&mut self, sink: &mut EditSink<'_, '_>, work: &mut Work, table: &Table) -> Result<Id> {
        let rows: Vec<&Vec<Vec<Inline>>> = if table.headed {
            std::iter::once(&table.header).chain(&table.rows).collect()
        } else {
            table.rows.iter().collect()
        };
        let columns = rows.iter().map(|r| r.len()).max().unwrap_or(0).max(1);
        let style = self.style(sink, "table", 0)?;
        let width = self.text_width(sink)?;
        let count = i64::try_from(columns).unwrap_or(1);
        let shares: Vec<i64> = (0..count)
            .map(|k| width.div_euclid(count) + i64::from(k >= count - width.rem_euclid(count)))
            .collect();
        let node = make(work, "tbl", &[]);
        let properties = make(work, "tblPr", &[]);
        work.append(node, properties);
        if let Some(style) = &style {
            let s = make(work, "tblStyle", &[("val", style)]);
            work.append(properties, s);
        }
        let w = make(work, "tblW", &[("w", "0"), ("type", "auto")]);
        work.append(properties, w);
        let (look, first) = if table.headed { ("04A0", "1") } else { ("0480", "0") };
        let look = make(
            work,
            "tblLook",
            &[
                ("val", look),
                ("firstRow", first),
                ("lastRow", "0"),
                ("firstColumn", "1"),
                ("lastColumn", "0"),
                ("noHBand", "0"),
                ("noVBand", "1"),
            ],
        );
        work.append(properties, look);
        let grid = make(work, "tblGrid", &[]);
        work.append(node, grid);
        for share in &shares {
            let col = make(work, "gridCol", &[("w", &share.to_string())]);
            work.append(grid, col);
        }
        let mut looks = Vec::new();
        for c in 0..columns {
            let align = table.aligns.get(c).cloned().flatten();
            looks.push(self.cell_look(sink, align.as_deref())?);
        }
        for (k, row) in rows.iter().enumerate() {
            let tr = make(work, "tr", &[]);
            work.append(node, tr);
            if k == 0 && table.headed {
                let tr_pr = make(work, "trPr", &[]);
                work.append(tr, tr_pr);
                let header = make(work, "tblHeader", &[]);
                work.append(tr_pr, header);
            }
            for (c, (style, align)) in looks.iter().enumerate() {
                let cell = make(work, "tc", &[]);
                work.append(tr, cell);
                let tc_pr = make(work, "tcPr", &[]);
                work.append(cell, tc_pr);
                let share = shares.get(c).copied().unwrap_or_default().to_string();
                let tc_w = make(work, "tcW", &[("w", &share), ("type", "dxa")]);
                work.append(tc_pr, tc_w);
                let inline = row.get(c).map_or(&[][..], Vec::as_slice);
                let paragraph = self.paragraph(sink, work, inline, style.as_deref(), align.as_deref())?;
                work.append(cell, paragraph);
            }
        }
        self.tables.push((node, shares));
        Ok(node)
    }

    /// A column's cells' paragraph style and direct `w:jc`.
    fn cell_look(&mut self, sink: &mut EditSink<'_, '_>, align: Option<&str>) -> Result<(Option<String>, Option<String>)> {
        let Some(align) = align.filter(|a| matches!(*a, "center" | "right")) else {
            return Ok((self.style(sink, "table_cell", 0)?, align.map(str::to_owned)));
        };
        let role = format!("table_cell_{align}");
        if self.map.style_for(&role, 0).is_some_and(|r| r.style.is_some()) {
            return Ok((self.style(sink, &role, 0)?, None));
        }
        self.warn_once(format!(
            "a column aligned {align} is written as direct alignment (w:jc) on its cells, which check reports as \
             direct_formatting; map {role} to a style aligned {align} to avoid it"
        ));
        Ok((self.style(sink, "table_cell", 0)?, Some(align.to_owned())))
    }

    /// The text width of the body's section, in twips.
    pub(crate) fn text_width(&mut self, sink: &EditSink<'_, '_>) -> Result<i64> {
        if let Some(width) = self.text_width {
            return Ok(width);
        }
        let docx = sink.docx()?;
        let section = docx
            .body()
            .and_then(|b| b.root().child_elements().find(|c| c.name().is(Some(W), "body")))
            .and_then(|body| body.child_elements().find(|c| c.name().is(Some(W), "sectPr")));
        let width = section.and_then(|s| section_width(s)).filter(|w| *w > 0);
        let width = width.unwrap_or(DEFAULT_TEXT_WIDTH);
        self.text_width = Some(width);
        Ok(width)
    }
}

/// `pgSz/@w` less the margins and the gutter, as Python's `int` reads them.
fn section_width(section: &Element) -> Option<i64> {
    let size = section.child_elements().find(|c| c.name().is(Some(W), "pgSz"))?;
    let margins = section.child_elements().find(|c| c.name().is(Some(W), "pgMar"))?;
    let int = |value: Option<&str>| -> Option<i64> { ooxml_word_model::xmlread::py_int(value?) };
    let or_zero = |local: &str| match margins.attribute(Some(W), local).filter(|v| !v.is_empty()) {
        Some(v) => int(Some(v)),
        None => Some(0),
    };
    Some(int(size.attribute(Some(W), "w"))? - or_zero("left")? - or_zero("right")? - or_zero("gutter")?)
}

/// The table style the settings name as new tables' (`w:defaultTableStyle`), when the
/// document has it as a table style.
///
/// # Errors
/// When the settings part is not XML.
pub(crate) fn table_default(sink: &EditSink<'_, '_>, docx: &Docx) -> Result<Option<String>> {
    let Some(part) = ooxml_word_model::parts::related(sink.doc().package(), &docx.main, REL_SETTINGS)? else {
        return Ok(None);
    };
    let Some(tree) = sink.doc().tree(&part)? else {
        return Ok(None);
    };
    let wanted = child_val(tree.root(), "defaultTableStyle").filter(|v| !v.is_empty());
    Ok(wanted.and_then(|id| {
        docx.styles()
            .iter()
            .find(|s| style_id(s) == id && style_kind(s) == "table")
            .map(|s| style_id(s).to_owned())
    }))
}

/// The table style the document's tables use most -- of those its styles declare as table
/// styles, ties in the order first used -- or none.
pub(crate) fn table_style_in_use(docx: &Docx) -> Option<String> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    let mut parts = vec![docx.main.clone()];
    parts.extend(ooxml_word_model::parts::story_parts(&docx.package, &docx.main).unwrap_or_default());
    for part in parts {
        let Some(story) = docx.part(&part) else { continue };
        let mut stack = vec![story.root()];
        while let Some(element) = stack.pop() {
            if element.name().is(Some(W), "tblStyle") {
                if let Some(value) = element.attribute(Some(W), "val").filter(|v| !v.is_empty()) {
                    match counts.iter_mut().find(|(k, _)| k == value) {
                        Some(slot) => slot.1 += 1,
                        None => counts.push((value.to_owned(), 1)),
                    }
                }
            }
            let children: Vec<_> = element.child_elements().collect();
            stack.extend(children.into_iter().rev());
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1));
    counts.into_iter().map(|(id, _)| id).find(|id| {
        docx.styles()
            .iter()
            .any(|s| style_id(s) == id && style_kind(s) == "table")
    })
}

/// The style's `w:pPr/w:ind` through its `basedOn` chain.
pub(crate) fn style_indent(docx: &Docx, style: Option<&str>) -> Option<Element> {
    let mut current = style.map(str::to_owned);
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = current.take() {
        if !seen.insert(id.clone()) {
            break;
        }
        let node = docx.style(&id)?;
        if let Some(found) = find_path(node, &["pPr", "ind"]) {
            return Some(found.clone());
        }
        current = child_val(node, "basedOn").map(str::to_owned);
    }
    None
}
