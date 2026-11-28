//! Owns: a table read in the view (docx-agent's `Reader._table_blocks`): its rows in the
//! view (a row deleted as a tracked change gone from the final view, one inserted from the
//! original), header rows, cells placed by `w:gridBefore` and `w:gridSpan`, vertical merges
//! (and a pending tracked one, as accepting leaves it), cells the view leaves out, the grid
//! as Word rebuilds it from the cells left,
//! each cell's blocks; the paragraphs whose marks are deleted just before the table joined into its first
//! cell's first paragraph.
//!
//! A row whose deletion is pending goes whole in the final view, and a merge whose first
//! row goes starts at its first kept cell (Word's Accept All, round 60 R137).
//!
//! Each cell is where the view's grid has it ([`ViewGrid`]: a removed cell merged into the
//! one beside it or gone with the cells after it moving left, each kept cell's span in the
//! view's columns, a vertical merge the view breaks gone). A cell Accept All takes into the
//! one before it adds its blocks after that cell's (its first paragraph not joined to the
//! last one: not yet ported, as before).
//!
//! Must not: render.

use super::{CellRecord, Item2, Placed, Reader, Record, TableRecord};
use crate::docx::find_path;
use crate::docx::tables::{grid_widths, merge_heads, row_cells, RowSpec, VMerge};
use crate::ids::W;
use crate::view::{View, ViewGrid};
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::Arc;

