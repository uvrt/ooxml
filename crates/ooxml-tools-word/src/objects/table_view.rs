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
use ooxml_word_edit::facts::{row_off_grid, tracked_structure};
use ooxml_word_edit::WordDoc;
use ooxml_word_model::address::{resolve, AddressError};
use ooxml_word_model::docx::tables::{grid_columns, Accepted, Grid, GridPlaces, Misplaced, RowSpec, TableGrid};
use ooxml_word_model::docx::{find_path, style_name, Docx};
use ooxml_word_model::ids::{Entry, TableId, W};
use ooxml_xml::{Element, Node};

/// A table found on a version.
pub(crate) struct TableView {
    /// Its part.
    pub(crate) part: String,
    /// The table.
    pub(crate) table: TableId,
    /// Its rows on the grid ([`Grid`], by the one value rule): each cell's first grid column
    /// and span, each row against `w:tblGrid`; a row not found has no cells.
    shape: Grid,
    /// Whether each row was found.
    found: Vec<bool>,
    /// Per row, per grid position: the origin `(row, column)` of the cell there, as runs.
    pub(crate) grid: GridPlaces,
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
            let (shape, found) = shape(&table);
            Ok(TableView {
                part: story.part.clone(),
                grid: GridPlaces::of(&shape),
                shape,
                found,
                table,
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

/// The table's rows on its grid ([`Grid::build`], `w:tblGrid` its columns), and whether
/// each row was found (one not found placed with no cells).
fn shape(table: &TableId) -> (Grid, Vec<bool>) {
    let rows: Vec<Option<&Element>> = table
        .rows
        .iter()
        .map(|(path, _)| {
            ooxml_word_model::docx::at_path(&table.element, path.get(table.path.len()..).unwrap_or_default())
                .map(|row| &**row)
        })
        .collect();
    let specs: Vec<RowSpec> = rows
        .iter()
        .map(|row| row.map_or_else(|| RowSpec::new(None, None, Vec::new()), RowSpec::of))
        .collect();
    let grid = Grid::build(&specs, grid_columns(&table.element));
    (grid, rows.iter().map(Option::is_some).collect())
}

impl TableView {
    /// The key the table is read and written under (its block's): a plan worked out from its
    /// rows and cells reads it ([`crate::planned::Plan::reads`]).
    pub(crate) fn key(&self) -> ooxml_word_edit::Key {
        ooxml_word_edit::Key::block(self.part.as_str(), self.table.id.as_str())
    }

    /// The grid's column count (`w:gridCol` in `w:tblGrid`).
    pub(crate) fn column_count(&self) -> usize {
        self.shape.columns
    }

    /// The cell origin at `(row, column)`.
    pub(crate) fn origin(&self, row: usize, column: usize) -> Option<(usize, usize)> {
        self.grid.get(row, column)
    }

    /// Whether a row is off the grid as check's `table_grid_mismatch` says it
    /// ([`row_off_grid`], `_off_grid`): past it, or -- in a table holding tracked structure
    /// records -- not reaching exactly across it with its written `w:gridAfter`.
    pub(crate) fn off_grid(&self) -> bool {
        let tracked = tracked_structure(&self.table.element);
        self.shape
            .rows
            .iter()
            .zip(&self.found)
            .any(|(line, found)| *found && row_off_grid(line, self.shape.columns, tracked).is_some())
    }

    /// Row `row`'s cells as (first grid column, span) (docx-agent's `_row_cells`); none
    /// past the last row.
    pub(crate) fn row_cells(&self, row: usize) -> Vec<(usize, usize)> {
        self.shape
            .rows
            .get(row)
            .map(|line| line.cells.iter().map(|c| (c.first, c.span)).collect())
            .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use ooxml_base::Limits;
    use ooxml_opc::Package;
    use ooxml_zip::{Method, NewEntry, Writer};
    use std::sync::Arc;

    const NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";

    /// The view of a table on a grid of two columns whose rows are `rows` (each a `w:tr`'s
    /// content).
    fn view(rows: &[&str]) -> TableView {
        let rows: String = rows.iter().map(|r| format!("<w:tr>{r}</w:tr>")).collect();
        let body = format!(
            r#"<w:document xmlns:w="{NS}"><w:body><w:tbl><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>{rows}</w:tbl><w:p/></w:body></w:document>"#
        );
        let parts = [
            ("[Content_Types].xml", r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_owned()),
            ("_rels/.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}" Target="word/document.xml"/></Relationships>"#)),
            ("word/document.xml", body),
        ];
        let mut zip = Writer::new();
        for (name, data) in parts {
            let entry = NewEntry {
                name: name.to_owned(),
                method: Method::Deflated,
                dos_time: 0,
                dos_date: 33,
                external_attr: 0,
                create_system: 3,
            };
            zip.add(&entry, data.as_bytes()).unwrap();
        }
        let package = Package::open(Arc::from(zip.finish(b"").unwrap()), &Limits::default()).unwrap();
        let docx = Docx::open(&ooxml_word_model::WordPackage::open(package).0).unwrap();
        let story = docx.body().unwrap();
        let table = story.ids.tables().get(0).unwrap();
        let (shape, found) = shape(&table);
        TableView {
            part: story.part.clone(),
            grid: GridPlaces::of(&shape),
            shape,
            found,
            table,
        }
    }

    fn cell(span: Option<&str>) -> String {
        let span = span.map_or_else(String::new, |s| {
            format!(r#"<w:tcPr><w:gridSpan w:val="{s}"/></w:tcPr>"#)
        });
        format!("<w:tc>{span}<w:p/></w:tc>")
    }

    const PLAIN: &str = "<w:tc><w:p/></w:tc><w:tc><w:p/></w:tc>";

    #[test]
    fn a_span_is_read_by_the_one_rule_as_word_reads_it() {
        // (value, span as read now, span as the view read it before: `trim`, `parse::<i64>`)
        for (raw, span, _before) in [
            ("2", 2, 2),
            ("+2", 2, 2),
            ("2.0", 2, 1),
            ("2.5", 2, 1),
            (" 2", 1, 2),
            ("99999999999999999999", 1_661_992_959, 1),
            ("4294967298", 2, 4_294_967_298_usize),
        ] {
            let view = view(&[&cell(Some(raw)), PLAIN]);
            assert_eq!(view.row_cells(0), [(0, span)], "{raw:?}");
            assert_eq!(view.origin(0, span - 1), Some((0, 0)), "{raw:?}");
            assert_eq!(view.origin(0, span), None, "{raw:?}");
            // off its grid only past it: a short row's trailing columns Word fills
            assert_eq!(view.off_grid(), span > 2, "{raw:?}");
        }
    }

    #[test]
    fn grid_before_and_after_are_read_by_the_one_rule() {
        // `1.0` was 0 before: the row read one column short, off its grid
        let before = view(&[
            r#"<w:trPr><w:gridBefore w:val="1.0"/></w:trPr><w:tc><w:p/></w:tc>"#,
            PLAIN,
        ]);
        assert_eq!(before.row_cells(0), [(1, 1)]);
        assert_eq!((before.origin(0, 0), before.origin(0, 1)), (None, Some((0, 1))));
        assert!(!before.off_grid());
        let after = view(&[
            r#"<w:trPr><w:gridAfter w:val="1.0"/></w:trPr><w:tc><w:p/></w:tc>"#,
            PLAIN,
        ]);
        assert!(!after.off_grid());
        // a short row is on its grid (Word fills its trailing column), one past it is not
        assert!(!view(&[r#"<w:tc><w:p/></w:tc>"#, PLAIN]).off_grid());
        assert!(view(&[&cell(Some("3")), PLAIN]).off_grid());
    }

    #[test]
    fn a_merged_cells_size_is_counted_by_runs() {
        let view = view(&[
            r#"<w:tc><w:tcPr><w:gridSpan w:val="2147483645"/><w:vMerge w:val="restart"/></w:tcPr><w:p/></w:tc>"#,
            r#"<w:tc><w:tcPr><w:gridSpan w:val="2147483645"/><w:vMerge/></w:tcPr><w:p/></w:tc>"#,
        ]);
        assert_eq!(view.grid.size_of((0, 0)), (2, 2_147_483_645));
        assert_eq!(view.origin(1, 2_147_483_644), Some((0, 0)));
    }
}
