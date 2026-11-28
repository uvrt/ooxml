//! Owns: a table read in the view (docx-agent's `Reader._table_blocks`): its rows in the
//! view (a row deleted as a tracked change gone from the final view, one inserted from the
//! original), header rows, cells placed by `w:gridBefore` and `w:gridSpan`, vertical merges
//! (and a pending tracked one, as accepting leaves it), cells the view leaves out, each cell's
//! blocks; the paragraphs whose marks are deleted just before the table joined into its first
//! cell's first paragraph.
//!
//! A row whose deletion is pending goes whole in the final view, and a merge whose first
//! row goes starts at its first kept cell (Word's Accept All, round 60 R137).
//!
//! Not yet ported (listed in the lane's hand-back): Accept All's cell ends (a cell whose
//! last mark is deleted taking the next cell in), and a deleted cell whose content is kept
//! widening the cell before it.
//!
//! Must not: render.

use super::{CellRecord, Item2, Placed, Reader, Record, TableRecord};
use crate::docx::find_path;
use crate::docx::tables::{merge_heads, row_cells, RowSpec, VMerge};
use crate::ids::W;
use crate::view::{content_kept, View};
use ooxml_xml::Element;
use std::collections::{HashMap, HashSet};
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
        let columns = find_path(&table.element, &["tblGrid"]).map_or(0, |g| {
            g.child_elements().filter(|c| c.name().is(Some(W), "gridCol")).count()
        });
        let mut grid = Grid {
            columns,
            prefix,
            counting_header: true,
            heads: self.merge_heads(&root, &entry.rows),
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

    /// One row of a table in the view, into `grid`.
    fn table_row(&mut self, part: &str, table_id: &str, style: &str, grid: &mut Grid, r: usize, row: &Placed) {
        if self.view.removes_row(&row.element) {
            return;
        }
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
        let mut gone_here = HashSet::new();
        let spec = RowSpec::of(&row.element);
        let mut column = spec.before;
        let all = cells(row);
        for (n, tc) in all.iter().enumerate() {
            let span = spec.cells.get(n).map_or(1, |c| c.span);
            if self.view.removes_cell(&tc.element) {
                // one whose content is not deleted widens the cell before it, as Word's
                // Accept All does (round 54, R110)
                match line.last_mut() {
                    Some(before) if self.view == View::Final && content_kept(&tc.element) => {
                        before.column_span += span;
                    }
                    _ => gone_here.extend(column..column + span),
                }
            } else if let (true, Some(origin)) = (
                self.continues(&tc.element) && !grid.heads.contains_key(&(r, n)),
                grid.origins.get(&column).copied(),
            ) {
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
                    path: tc.path.clone(),
                });
            }
            column += span;
        }
        grid.columns = grid.columns.max(column + spec.after);
        if !line.is_empty() || all.is_empty() {
            grid.rows.push(line);
            grid.hidden.push(gone_here);
        }
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
    columns: usize,
    origins: HashMap<usize, (usize, usize)>,
    /// The cells continuing a merge the final view starts again or leaves alone.
    heads: HashMap<(usize, usize), VMerge>,
    rows: Vec<Vec<CellRecord>>,
    hidden: Vec<HashSet<usize>>,
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
