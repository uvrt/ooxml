//! Owns: a table read in the view (docx-agent's `Reader._table_blocks`): its rows in the
//! view (a row deleted as a tracked change gone from the final view, one inserted from the
//! original), header rows, cells placed by `w:gridBefore` and `w:gridSpan`, vertical merges
//! (and a pending tracked one, as accepting leaves it), cells the view leaves out, each cell's
//! blocks; the paragraphs whose marks are deleted just before the table joined into its first
//! cell's first paragraph.
//!
//! Not yet ported (listed in the lane's hand-back): Accept All's cell ends (a cell whose
//! last mark is deleted taking the next cell in), and a deleted cell whose content is kept
//! widening the cell before it.
//!
//! Must not: render.

use super::{child_at, index, CellRecord, Item2, Placed, Reader, Record, TableRecord};
use crate::docx::find_path;
use crate::docx::text::View;
use crate::ids::W;
use crate::xmlread::py_int;
use ooxml_xml::{Element, Node};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

fn int_val(node: Option<&Element>, default: i64) -> i64 {
    node.and_then(|n| n.attribute(Some(W), "val"))
        .and_then(py_int)
        .unwrap_or(default)
}

/// A row's `w:tc`, also inside row-level `w:sdt` and `w:customXml`.
fn cells(row: &Placed) -> Vec<Placed> {
    let mut out = Vec::new();
    for (k, node) in row.element.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        let path = [&row.path[..], &[index(k)]].concat().into_boxed_slice();
        if child.name().is(Some(W), "tc") {
            out.push(Placed {
                element: Arc::clone(child),
                path,
            });
        } else if child.name().is(Some(W), "sdt") {
            if let Some((j, content)) = child_at(child, "sdtContent") {
                out.extend(cells(&Placed {
                    element: content,
                    path: [&path[..], &[j]].concat().into_boxed_slice(),
                }));
            }
        } else if child.name().is(Some(W), "customXml") {
            out.extend(cells(&Placed {
                element: Arc::clone(child),
                path,
            }));
        }
    }
    out
}

