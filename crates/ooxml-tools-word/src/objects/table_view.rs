//! Owns: a table as the object tools read it before and after an edit (docx-agent's
//! `Document.table`, `Table.grid`, `table_properties` and `tools/style.py`'s `looks_off`):
//! the table an id names, refused as `_of_kind` refuses another kind; each grid
//! position's cell origin (a merged cell answering at each position it covers); its
//! positions as accepting its tracked changes leaves it (docx-agent's `_Accepted`: the
//! one view `word_format_table`, `word_edit_table` and `set_cells` count positions by, its
//! refusals and notes); and the note where its style formats a region its look turns off.
//!
//! Must not: change the document.

use crate::refusal::address_refusal;
use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{ToolError, TryCall};
use ooxml_word_edit::WordDoc;
use ooxml_word_model::address::{resolve, AddressError};
use ooxml_word_model::docx::tables::{Accepted, Misplaced, TableGrid};
use ooxml_word_model::docx::{find_path, style_name, Docx};
use ooxml_word_model::ids::{Entry, TableId, W};
use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// A table found on a version.
pub(crate) struct TableView {
    /// Its part.
    pub(crate) part: String,
    /// The table.
    pub(crate) table: TableId,
    /// Per row, per grid position: the origin `(row, column)` of the cell there.
    pub(crate) grid: Vec<Vec<Option<(usize, usize)>>>,
}

/// The table `identifier` names on `doc` (docx-agent's `Document.table`): an id of another
/// kind refused naming its kind (`_of_kind`).
pub(crate) fn table_view(doc: &WordDoc, docx: &Docx, identifier: &str, call: &TryCall) -> Result<TableView, ToolError> {
    let followed = doc.ids().follow(identifier);
    let found = resolve(docx, &followed).or_else(|e| resolve(docx, identifier).map_err(|_| e));
    let (story, entry) = match found {
        Ok(found) => found,
        Err(error) if error.code == "invalid_arguments" => {
            let message = format!(
                "{}; block_text({}) gives the text of any block, span, cell or control",
                error.message,
                repr_str(identifier)
            );
            return Err(address_refusal(
                AddressError::invalid(message, error.options),
                "",
                call.clone(),
            ));
        }
        Err(error) => return Err(address_refusal(error, "", call.clone())),
    };
    let refuse = |message: String, options: Vec<String>| {
        address_refusal(AddressError::invalid(message, options), "", call.clone())
    };
    match entry {
        Entry::Table(k) => {
            let table = story
                .ids
                .tables()
                .get(k)
                .ok_or_else(|| refuse(format!("no table {identifier}"), vec![]))?;
            let grid = grid(&table);
            Ok(TableView {
                part: story.part.clone(),
                table,
                grid,
            })
        }
        Entry::Paragraph(k) => {
            let paragraph = story.ids.paragraphs().get(k);
            let holder = paragraph.as_ref().and_then(|p| {
                (1..p.path.len())
                    .rev()
                    .find_map(|n| story.ids.table_at(p.path.get(..n)?))
            });
            let quoted = repr_str(identifier);
            if let Some(table) = holder {
                return Err(refuse(
                    format!(
                        "{identifier} is a paragraph in table {}: give the table (block_text({quoted}) gives the \
                         paragraph's text)",
                        table.id
                    ),
                    vec![table.id],
                ));
            }
            let id = paragraph.map_or_else(|| identifier.to_owned(), |p| p.id);
            Err(refuse(
                format!(
                    "{identifier} is a paragraph, not a table: paragraph({quoted}) reads it, block_text({quoted}) \
                     gives its text"
                ),
                vec![id],
            ))
        }
    }
}

/// A row's `w:tc`, also inside row-level `w:sdt` and `w:customXml` (docx-agent's `_cells`).
fn cells(element: &Element, out: &mut Vec<Arc<Element>>) {
    for node in element.children().iter() {
        let Node::Element(child) = node else { continue };
        let name = child.name();
        if name.is(Some(W), "tc") {
            out.push(Arc::clone(child));
        } else if name.is(Some(W), "sdt") {
            if let Some(content) = child.find(Some(W), "sdtContent") {
                cells(content, out);
            }
        } else if name.is(Some(W), "customXml") {
            cells(child, out);
        }
    }
}

