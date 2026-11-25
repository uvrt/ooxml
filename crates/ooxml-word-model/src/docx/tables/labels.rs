//! Owns: a cell of a [`TableGrid`] found by place or by labels, as docx-agent's
//! `word_edit_table` finds one (`tools/objects.py`, `_locate`): a row by its first cell's
//! text, a column by a header row's text (white space collapsed, one match or a refusal
//! naming the texts), and the refusal for a place no cell covers (`no_cell_message`, with
//! the table's shape).
//!
//! Must not: change a table, or count a table as accepting its tracked changes leaves it.

use super::places::{GridPlaces, Spot};
use super::TableGrid;
use crate::address::AddressError;

/// Text with Python's `" ".join(text.split())`: runs of white space as one space, none at
/// either end.
#[must_use]
pub fn collapse_space(text: &str) -> String {
    text.split(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The one of `texts` equal to `label` with white space collapsed, else `not_found` (none)
/// or `ambiguous` (several), the texts the options (`_locate`'s `labelled`).
pub(super) fn labelled(texts: &[String], label: &str, what: &str) -> Result<usize, AddressError> {
    let clean: Vec<String> = texts.iter().map(|t| collapse_space(t)).collect();
    let wanted = collapse_space(label);
    let hits: Vec<usize> = (0..clean.len())
        .filter(|k| clean.get(*k).is_some_and(|t| *t == wanted))
        .collect();
    if let [one] = hits.as_slice() {
        return Ok(*one);
    }
    let message = format!(
        "{what} label {} matches {} {what}s",
        ooxml_base::pyfmt::repr_str(label),
        hits.len()
    );
    let options: Vec<String> = clean.into_iter().filter(|t| !t.is_empty()).take(50).collect();
    Err(AddressError {
        code: if hits.is_empty() { "not_found" } else { "ambiguous" },
        message,
        options,
    })
}

impl TableGrid<'_> {
    /// The cell origin `(row, column)` over grid position `(row, column)`.
    #[must_use]
    pub fn origin(&self, row: usize, column: usize) -> Option<(usize, usize)> {
        self.grid.get(row, column)
    }

    /// The row whose first cell's text is `label`.
    ///
    /// # Errors
    /// `not_found` or `ambiguous`, the rows' texts the options.
    pub fn row_labelled(&self, label: &str) -> Result<usize, AddressError> {
        let texts: Vec<String> = (0..self.row_count())
            .map(|r| self.cell_text(r, 0).unwrap_or_default())
            .collect();
        labelled(&texts, label, "row")
    }

    /// The grid column whose cell in `header_row` has the text `label`.
    ///
    /// # Errors
    /// `not_found` or `ambiguous`, the header row's texts the options.
    pub fn column_labelled(&self, header_row: usize, label: &str) -> Result<usize, AddressError> {
        let texts: Vec<String> = (0..self.column_count())
            .map(|c| self.cell_text(header_row, c).unwrap_or_default())
            .collect();
        labelled(&texts, label, "column")
    }

    /// The ids `<table>/cR,C` of the table's cells, each merged cell once at its first
    /// place, in grid order (docx-agent's `_origins`).
    #[must_use]
    pub fn cell_ids(&self, table: &str) -> Vec<String> {
        let mut seen: Vec<(usize, usize)> = Vec::new();
        for run in (0..self.grid.row_count()).flat_map(|r| self.grid.runs(r)) {
            if !seen.contains(&run.origin) {
                seen.push(run.origin);
            }
        }
        seen.into_iter().map(|(r, c)| format!("{table}/c{r},{c}")).collect()
    }

    /// No cell at `(row, column)` of the table `table` (docx-agent's `NoCell`): the
    /// message says the table's rows and columns, each merged cell with the positions it
    /// covers, the positions no cell covers; its cells' ids (the first 50) the options.
    #[must_use]
    pub fn no_cell(&self, table: &str, row: usize, column: usize) -> AddressError {
        let text = no_cell_text(&self.grid, table, row, column);
        let mut ids = self.cell_ids(table);
        ids.truncate(50);
        AddressError::not_found(text, ids)
    }
}

/// `t:X has no cell at (1, 2)`, then what the table whose grid is `grid` (each position's
/// cell as its first position, none where no cell covers it) has: its rows and columns,
/// each merged cell with the positions it covers, the positions no cell covers
/// (docx-agent's `no_cell_message`). The one wording of it, for the edit core's refusals
/// too. Counted by runs of columns, not by position: a span of billions of columns costs
/// one run.
#[must_use]
pub fn no_cell_text(grid: &GridPlaces, table: &str, row: usize, column: usize) -> String {
    let columns = grid.width();
    // each origin with how many positions it covers and its last, in the order first met
    let mut spans: Vec<(Spot, usize, Spot)> = Vec::new();
    let mut empty = Listed::default();
    for r in 0..grid.row_count() {
        let mut at = 0;
        for run in grid.runs(r) {
            empty.add_columns(r, at..run.columns.start);
            let last = (r, run.columns.end.saturating_sub(1));
            let count = run.columns.len();
            match spans.iter_mut().find(|(o, _, _)| *o == run.origin) {
                Some((_, covered, end)) => {
                    *covered = covered.saturating_add(count);
                    *end = last;
                }
                None => spans.push((run.origin, count, last)),
            }
            at = run.columns.end;
        }
        empty.add_columns(r, at..columns);
    }
    let mut merged = Listed::default();
    for (o, _, l) in spans.iter().filter(|(_, covered, _)| *covered > 1) {
        merged.add(1, || format!("({}, {})-({}, {})", o.0, o.1, l.0, l.1));
    }
    let last = |n: usize| i64::try_from(n).unwrap_or(i64::MAX) - 1;
    let rows = grid.row_count();
    let mut text = format!(
        "{table} has no cell at ({row}, {column}): it has {rows} row(s) (0 to {}) and {columns} column(s) \
         (0 to {}), counted from 0",
        last(rows),
        last(columns)
    );
    if merged.total > 0 {
        text.push_str("; merged: ");
        text.push_str(&merged.text());
        text.push_str(" (a merged cell's id is its first position's)");
    }
    if empty.total > 0 {
        text.push_str("; no cell at ");
        text.push_str(&empty.text());
    }
    text
}

/// How many items a list shown as its first ten holds, and those ten.
#[derive(Default)]
struct Listed {
    shown: Vec<String>,
    total: usize,
}

/// Items a list shows before it counts the rest.
const SHOWN: usize = 10;

impl Listed {
    /// `count` more items, the first of them written by `text` while fewer than ten are
    /// shown.
    fn add(&mut self, count: usize, text: impl FnOnce() -> String) {
        if count > 0 && self.shown.len() < SHOWN {
            self.shown.push(text());
        }
        self.total = self.total.saturating_add(count);
    }

    /// The positions `(row, c)` for each `c` of `columns`.
    fn add_columns(&mut self, row: usize, columns: std::ops::Range<usize>) {
        for c in columns.clone().take(SHOWN.saturating_sub(self.shown.len())) {
            self.shown.push(format!("({row}, {c})"));
        }
        self.total = self.total.saturating_add(columns.len());
    }

    /// The first ten, the rest counted.
    fn text(&self) -> String {
        let mut out = self.shown.join(", ");
        if self.total > SHOWN {
            out.push_str(&format!(" and {} more", self.total - SHOWN));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{collapse_space, labelled, no_cell_text, GridPlaces, Spot};
    use crate::docx::tables::{CellSpec, Grid, RowSpec, VMerge};

    /// The refusal's text as it was written over one place per grid position.
    fn dense_text(grid: &[Vec<Option<Spot>>], table: &str, row: usize, column: usize) -> String {
        let columns = grid.iter().map(Vec::len).max().unwrap_or(0);
        let mut spans: Vec<(Spot, Vec<Spot>)> = Vec::new();
        let mut empty = Vec::new();
        for (r, line) in grid.iter().enumerate() {
            for c in 0..columns {
                match line.get(c).copied().flatten() {
                    None => empty.push(format!("({r}, {c})")),
                    Some(origin) => match spans.iter_mut().find(|(o, _)| *o == origin) {
                        Some((_, cells)) => cells.push((r, c)),
                        None => spans.push((origin, vec![(r, c)])),
                    },
                }
            }
        }
        let merged: Vec<String> = spans
            .iter()
            .filter(|(_, cells)| cells.len() > 1)
            .filter_map(|(o, cells)| cells.last().map(|l| format!("({}, {})-({}, {})", o.0, o.1, l.0, l.1)))
            .collect();
        let listed = |items: &[String]| {
            let mut out = items.iter().take(10).cloned().collect::<Vec<_>>().join(", ");
            if items.len() > 10 {
                out.push_str(&format!(" and {} more", items.len() - 10));
            }
            out
        };
        let last = |n: usize| i64::try_from(n).unwrap_or(i64::MAX) - 1;
        let mut text = format!(
            "{table} has no cell at ({row}, {column}): it has {} row(s) (0 to {}) and {columns} column(s) \
             (0 to {}), counted from 0",
            grid.len(),
            last(grid.len()),
            last(columns)
        );
        if !merged.is_empty() {
            text.push_str("; merged: ");
            text.push_str(&listed(&merged));
            text.push_str(" (a merged cell's id is its first position's)");
        }
        if !empty.is_empty() {
            text.push_str("; no cell at ");
            text.push_str(&listed(&empty));
        }
        text
    }

    #[test]
    fn the_refusal_reads_as_it_did_over_one_place_per_position() {
        let merges = [VMerge::None, VMerge::Restart, VMerge::Continue];
        let mut seed = 7u64;
        let mut next = |n: u64| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            usize::try_from((seed >> 33) % n).unwrap_or(0)
        };
        for _ in 0..2000 {
            let specs: Vec<RowSpec> = (0..next(5))
                .map(|_| {
                    let before = next(3).to_string();
                    let cells = (0..next(5))
                        .map(|_| {
                            CellSpec::new(
                                Some(&(1 + next(3)).to_string()),
                                merges.get(next(3)).copied().unwrap_or(VMerge::None),
                            )
                        })
                        .collect();
                    RowSpec::new(Some(&before), None, cells)
                })
                .collect();
            let places = GridPlaces::of(&Grid::build(&specs, 3));
            let dense: Vec<Vec<Option<Spot>>> = (0..places.row_count())
                .map(|r| (0..places.end(r)).map(|c| places.get(r, c)).collect())
                .collect();
            assert_eq!(
                no_cell_text(&places, "t:1", 9, 9),
                dense_text(&dense, "t:1", 9, 9),
                "{specs:?}"
            );
        }
    }

    #[test]
    fn white_space_collapses_as_python_splits() {
        assert_eq!(collapse_space("  a \t b\u{1f}c\n"), "a b c");
        assert_eq!(collapse_space(""), "");
    }

    #[test]
    fn a_label_matches_once_or_is_refused_with_the_texts() {
        let texts = vec![
            "Region".to_owned(),
            "".to_owned(),
            "North  ".to_owned(),
            "North".to_owned(),
        ];
        assert_eq!(labelled(&texts, " Region", "row").ok(), Some(0));
        let twice = labelled(&texts, "North", "row").err();
        assert_eq!(twice.as_ref().map(|e| e.code), Some("ambiguous"));
        assert_eq!(
            twice.map(|e| e.message),
            Some("row label 'North' matches 2 rows".to_owned())
        );
        let none = labelled(&texts, "South", "column").err();
        assert_eq!(none.as_ref().map(|e| e.code), Some("not_found"));
        assert_eq!(none.map(|e| e.options.len()), Some(3));
    }
}