impl Reader<'_> {
    /// The records a `w:tbl` is in the view: its table, with the paragraphs whose marks are
    /// deleted just before it joined into its first cell (or before it, where it cannot).
    pub(super) fn table_with(&mut self, part: &str, table: &Placed, _outer: Option<&str>) -> Vec<Record> {
        let prefix = self.into.remove(&(part.to_owned(), table.path.clone()));
        let (mut records, placed) = self.table_blocks(part, table, prefix.clone());
        if let Some(prefix) = prefix {
            if !placed
                && prefix
                    .iter()
                    .any(|p| !crate::docx::text::paragraph_text(&p.element, self.view).is_empty())
            {
                let style = table_style(&table.element);
                let record = self.paragraph(part, prefix, Some(&style));
                records.insert(0, Record::Paragraph(Box::new(record)));
            }
        }
        records
    }

    fn cell_hidden(&self, cell: &Element) -> bool {
        let tag = match self.view {
            View::Markup => return false,
            View::Current => "cellDel",
            View::Original => "cellIns",
        };
        find_path(cell, &["tcPr", tag]).is_some()
    }

    fn cell_records(
        &mut self,
        part: &str,
        cell: &Placed,
        prefix: Option<Vec<Placed>>,
        style: &str,
    ) -> (Vec<Record>, bool) {
        let items = self.items(part, cell);
        let mut groups = self.groups(part, items);
        let mut placed = false;
        if let Some(prefix) = prefix {
            let first = groups.iter().position(|g| matches!(g, Item2::P(_) | Item2::Tbl(_)));
            match first.and_then(|k| groups.get_mut(k)) {
                Some(Item2::P(elements)) => {
                    let mut joined = prefix;
                    joined.append(elements);
                    *elements = joined;
                }
                Some(Item2::Tbl(t)) => {
                    self.into.insert((part.to_owned(), t.path.clone()), prefix);
                }
                _ => groups.insert(0, Item2::P(prefix)),
            }
            placed = true;
        }
        let mut out = Vec::new();
        for group in groups {
            match group {
                Item2::P(elements) => {
                    out.push(Record::Paragraph(Box::new(self.paragraph(part, elements, Some(style)))))
                }
                Item2::Tbl(t) => out.extend(self.table_with(part, &t, Some(style))),
                Item2::Marker(m) => out.push(Record::Marker(m)),
            }
        }
        (out, placed)
    }

    fn table_blocks(&mut self, part: &str, table: &Placed, prefix: Option<Vec<Placed>>) -> (Vec<Record>, bool) {
        let Some(story) = self.docx.part(part) else {
            return (Vec::new(), false);
        };
        let Some(entry) = story.ids.table_at(&table.path).cloned() else {
            return (Vec::new(), false);
        };
        let root = Arc::clone(story.root());
        let style = table_style(&table.element);
        let style_name = match self.docx.style(&style) {
            _ if style.is_empty() => None,
            Some(found) => crate::docx::style_name(found).map(str::to_owned),
            None => Some(style.clone()),
        };
        let columns = find_path(&table.element, &["tblGrid"]).map_or(0, |g| {
            g.child_elements().filter(|c| c.name().is(Some(W), "gridCol")).count()
        });
        let mut grid = Grid {
            columns,
            prefix,
            counting_header: true,
            ..Grid::default()
        };
        for (r, (row_path, _)) in entry.rows.iter().enumerate() {
            let Some(row) = crate::docx::at_path(&root, row_path) else {
                continue;
            };
            let row = Placed {
                element: Arc::clone(row),
                path: row_path.clone(),
            };
            self.table_row(part, &entry.id, &style, &mut grid, r, &row);
        }
        if grid.rows.is_empty() {
            return (Vec::new(), grid.placed);
        }
        let dropped: HashSet<usize> = grid.hidden.iter().flatten().copied().collect();
        let mut width = grid.columns;
        if !dropped.is_empty() {
            let shown: HashSet<usize> = grid
                .rows
                .iter()
                .flatten()
                .flat_map(|c| c.column..c.column + c.column_span)
                .collect();
            width -= (0..grid.columns)
                .filter(|c| !shown.contains(c) && dropped.contains(c))
                .count();
        }
        let record = TableRecord {
            volatile: entry.id.starts_with("t@"),
            id: entry.id,
            element: Arc::clone(&table.element),
            style_name,
            columns: width,
            rows: grid.rows,
            header_rows: grid.header_rows,
            table_style: style,
        };
        (vec![Record::Table(Box::new(record))], grid.placed)
    }

    /// One row of a table in the view, into `grid`.
    fn table_row(&mut self, part: &str, table_id: &str, style: &str, grid: &mut Grid, r: usize, row: &Placed) {
        let properties = row.element.find(Some(W), "trPr");
        let has = |local: &str| properties.is_some_and(|p| p.find(Some(W), local).is_some());
        if (self.view == View::Current && has("del")) || (self.view == View::Original && has("ins")) {
            return;
        }
        let header = properties
            .and_then(|p| p.find(Some(W), "tblHeader"))
            .is_some_and(|h| !matches!(h.attribute(Some(W), "val"), Some("0" | "false" | "off")));
        if grid.counting_header && header {
            grid.header_rows += 1;
        } else {
            grid.counting_header = false;
        }
        let mut line: Vec<CellRecord> = Vec::new();
        let mut gone_here = HashSet::new();
        let mut column = usize::try_from(int_val(find_path(&row.element, &["trPr", "gridBefore"]), 0)).unwrap_or(0);
        let all = cells(row);
        for tc in &all {
            let span = usize::try_from(int_val(find_path(&tc.element, &["tcPr", "gridSpan"]), 1).max(1)).unwrap_or(1);
            if self.cell_hidden(&tc.element) {
                gone_here.extend(column..column + span);
            } else if let (true, Some(origin)) = (self.continues(&tc.element), grid.origins.get(&column).copied()) {
                if let Some(cell) = grid.rows.get_mut(origin.0).and_then(|l| l.get_mut(origin.1)) {
                    cell.row_span += 1;
                }
            } else {
                let first = if grid.rows.is_empty() && line.is_empty() {
                    grid.prefix.take()
                } else {
                    None
                };
                let (records, placed) = self.cell_records(part, tc, first, style);
                grid.placed |= placed;
                for c in column..column + span {
                    grid.origins.insert(c, (grid.rows.len(), line.len()));
                }
                let id = format!("{table_id}/c{r},{column}");
                line.push(CellRecord {
                    id,
                    row: r,
                    column,
                    row_span: 1,
                    column_span: span,
                    records,
                });
            }
            column += span;
        }
        let after = usize::try_from(int_val(find_path(&row.element, &["trPr", "gridAfter"]), 0)).unwrap_or(0);
        grid.columns = grid.columns.max(column + after);
        if !line.is_empty() || all.is_empty() {
            grid.rows.push(line);
            grid.hidden.push(gone_here);
        }
    }

    /// Whether a cell continues a vertical merge (a pending tracked one as accepting leaves it).
    fn continues(&self, tc: &Element) -> bool {
        if self.view == View::Current {
            if let Some(tracked) = find_path(tc, &["tcPr", "cellMerge"]) {
                return tracked.attribute(Some(W), "vMerge") == Some("cont");
            }
        }
        find_path(tc, &["tcPr", "vMerge"])
            .is_some_and(|m| matches!(m.attribute(Some(W), "val"), None | Some("continue")))
    }
}

/// A table's rows as they are read.
#[derive(Default)]
struct Grid {
    columns: usize,
    origins: HashMap<usize, (usize, usize)>,
    rows: Vec<Vec<CellRecord>>,
    hidden: Vec<HashSet<usize>>,
    header_rows: usize,
    counting_header: bool,
    placed: bool,
    prefix: Option<Vec<Placed>>,
}

/// A table's `w:tblStyle` (`""` where it names none).
pub(super) fn table_style(table: &Element) -> String {
    find_path(table, &["tblPr", "tblStyle"])
        .and_then(|s| s.attribute(Some(W), "val"))
        .unwrap_or("")
        .to_owned()
}