fn val(element: &Element, path: &[&str]) -> Option<i64> {
    find_path(element, path)?.attribute(Some(W), "val")?.trim().parse().ok()
}

/// Each grid position's cell origin (docx-agent's `Table.grid`).
fn grid(table: &TableId) -> Vec<Vec<Option<(usize, usize)>>> {
    let mut out = Vec::new();
    let mut origins: std::collections::HashMap<usize, (usize, usize)> = std::collections::HashMap::new();
    for (r, (path, _)) in table.rows.iter().enumerate() {
        let Some(row) =
            ooxml_word_model::docx::at_path(&table.element, path.get(table.path.len()..).unwrap_or_default())
        else {
            out.push(Vec::new());
            continue;
        };
        let before = usize::try_from(val(row, &["trPr", "gridBefore"]).unwrap_or(0)).unwrap_or(0);
        let mut line = vec![None; before];
        let mut column = before;
        let mut found = Vec::new();
        cells(row, &mut found);
        for cell in found {
            let span = usize::try_from(val(&cell, &["tcPr", "gridSpan"]).unwrap_or(1).max(1)).unwrap_or(1);
            let merge = find_path(&cell, &["tcPr", "vMerge"]);
            let continues = merge.is_some_and(|m| m.attribute(Some(W), "val").is_none_or(|v| v == "continue"));
            let origin = continues
                .then(|| origins.get(&column).copied())
                .flatten()
                .unwrap_or_else(|| {
                    for c in column..column + span {
                        origins.insert(c, (r, column));
                    }
                    (r, column)
                });
            line.extend(std::iter::repeat_n(Some(origin), span));
            column += span;
        }
        out.push(line);
    }
    out
}

impl TableView {
    /// The grid's column count (`w:gridCol` in `w:tblGrid`).
    pub(crate) fn column_count(&self) -> usize {
        self.table.element.find(Some(W), "tblGrid").map_or(0, |g| {
            g.children()
                .iter()
                .filter(|n| n.as_element().is_some_and(|e| e.name().is(Some(W), "gridCol")))
                .count()
        })
    }

    /// The cell origin at `(row, column)`.
    pub(crate) fn origin(&self, row: usize, column: usize) -> Option<(usize, usize)> {
        self.grid.get(row)?.get(column).copied().flatten()
    }

    /// Whether a row's cells, with `w:gridBefore` and `w:gridAfter`, do not reach exactly
    /// across the grid (`_off_grid`: check's `table_grid_mismatch`).
    pub(crate) fn off_grid(&self) -> bool {
        let count = self.column_count();
        self.table.rows.iter().any(|(path, _)| {
            let Some(row) = ooxml_word_model::docx::at_path(
                &self.table.element,
                path.get(self.table.path.len()..).unwrap_or_default(),
            ) else {
                return false;
            };
            let skips = val(row, &["trPr", "gridBefore"]).unwrap_or(0) + val(row, &["trPr", "gridAfter"]).unwrap_or(0);
            let mut found = Vec::new();
            cells(row, &mut found);
            let reach = skips
                + found
                    .iter()
                    .map(|c| val(c, &["tcPr", "gridSpan"]).unwrap_or(1).max(1))
                    .sum::<i64>();
            usize::try_from(reach).ok() != Some(count)
        })
    }

    /// Row `row`'s cells as (first grid column, span) (docx-agent's `_row_cells`); none
    /// past the last row.
    pub(crate) fn row_cells(&self, row: usize) -> Vec<(usize, usize)> {
        let Some(element) = self.table.rows.get(row).and_then(|(path, _)| {
            ooxml_word_model::docx::at_path(
                &self.table.element,
                path.get(self.table.path.len()..).unwrap_or_default(),
            )
        }) else {
            return Vec::new();
        };
        let mut column = usize::try_from(val(element, &["trPr", "gridBefore"]).unwrap_or(0)).unwrap_or(0);
        let mut found = Vec::new();
        cells(element, &mut found);
        found
            .iter()
            .map(|cell| {
                let span = usize::try_from(val(cell, &["tcPr", "gridSpan"]).unwrap_or(1).max(1)).unwrap_or(1);
                let placed = (column, span);
                column += span;
                placed
            })
            .collect()
    }

