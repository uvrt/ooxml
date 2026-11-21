//! Owns: where a chart's data is in its workbook, for an edit that changes the data's shape
//! (ooxml-edit's `_Block`, `charts/chart.py`): each source a row or column of one sheet over
//! the same span (`Chart._block`, refused where the workbook is laid out in a way insertion
//! cannot follow), and the cells moved, cleared and written as a category or series is put in
//! or taken out -- the formulas grown or shrunk with them and the sheet's tables shifted
//! (`_insert_line_cells`, `_delete_line_cells`, `_append_series_cells`,
//! `_delete_series_cells`).
//!
//! Must not: change the chart's caches (`ooxml_chart::edit` does), or open the workbook but
//! through the [`Keeper`].

use super::chart_keeper::{refused, unported, Keeper};
use ooxml_base::pyfmt::repr_str;
use ooxml_chart::edit::{ChartTree, Which, POINT_SOURCES};
use ooxml_sheets::{parse_formula, Area, Axis, Cell, SheetRange, Shift, Write};
use ooxml_word_edit::EditError;
use std::collections::BTreeSet;

/// One source of a series as the block reads it: its kind, formula and range.
#[derive(Debug, Clone)]
pub(crate) struct Src {
    pub(crate) kind: String,
    pub(crate) formula: Option<String>,
    pub(crate) range: Option<SheetRange>,
}

/// A series' sources, by kind.
#[derive(Debug, Clone, Default)]
pub(crate) struct SerSources {
    pub(crate) sources: Vec<(Which, Src)>,
}

impl SerSources {
    pub(crate) fn of(chart: &ChartTree, series: usize) -> Result<SerSources, EditError> {
        let mut sources = Vec::new();
        for which in [
            Which::Name,
            Which::Categories,
            Which::Values,
            Which::Sizes,
            Which::Labels,
        ] {
            if let Some(source) = chart.source(series, which) {
                let range = match source.formula.as_deref() {
                    None => None,
                    Some(text) => parse_formula(text).map_err(|_| unported("a formula ooxml-sheets cannot read"))?,
                };
                sources.push((
                    which,
                    Src {
                        kind: source.kind,
                        formula: source.formula,
                        range,
                    },
                ));
            }
        }
        Ok(SerSources { sources })
    }

    pub(crate) fn get(&self, which: Which) -> Option<&Src> {
        self.sources.iter().find(|(w, _)| *w == which).map(|(_, s)| s)
    }

    fn range(&self, which: Which) -> Option<&SheetRange> {
        self.get(which).and_then(|s| s.range.as_ref())
    }

    /// The sources with one point per category (`point_sources`).
    pub(crate) fn point_sources(&self) -> impl Iterator<Item = (Which, &Src)> {
        POINT_SOURCES
            .into_iter()
            .filter_map(|which| self.get(which).map(|s| (which, s)))
    }
}

/// Every series' sources.
pub(crate) fn all_sources(chart: &ChartTree) -> Result<Vec<SerSources>, EditError> {
    (0..chart.series_count()).map(|s| SerSources::of(chart, s)).collect()
}

/// The chart's data as lines of cells (`_Block`): `down` where points run down a column.
#[derive(Debug, Clone)]
pub(crate) struct Block {
    pub(crate) sheet: String,
    pub(crate) down: bool,
    pub(crate) start: u32,
    pub(crate) count: u32,
}

/// `(across, start, length)` of a one-row or one-column range (`_line_of`).
fn line_of(range: &SheetRange, down: bool) -> (u32, u32, u32) {
    let area = range.area;
    if down {
        (area.first.column, area.first.row, area.rows())
    } else {
        (area.first.row, area.first.column, area.columns())
    }
}