/// A row's `w:tc`, also inside row-level `w:sdt` and `w:customXml` (the grid's walk,
/// [`row_cells`]).
fn cells(row: &Placed) -> Vec<Placed> {
    row_cells(&row.element)
        .into_iter()
        .map(|(element, place)| Placed {
            element,
            path: [&row.path[..], &place[..]].concat().into_boxed_slice(),
        })
        .collect()
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
            } else if let Some(Record::Table(record)) = records.first_mut() {
                // Joined into the table (or, with no paragraph to take them, gone with
                // nothing to show): the table holds them in the view, as a paragraph holds
                // those joined into it.
                record.joins = self.ids_of(part, &prefix);
            }
        }
        records
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
        let Some(entry) = story.ids.table_at(&table.path) else {
            return (Vec::new(), false);
        };
        let root = Arc::clone(story.root());
        let style = table_style(&table.element);
        let style_name = match self.docx.style(&style) {
            _ if style.is_empty() => None,
            Some(found) => crate::docx::style_name(found).map(str::to_owned),
            None => Some(style.clone()),
        };
        let mut grid = Grid {
            prefix,
            counting_header: true,
            heads: self.merge_heads(&root, &entry.rows),
            ..Grid::default()
        };
        let rows: Vec<Arc<Element>> = entry
            .rows
            .iter()
            .filter_map(|(path, _)| crate::docx::at_path(&root, path).cloned())
            .collect();
        let refs: Vec<&Element> = rows.iter().map(|r| &**r).collect();
        let layout = ViewGrid::of(self.view, &refs, &grid_widths(&table.element));
        for (r, (row_path, _)) in entry.rows.iter().enumerate() {
            let Some(row) = crate::docx::at_path(&root, row_path) else {
                continue;
            };
            let row = Placed {
                element: Arc::clone(row),
                path: row_path.clone(),
            };
            self.table_row(part, &entry.id, &style, (&mut grid, &layout), r, &row);
        }
        if grid.rows.is_empty() {
            return (Vec::new(), grid.placed);
        }
        let width = layout.width();
        let record = TableRecord {
            id: entry.id,
            element: Arc::clone(&table.element),
            style_name,
            columns: width,
            rows: grid.rows,
            header_rows: grid.header_rows,
            table_style: style,
            part: part.to_owned(),
            path: table.path.clone(),
            joins: Vec::new(),
        };
        (vec![Record::Table(Box::new(record))], grid.placed)
    }

    /// One row of a table in the view, into `grid`: each cell where the view's grid
    /// (`layout`) has it, spanning its columns there.
    fn table_row(
        &mut self,
        part: &str,
        table_id: &str,
        style: &str,
        (grid, layout): (&mut Grid, &ViewGrid),
        r: usize,
        row: &Placed,
    ) {
        let Some(view_row) = layout.row(r) else {
            return;
        };
        let properties = row.element.find(Some(W), "trPr");
        let header = properties
            .and_then(|p| p.find(Some(W), "tblHeader"))
            .is_some_and(|h| !matches!(h.attribute(Some(W), "val"), Some("0" | "false" | "off")));
        if grid.counting_header && header {
            grid.header_rows += 1;
        } else {
            grid.counting_header = false;
        }
        let mut line: Vec<CellRecord> = Vec::new();
        let spec = RowSpec::of(&row.element);
        // ids and addresses keep the document's grid (`column`); the view's grid places it
        let mut column = spec.before;
        let all = cells(row);
        for (n, tc) in all.iter().enumerate() {
            let span = spec.cells.get(n).map_or(1, |c| c.span);
            let at = column;
            column += span;
            let Some(place) = view_row.cells.iter().position(|c| c.cell == n) else {
                // removed (merged into a cell beside it, its text not kept, or gone), taken
                // into the cell before it, or gone with its end
                if !self.view.removes_cell(&tc.element) {
                    self.taken_into(part, style, &mut line, view_row.holding(n).map(|c| c.cell), tc);
                }
                continue;
            };
            let Some(here) = view_row.cells.get(place).map(|c| c.columns.clone()) else {
                continue;
            };
            let continued =
                self.continues(&tc.element) && !grid.heads.contains_key(&(r, n)) && !layout.merge_broken(r, place);
            if let (true, Some(origin)) = (continued, grid.origins.get(&here.start).copied()) {
                if let Some(cell) = grid.rows.get_mut(origin.0).and_then(|l| l.get_mut(origin.1)) {
                    cell.row_span += 1;
                }
                continue;
            }
            let first = if grid.rows.is_empty() && line.is_empty() {
                grid.prefix.take()
            } else {
                None
            };
            let (records, placed) = self.cell_records(part, tc, first, style);
            grid.placed |= placed;
            for c in here.clone() {
                grid.origins.insert(c, (grid.rows.len(), line.len()));
            }
            line.push(CellRecord {
                id: format!("{table_id}/c{r},{at}"),
                row: r,
                column: at,
                row_span: 1,
                column_span: span,
                view_span: here.len(),
                records,
                path: tc.path.clone(),
                cell: n,
            });
        }
        if !line.is_empty() || all.is_empty() {
            grid.rows.push(line);
        }
    }

    /// A cell the final view takes into the cell before it (its end removed): its blocks
    /// after that cell's, in `line` (`holder`, the row's cell taking it in, by place).
    fn taken_into(&mut self, part: &str, style: &str, line: &mut [CellRecord], holder: Option<usize>, tc: &Placed) {
        let Some(into) = holder.and_then(|h| line.iter_mut().find(|c| c.cell == h)) else {
            return;
        };
        let (records, _) = self.cell_records(part, tc, None, style);
        into.records.extend(records);
    }

    /// In the final view, the cells continuing a merge whose rows above them go that start
    /// it or stand alone ([`crate::docx::tables::merge_heads`]).
    fn merge_heads(&self, root: &Arc<Element>, rows: &[(Box<[u32]>, String)]) -> HashMap<(usize, usize), VMerge> {
        if self.view != View::Final {
            return HashMap::new();
        }
        // the rows by their place in the table, as the reader counts them
        let Some(elements) = rows
            .iter()
            .map(|(path, _)| crate::docx::at_path(root, path).cloned())
            .collect::<Option<Vec<Arc<Element>>>>()
        else {
            return HashMap::new();
        };
        let refs: Vec<&Element> = elements.iter().map(|e| &**e).collect();
        let kept = |row: &Element| !View::Final.removes_row(row);
        merge_heads(&refs, &kept, &|cell| View::Final.merge(cell))
    }

    /// Whether a cell continues a vertical merge (a pending tracked one as accepting leaves it).
    fn continues(&self, tc: &Element) -> bool {
        self.view.merge(tc) == VMerge::Continue
    }
}

/// A table's rows as they are read.
#[derive(Default)]
struct Grid {
    origins: HashMap<usize, (usize, usize)>,
    /// The cells continuing a merge the final view starts again or leaves alone.
    heads: HashMap<(usize, usize), VMerge>,
    rows: Vec<Vec<CellRecord>>,
    header_rows: usize,
    counting_header: bool,
    placed: bool,
    prefix: Option<Vec<Placed>>,
}

/// A table's `w:tblStyle` (`""` where it names none).
pub fn table_style(table: &Element) -> String {
    find_path(table, &["tblPr", "tblStyle"])
        .and_then(|s| s.attribute(Some(W), "val"))
        .unwrap_or("")
        .to_owned()
}