    /// The table as accepting its tracked changes leaves it (docx-agent's `_Accepted`), read
    /// on `docx`, the version it was found on.
    pub(crate) fn accepted(&self, docx: &Docx) -> AcceptedView {
        let table = docx.part(&self.part).map_or_else(
            || Accepted::plain(&self.table.id, self.column_count(), self.table.rows.len()),
            |part| TableGrid::new(part, &self.table).accepted(&self.table.id),
        );
        AcceptedView {
            table,
            notes: Vec::new(),
        }
    }
}

/// A table's positions as accepted, and what the call's positions stepped over or into,
/// said once each (docx-agent's `_Accepted` and its `notes`, rule 2). Cell ids stay on the
/// document's grid, rows and columns.
pub(crate) struct AcceptedView {
    /// The table as accepted.
    pub(crate) table: Accepted,
    /// The notes, in the order made.
    pub(crate) notes: Vec<String>,
}

impl AcceptedView {
    fn refusal(misplaced: Misplaced, call: &TryCall) -> ToolError {
        address_refusal(misplaced.error, &misplaced.field, call.clone())
    }

    /// The document row of position `row` (`to_row`).
    pub(crate) fn document_row(&mut self, row: i64, field: &str, call: &TryCall) -> Result<usize, ToolError> {
        self.table
            .to_row(row, field, &mut self.notes)
            .map_err(|m| Self::refusal(m, call))
    }

    /// The grid column of position `column` (`to_grid`).
    pub(crate) fn grid_column(&mut self, column: i64, field: &str, call: &TryCall) -> Result<usize, ToolError> {
        self.table
            .to_grid(column, field, &mut self.notes)
            .map_err(|m| Self::refusal(m, call))
    }

    /// Where document row `row`, grid column `grid` is written (`at`).
    pub(crate) fn at(&mut self, row: usize, grid: usize, call: &TryCall) -> Result<usize, ToolError> {
        self.table
            .at(row, grid, &mut self.notes)
            .map_err(|m| Self::refusal(m, call))
    }

    /// Position `(row, column)` as (document row, grid column) (`place`).
    pub(crate) fn place(
        &mut self,
        row: i64,
        column: i64,
        field: &str,
        call: &TryCall,
    ) -> Result<(usize, usize), ToolError> {
        self.table
            .place(row, column, field, &mut self.notes)
            .map_err(|m| Self::refusal(m, call))
    }

    /// The row a position names: the document's, as accepted where rows count otherwise.
    pub(crate) fn row(&mut self, row: i64, field: &str, call: &TryCall) -> Result<usize, ToolError> {
        if self.table.rows_active() {
            self.document_row(row, field, call)
        } else {
            Ok(usize::try_from(row).unwrap_or(usize::MAX))
        }
    }

    /// The grid column a position names: as accepted where columns count otherwise.
    pub(crate) fn column(&mut self, column: i64, field: &str, call: &TryCall) -> Result<usize, ToolError> {
        if self.table.active() {
            self.grid_column(column, field, call)
        } else {
            Ok(usize::try_from(column).unwrap_or(usize::MAX))
        }
    }
}