impl Block {
    /// `Chart._block`: `None` where there is no workbook to keep in step (the edit is then
    /// cache only, its reason noted).
    pub(crate) fn find(
        address: &str,
        series: &[SerSources],
        keeper: &mut Keeper<'_>,
        count: usize,
    ) -> Result<Option<Block>, EditError> {
        let sources: Vec<&Src> = series
            .iter()
            .flat_map(|s| s.point_sources().map(|(_, src)| src))
            .filter(|s| s.formula.is_some())
            .collect();
        let Some(first) = sources.first() else {
            return Ok(None);
        };
        if let Some(bad) = sources.iter().find(|s| s.range.is_none()) {
            let text = bad.formula.as_deref().filter(|t| !t.is_empty());
            let spelled = text.map_or_else(|| "None".to_owned(), repr_str);
            return Err(refused(&format!(
                "{address}: the formula {spelled} is not a single range, so the workbook cannot be kept in step"
            )));
        }
        let ranges: Vec<&SheetRange> = sources.iter().filter_map(|s| s.range.as_ref()).collect();
        let sheets: BTreeSet<&str> = ranges.iter().map(|r| r.sheet.as_str()).collect();
        if sheets.len() != 1 {
            return Err(refused(&format!("{address}: the chart's data spans several sheets")));
        }
        let Some(range) = first.range.as_ref() else {
            return Ok(None);
        };
        if !keeper.sheet(range)? {
            return Ok(None);
        }
        if ranges.iter().any(|r| !r.is_line()) {
            return Err(refused(&format!(
                "{address}: a data range is not a single row or column (multi-level categories?); edit the structure in Excel"
            )));
        }
        let down = direction(&ranges, series);
        let starts: BTreeSet<(u32, u32)> = ranges
            .iter()
            .map(|r| {
                let (_, start, length) = line_of(r, down);
                (start, length)
            })
            .collect();
        let count = u32::try_from(count).unwrap_or(u32::MAX);
        let lined_up = starts.len() == 1 && starts.iter().next().is_some_and(|(_, length)| *length == count);
        if !lined_up {
            let mut spelled = BTreeSet::new();
            for r in &ranges {
                spelled.insert(
                    r.formula()
                        .map_err(|_| unported("a formula ooxml-sheets cannot spell"))?,
                );
            }
            let spelled: Vec<String> = spelled.into_iter().collect();
            return Err(refused(&format!(
                "{address}: the chart's ranges do not line up ({}); the workbook cannot be kept in step",
                spelled.join(", ")
            )));
        }
        let start = starts.iter().next().map_or(0, |(s, _)| *s);
        Ok(Some(Block {
            sheet: range.sheet.clone(),
            down,
            start,
            count,
        }))
    }

    pub(crate) fn cell(&self, across: u32, position: u32) -> Cell {
        if self.down {
            Cell {
                row: position,
                column: across,
            }
        } else {
            Cell {
                row: across,
                column: position,
            }
        }
    }

    /// `line_range`: `template` over `count` cells of line `across` from `start`.
    fn line_range(&self, template: &SheetRange, across: u32, start: u32, count: u32) -> SheetRange {
        let area = Area {
            first: self.cell(across, start),
            last: self.cell(across, start + count.saturating_sub(1)),
        };
        SheetRange {
            area,
            ..template.clone()
        }
    }

    fn axis(&self) -> Axis {
        if self.down {
            Axis::Row
        } else {
            Axis::Column
        }
    }

    fn across_axis(&self) -> Axis {
        if self.down {
            Axis::Column
        } else {
            Axis::Row
        }
    }

    /// `_lines`: `(series, source, across)` for every source with cells, each line once (the
    /// first source on it), by line.
    fn lines(&self, series: &[SerSources]) -> Vec<(usize, Which, u32)> {
        let mut seen: Vec<(usize, Which, u32)> = Vec::new();
        for (at, ser) in series.iter().enumerate() {
            for (which, source) in ser.point_sources() {
                if let Some(range) = &source.range {
                    let across = line_of(range, self.down).0;
                    if !seen.iter().any(|(_, _, a)| *a == across) {
                        seen.push((at, which, across));
                    }
                }
            }
        }
        seen.sort_by_key(|(_, _, across)| *across);
        seen
    }