/// The note where the table's style formats a region its look turns off (round 58, R129:
/// `_regions_off`), on the version after the edit.
pub(crate) fn regions_off(docx: &Docx, table: &TableId) -> Option<String> {
    let properties = table.element.find(Some(W), "tblPr")?;
    let id = properties.find(Some(W), "tblStyle")?.attribute(Some(W), "val")?;
    let name = docx
        .styles()
        .iter()
        .find(|s| s.attribute(Some(W), "styleId") == Some(id))
        .map_or(id, |s| style_name(s).unwrap_or(id));
    let style = docx.find_style(name, "table")?;
    let wanted = needed_looks(style);
    let look = read_look(properties.find(Some(W), "tblLook").map(|l| &**l));
    let on: Vec<&str> = LOOK_NAMES
        .iter()
        .zip(look)
        .filter(|(_, on)| *on)
        .map(|(n, _)| *n)
        .collect();
    let off: Vec<&str> = wanted.iter().copied().filter(|n| !on.contains(n)).collect();
    if off.is_empty() {
        return None;
    }
    let fix: Vec<String> = on.iter().chain(off.iter()).map(|n| format!("\"{n}\"")).collect();
    Some(format!(
        "{}'s style {} formats {}, which its look turns off (not shown): look [{}] shows them",
        table.id,
        repr_str(name),
        off.join(", "),
        fix.join(", ")
    ))
}

/// The look options as the tool names them, in `LOOK_NAMES`' order.
pub(crate) const LOOK_NAMES: [&str; 6] = [
    "header_row",
    "first_column",
    "banded_rows",
    "total_row",
    "last_column",
    "banded_columns",
];
/// Each option's `w:tblLook` attribute, bit, and whether the attribute says "no".
const LOOK_BITS: [(&str, u32, bool); 6] = [
    ("firstRow", 0x0020, false),
    ("firstColumn", 0x0080, false),
    ("noHBand", 0x0200, true),
    ("lastRow", 0x0040, false),
    ("lastColumn", 0x0100, false),
    ("noVBand", 0x0400, true),
];

/// `w:tblLook` as the options it switches on, in `LOOK_NAMES`' order (`read_look`): the
/// default look (header row, first column, banded rows) where there is none.
pub(crate) fn read_look(node: Option<&Element>) -> [bool; 6] {
    let Some(node) = node else {
        return [true, true, true, false, false, false];
    };
    let bits = node
        .attribute(Some(W), "val")
        .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_hexdigit()))
        .and_then(|v| u32::from_str_radix(v, 16).ok());
    LOOK_BITS.map(|(attribute, bit, negated)| {
        let flag = match (node.attribute(Some(W), attribute), bits) {
            (Some(value), _) => matches!(value, "1" | "true" | "on"),
            (None, Some(bits)) => bits & bit != 0,
            (None, None) => false,
        };
        flag != negated
    })
}

/// The look options the style's regions need, in the order its regions are written
/// (`needed_looks` of `read_regions`).
fn needed_looks(style: &Element) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for node in style.children().iter() {
        let Node::Element(part) = node else { continue };
        if !part.name().is(Some(W), "tblStylePr") || !states(part) {
            continue;
        }
        let options: &[&'static str] = match part.attribute(Some(W), "type").unwrap_or("") {
            "firstRow" => &["header_row"],
            "lastRow" => &["total_row"],
            "firstCol" => &["first_column"],
            "lastCol" => &["last_column"],
            "band1Horz" | "band2Horz" => &["banded_rows"],
            "band1Vert" | "band2Vert" => &["banded_columns"],
            "nwCell" => &["header_row", "first_column"],
            "neCell" => &["header_row", "last_column"],
            "swCell" => &["total_row", "first_column"],
            "seCell" => &["total_row", "last_column"],
            _ => &[],
        };
        for option in options {
            if !out.contains(option) {
                out.push(option);
            }
        }
    }
    out
}

/// Whether a region states anything a region reports (`_read_part`): a run's colour,
/// bold, italic or size, a paragraph's alignment, a cell's shading, a border.
fn states(part: &Element) -> bool {
    let has = |path: &[&str]| find_path(part, path).is_some();
    ["color", "b", "i", "sz"].iter().any(|l| has(&["rPr", l]))
        || has(&["pPr", "jc"])
        || has(&["tcPr", "shd"])
        || find_path(part, &["tcPr", "tcBorders"])
            .is_some_and(|b| b.children().iter().any(|n| n.as_element().is_some()))
}