    /// `_check_free`: every cell at `positions` of the lines `across` empty.
    pub(crate) fn check_free(
        &self,
        address: &str,
        keeper: &mut Keeper<'_>,
        series: &[SerSources],
        positions: &[u32],
        across: Option<&[u32]>,
    ) -> Result<(), EditError> {
        let lines: Vec<u32> = match across {
            Some(lines) => lines.to_vec(),
            None => self.lines(series).into_iter().map(|(_, _, a)| a).collect(),
        };
        for line in lines {
            for &position in positions {
                let cell = self.cell(line, position);
                if !keeper.is_empty(&self.sheet, cell)? {
                    let spelled = cell.reference().map_err(|_| unported("a cell off the sheet"))?;
                    return Err(refused(&format!(
                        "{address}: workbook cell {spelled} is in the way of the chart's data growing; move it in Excel first"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Every ranged source's formula given `count` cells of its line (`set_range`).
    fn resize(&self, chart: &mut ChartTree, series: &[SerSources], count: u32) -> Result<(), EditError> {
        for (at, ser) in series.iter().enumerate() {
            for (which, source) in ser.point_sources() {
                if let Some(range) = &source.range {
                    let across = line_of(range, self.down).0;
                    let resized = self.line_range(range, across, self.start, count);
                    chart.set_formula(at, which, &spell(&resized)?);
                }
            }
        }
        Ok(())
    }

    /// `_insert_line_cells`: a category's cells put in at `position` in every line, the
    /// later cells moved along, the new ones written (styled as their neighbour).
    pub(crate) fn insert_line(
        &self,
        chart: &mut ChartTree,
        keeper: &mut Keeper<'_>,
        series: &[SerSources],
        position: u32,
        label: Option<&str>,
        values: &[Option<f64>],
    ) -> Result<(), EditError> {
        let target = self.start + position;
        let last = self.start + self.count - 1;
        let lines = self.lines(series);
        let payload = |ser: usize, which: Which| match which {
            Which::Categories => label.map(|l| Write::Text(l.to_owned())),
            Which::Values => values.get(ser).copied().flatten().map(Write::Number),
            _ => None,
        };
        for &(ser, which, across) in &lines {
            for at in (target..=last).rev() {
                keeper.move_cell(&self.sheet, self.cell(across, at), self.cell(across, at + 1))?;
            }
            let neighbour = if position < self.count { target + 1 } else { target - 1 };
            let given = payload(ser, which).unwrap_or(Write::Blank);
            let given = label_for(series.get(ser).and_then(|s| s.get(which)), given)?;
            keeper.set(
                &self.sheet,
                self.cell(across, target),
                &given,
                Some(self.cell(across, neighbour)),
            )?;
        }
        let span = span_of(&lines);
        let shift = Shift {
            axis: self.axis(),
            position: target,
            delta: 1,
            span,
        };
        keeper.shift(&self.sheet, shift)?;
        self.resize(chart, series, self.count + 1)?;
        keeper.sync(&self.sheet)
    }

    /// `_delete_line_cells`: a category's cells taken out of every line, the later cells
    /// moved back.
    pub(crate) fn delete_line(
        &self,
        chart: &mut ChartTree,
        keeper: &mut Keeper<'_>,
        series: &[SerSources],
        index: u32,
    ) -> Result<(), EditError> {
        let target = self.start + index;
        let last = self.start + self.count - 1;
        let lines = self.lines(series);
        for &(_, _, across) in &lines {
            keeper.clear(&self.sheet, self.cell(across, target))?;
            for at in target + 1..=last {
                keeper.move_cell(&self.sheet, self.cell(across, at), self.cell(across, at - 1))?;
            }
        }
        let shift = Shift {
            axis: self.axis(),
            position: target,
            delta: -1,
            span: span_of(&lines),
        };
        keeper.shift(&self.sheet, shift)?;
        self.resize(chart, series, self.count - 1)?;
        keeper.sync(&self.sheet)
    }

    /// `_header`: where the series names' header cells are, when the names live in the sheet.
    fn header(&self, series: &[SerSources]) -> Option<u32> {
        for ser in series {
            let (Some(name), Some(values)) = (ser.range(Which::Name), ser.range(Which::Values)) else {
                continue;
            };
            let (name_across, name_at, _) = line_of(name, self.down);
            if name_across == line_of(values, self.down).0 && Some(name_at) == self.start.checked_sub(1) {
                return Some(name_at);
            }
        }
        None
    }

    fn value_lines(&self, series: &[SerSources]) -> Vec<u32> {
        let lines: BTreeSet<u32> = series
            .iter()
            .filter_map(|s| s.range(Which::Values))
            .map(|r| line_of(r, self.down).0)
            .collect();
        lines.into_iter().collect()
    }

    /// `_append_series_cells`: the new series (at `new`) given the next free line, its name in
    /// the header cell where the names live in the sheet.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn append_series(
        &self,
        address: &str,
        chart: &mut ChartTree,
        keeper: &mut Keeper<'_>,
        series: &[SerSources],
        new: usize,
        name: Option<&str>,
        values: &[Option<f64>],
    ) -> Result<(), EditError> {
        let lines = self.lines(series);
        let value_lines = self.value_lines(series);
        let across = lines.iter().map(|(_, _, a)| *a).max().unwrap_or(0) + 1;
        let header = self.header(series);
        let mut positions: Vec<u32> = (self.start..self.start + self.count).collect();
        if let Some(header) = header {
            positions.insert(0, header);
        }
        self.check_free(address, keeper, series, &positions, Some(&[across]))?;
        let previous = value_lines.last().copied().unwrap_or(span_of(&lines).1);
        let Some(template) = series.iter().find_map(|s| s.range(Which::Values)) else {
            return Err(unported("a chart whose series draw their values from no range"));
        };
        let own = self.line_range(template, across, self.start, self.count);
        chart.set_formula(new, Which::Values, &spell(&own)?);
        for (offset, value) in values.iter().enumerate() {
            let position = self.start + u32::try_from(offset).unwrap_or(u32::MAX);
            let given = value.map_or(Write::Blank, Write::Number);
            keeper.set(
                &self.sheet,
                self.cell(across, position),
                &given,
                Some(self.cell(previous, position)),
            )?;
        }
        let named = chart.source(new, Which::Name).is_some_and(|s| s.formula.is_some());
        match header {
            Some(header) if named => {
                let range = self.line_range(template, across, header, 1);
                chart.set_formula(new, Which::Name, &spell(&range)?);
                let given = name.map_or(Write::Blank, |n| Write::Text(n.to_owned()));
                keeper.set(
                    &self.sheet,
                    self.cell(across, header),
                    &given,
                    Some(self.cell(previous, header)),
                )?;
            }
            _ if named => chart.name_to_literal(new, name),
            _ => {}
        }
        let low = header.unwrap_or(self.start);
        let shift = Shift {
            axis: self.across_axis(),
            position: across,
            delta: 1,
            span: (low, self.start + self.count - 1),
        };
        keeper.shift(&self.sheet, shift)?;
        keeper.sync(&self.sheet)
    }

    /// `_delete_series_cells`: the doomed series' cells cleared and, where the series sit
    /// side by side, the later series' lines moved back one.
    pub(crate) fn delete_series(
        &self,
        chart: &mut ChartTree,
        keeper: &mut Keeper<'_>,
        series: &[SerSources],
        doomed: usize,
    ) -> Result<(), EditError> {
        let header = self.header(series);
        let Some(gone) = series.get(doomed) else { return Ok(()) };
        if let Some(labels) = gone.range(Which::Labels) {
            let (line, start, length) = line_of(labels, self.down);
            for position in start..start + length {
                keeper.clear(&self.sheet, self.cell(line, position))?;
            }
            if header.is_some() && header == start.checked_sub(1) {
                if let Some(header) = header {
                    keeper.clear(&self.sheet, self.cell(line, header))?;
                }
            }
        }
        let Some(own) = gone.range(Which::Values) else {
            return Ok(());
        };
        let across = line_of(own, self.down).0;
        let others: Vec<(usize, &SerSources)> = series.iter().enumerate().filter(|(s, _)| *s != doomed).collect();
        let shared = others.iter().any(|(_, s)| {
            s.range(Which::Values)
                .is_some_and(|r| line_of(r, self.down).0 == across)
        });
        if shared {
            return Ok(()); // another series draws from the same cells
        }
        let low = header.unwrap_or(self.start);
        let high = self.start + self.count - 1;
        let value_lines = self.value_lines(series);
        let contiguous = value_lines
            .iter()
            .zip(value_lines.iter().skip(1))
            .all(|(a, b)| *b == a + 1);
        let category_lines: BTreeSet<u32> = series
            .iter()
            .filter_map(|s| s.range(Which::Categories))
            .map(|r| line_of(r, self.down).0)
            .collect();
        let later: Vec<u32> = value_lines.iter().copied().filter(|&a| a > across).collect();
        for position in low..=high {
            keeper.clear(&self.sheet, self.cell(across, position))?;
        }
        if !contiguous || later.iter().any(|l| category_lines.contains(l)) {
            return keeper.sync(&self.sheet);
        }
        for &line in &later {
            for position in low..=high {
                keeper.move_cell(&self.sheet, self.cell(line, position), self.cell(line - 1, position))?;
            }
        }
        for &(s, sources) in &others {
            for which in [Which::Name, Which::Values, Which::Sizes] {
                let Some(range) = sources.range(which) else { continue };
                let (line, start, length) = line_of(range, self.down);
                if later.contains(&line) {
                    let moved = self.line_range(range, line - 1, start, length);
                    chart.set_formula(s, which, &spell(&moved)?);
                }
            }
        }
        let shift = Shift {
            axis: self.across_axis(),
            position: across,
            delta: -1,
            span: (low, high),
        };
        keeper.shift(&self.sheet, shift)?;
        keeper.sync(&self.sheet)
    }
}

/// The first and last line (`lines` is by line, and a block has some).
fn span_of(lines: &[(usize, Which, u32)]) -> (u32, u32) {
    let first = lines.first().map_or(0, |l| l.2);
    (first, lines.last().map_or(first, |l| l.2))
}

/// `_direction`: whether points run down columns; a one-point chart is told by where its
/// name sits.
fn direction(ranges: &[&SheetRange], series: &[SerSources]) -> bool {
    for range in ranges {
        if range.area.rows() > 1 {
            return true;
        }
        if range.area.columns() > 1 {
            return false;
        }
    }
    for ser in series {
        if let (Some(name), Some(values)) = (ser.range(Which::Name), ser.range(Which::Values)) {
            if name.area.first.column == values.area.first.column {
                return true;
            }
            if name.area.first.row == values.area.first.row {
                return false;
            }
        }
    }
    true
}

/// `_label_for`: a category label as a cell value -- numeric categories' a number.
fn label_for(source: Option<&Src>, given: Write) -> Result<Write, EditError> {
    let numeric = source.is_some_and(|s| matches!(s.kind.as_str(), "numRef" | "numLit"));
    match given {
        Write::Text(text) if numeric => match ooxml_chart::data::decode_number(&text) {
            Some(number) => Ok(Write::Number(number.number().unwrap_or_default())),
            None => Err(refused(&format!(
                "{} is not a number, and these categories are numeric",
                repr_str(&text)
            ))),
        },
        other => Ok(other),
    }
}

fn spell(range: &SheetRange) -> Result<String, EditError> {
    range
        .formula()
        .map_err(|_| unported("a formula ooxml-sheets cannot spell"))
}
