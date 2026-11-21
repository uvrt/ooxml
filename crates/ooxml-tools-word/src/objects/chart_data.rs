//! Owns: `edit_chart`'s data actions that change points in place (docx-agent's `_edit_chart`
//! through ooxml-edit's `apply_data_action`): `set_values`, `set_value`, `rename_series` and
//! `rename_category` -- the chart part's caches changed (`ooxml_chart::edit`) and the
//! embedded workbook kept in step cell by cell (`_Editor.write`, with `ooxml-sheets`'
//! writer), or, where it cannot be, the chart changed in its cache only and the result
//! warned (`ChartDataWarning`, in ooxml-edit's words).
//!
//! Must not: resolve a series or category by name (the handler does, `chart_names`), or
//! write but through the [`EditGraphic`](ooxml_word_edit::EditGraphic) command.

use super::chart_workbook::workbook_part;
use ooxml_base::pyfmt::repr_str;
use ooxml_chart::data::Point;
use ooxml_chart::edit::{ChartTree, DataError, PointValue, SourceRef, Which};
use ooxml_opc::{Limits, Package};
use ooxml_sheets::{parse_formula, SheetsError, WorkbookWriter, Write};
use ooxml_word_edit::{EditError, GraphicEdit, GraphicParts, PartWrite};
use std::sync::Arc;

/// What docx-agent calls the package in a reason (`edit/charts.py`'s `DOCUMENT`).
const DOCUMENT: &str = "document";
/// The application whose Edit Data a warning names (`edit/charts.py`'s `APPLICATION`).
const APPLICATION: &str = "Word";

/// A data action, its series and category resolved to positions.
#[derive(Debug, Clone)]
pub(crate) enum DataAction {
    /// Every value of a series (`None` a blank).
    SetValues { series: usize, values: Vec<Option<f64>> },
    /// One value.
    SetValue { series: usize, category: usize, value: f64 },
    /// A series' name.
    RenameSeries { series: usize, text: String },
    /// A category's label in every series.
    RenameCategory { category: usize, text: String },
}

/// A data action on the chart `address` (the target, as ooxml-edit's messages name it).
#[derive(Debug, Clone)]
pub(crate) struct ChartDataEdit {
    pub(crate) address: String,
    pub(crate) action: DataAction,
}

/// What the edit says beside the chart: its warnings (a cache-only edit).
pub(crate) type Warnings = Vec<String>;

impl GraphicEdit for ChartDataEdit {
    type Out = Warnings;

    fn edit(&self, package: &Package, part: &str) -> Result<GraphicParts<Warnings>, EditError> {
        let tree = package.tree(part)?.ok_or_else(|| EditError::NoPart(part.to_owned()))?;
        let mut edit = Edit {
            chart: ChartTree::new(Arc::clone(tree.root())),
            keeper: Keeper::new(package, part),
            ran: false,
            changed: false,
        };
        match &self.action {
            DataAction::SetValues { series, values } => edit.set_values(&self.address, *series, values)?,
            DataAction::SetValue {
                series,
                category,
                value,
            } => edit.set_value(*series, *category, *value)?,
            DataAction::RenameSeries { series, text } => edit.rename_series(*series, text)?,
            DataAction::RenameCategory { category, text } => edit.rename_category(*category, text)?,
        }
        let mut writes = Vec::new();
        if edit.changed {
            writes.push(PartWrite::Tree(
                part.to_owned(),
                tree.with_root(Arc::clone(edit.chart.root())),
            ));
        }
        let mut read = Vec::new();
        let warnings = edit.keeper.finish(&mut writes, &mut read)?;
        let warnings = if edit.ran {
            warnings
                .map(|why| {
                    format!("ChartDataWarning: {part}: only the chart's cached data was changed, because {why}; {APPLICATION}'s Edit Data will show the old values")
                })
                .into_iter()
                .collect()
        } else {
            Vec::new()
        };
        Ok(GraphicParts {
            read,
            writes,
            out: warnings,
        })
    }
}

/// One edit of the chart part and its workbook (ooxml-edit's `Chart._edit`).
struct Edit<'p> {
    chart: ChartTree,
    keeper: Keeper<'p>,
    /// Whether the edit step ran (`with chart._edit()`), which warns.
    ran: bool,
    /// Whether the chart part changed (`editor.changed`).
    changed: bool,
}

impl Edit<'_> {
    fn points(&self, series: usize, which: Which) -> Result<Vec<Option<Point>>, EditError> {
        self.chart
            .points(series, which)
            .map_err(|_| unported("a chart whose data the model cannot read"))
    }

    fn set(&mut self, series: usize, which: Which, index: usize, value: &PointValue) -> Result<bool, EditError> {
        self.chart.set_point(series, which, index, value).map_err(data_refusal)
    }

    fn set_values(&mut self, address: &str, series: usize, values: &[Option<f64>]) -> Result<(), EditError> {
        let current = self.points(series, Which::Values)?;
        if values.len() != current.len() {
            return Err(refused(&format!(
                "{address}: {} values for {} categories; add or remove categories first",
                values.len(),
                current.len()
            )));
        }
        if current.iter().zip(values).all(|(a, b)| same(a.as_ref(), *b)) {
            return Ok(());
        }
        self.ran = true;
        let source = self.chart.source(series, Which::Values);
        for (index, (was, value)) in current.iter().zip(values).enumerate() {
            if !same(was.as_ref(), *value) {
                let given = value.map_or(PointValue::Blank, PointValue::Number);
                self.set(series, Which::Values, index, &given)?;
                self.keeper.write(source.as_ref(), index, &given)?;
            }
        }
        self.changed = true;
        Ok(())
    }

    fn set_value(&mut self, series: usize, category: usize, value: f64) -> Result<(), EditError> {
        let current = self.points(series, Which::Values)?;
        let was = current
            .get(category)
            .ok_or_else(|| refused("list index out of range"))?;
        if same(was.as_ref(), Some(value)) {
            return Ok(());
        }
        self.ran = true;
        let source = self.chart.source(series, Which::Values);
        let given = PointValue::Number(value);
        self.set(series, Which::Values, category, &given)?;
        self.keeper.write(source.as_ref(), category, &given)?;
        self.changed = true;
        Ok(())
    }

    fn rename_series(&mut self, series: usize, text: &str) -> Result<(), EditError> {
        let name = self.points(series, Which::Name)?.into_iter().next().flatten();
        if name == Some(Point::Text(text.to_owned())) {
            return Ok(());
        }
        self.ran = true;
        let given = PointValue::Text(text.to_owned());
        match self.chart.source(series, Which::Name) {
            None => self.chart.give_name(series, text),
            Some(source) => {
                self.set(series, Which::Name, 0, &given)?;
                self.keeper.write(Some(&source), 0, &given)?;
            }
        }
        self.changed = true;
        Ok(())
    }

    fn rename_category(&mut self, category: usize, text: &str) -> Result<(), EditError> {
        let label = Point::Text(text.to_owned());
        let mut with_categories = Vec::new();
        for series in 0..self.chart.series_count() {
            if let Some(source) = self.chart.source(series, Which::Categories) {
                with_categories.push((series, source));
            }
        }
        let mut all_same = true;
        for (series, _) in &with_categories {
            let points = self.points(*series, Which::Categories)?;
            let was = points.get(category).ok_or_else(|| refused("list index out of range"))?;
            all_same &= was.as_ref() == Some(&label);
        }
        if all_same {
            return Ok(());
        }
        self.ran = true;
        let given = PointValue::Text(text.to_owned());
        let mut written: Vec<String> = Vec::new();
        for (series, source) in with_categories {
            if self.set(series, Which::Categories, category, &given)? {
                self.changed = true;
            }
            if let Some(formula) = source.formula.as_ref().filter(|f| !written.contains(f)) {
                written.push(formula.clone());
                self.keeper.write(Some(&source), category, &given)?;
            }
        }
        Ok(())
    }
}

/// `_same_number`: two blanks, equal texts, or equal numbers.
fn same(a: Option<&Point>, b: Option<f64>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(point), Some(number)) => point.number().is_some_and(|n| n == number),
        _ => false,
    }
}

/// The embedded workbook kept in step with the cache (ooxml-edit's `_Editor`): opened when
/// first written, or the reason the edit changed the cache only.
struct Keeper<'p> {
    package: &'p Package,
    chart_part: &'p str,
    opened: bool,
    workbook: Option<(String, WorkbookWriter)>,
    cache_only: Option<String>,
}

impl<'p> Keeper<'p> {
    fn new(package: &'p Package, chart_part: &'p str) -> Keeper<'p> {
        Keeper {
            package,
            chart_part,
            opened: false,
            workbook: None,
            cache_only: None,
        }
    }

    fn note(&mut self, reason: String) {
        if self.cache_only.is_none() {
            self.cache_only = Some(reason);
        }
    }

    /// Open the workbook the first time it is written (`_Editor.workbook`): a reason the
    /// chart has none replaces any noted before, as ooxml-edit sets it.
    fn open(&mut self) -> Result<(), EditError> {
        if !self.opened {
            self.opened = true;
            let root = self
                .package
                .tree(self.chart_part)?
                .ok_or_else(|| EditError::NoPart(self.chart_part.to_owned()))?;
            let found =
                workbook_part(self.package, self.chart_part, root.root(), DOCUMENT).map_err(|why| unported(why.0))?;
            match found {
                Err(reason) => self.cache_only = Some(reason),
                Ok(part) => {
                    let bytes = self
                        .package
                        .read(&part)?
                        .ok_or_else(|| EditError::NoPart(part.clone()))?;
                    // ooxml-edit words a workbook it cannot open with the exception's text
                    let writer = WorkbookWriter::open(bytes, &Limits::default())
                        .map_err(|_| unported("an embedded workbook that cannot be read"))?;
                    self.workbook = Some((part, writer));
                }
            }
        }
        Ok(())
    }

    /// Write point `index` of `source` into its cell (`_Editor.write`).
    fn write(&mut self, source: Option<&SourceRef>, index: usize, value: &PointValue) -> Result<(), EditError> {
        let Some(formula) = source.and_then(|s| s.formula.as_deref()) else {
            return Ok(()); // a literal: nothing in the workbook to keep in step
        };
        let range = match parse_formula(formula) {
            Ok(Some(range)) => range,
            Ok(None) => {
                self.note(format!("the formula {} is not a single range", repr_str(formula)));
                return Ok(());
            }
            Err(_) => return Err(unported("a formula ooxml-sheets cannot read")),
        };
        self.open()?;
        let has_sheet = match self.workbook.as_mut() {
            Some((_, workbook)) => workbook.has_sheet(&range.sheet).map_err(sheets_error)?,
            None => return Ok(()),
        };
        if !has_sheet {
            self.note(format!("its workbook has no sheet {}", repr_str(&range.sheet)));
            return Ok(());
        }
        let cell = u32::try_from(index)
            .ok()
            .filter(|i| range.is_line() && *i < range.length())
            .and_then(|i| range.cell(i));
        let Some(cell) = cell else {
            let spelled = range
                .formula()
                .map_err(|_| unported("a formula ooxml-sheets cannot spell"))?;
            self.note(format!("the formula {spelled} does not have a cell for point {index}"));
            return Ok(());
        };
        let given = match value {
            PointValue::Blank => Write::Blank,
            PointValue::Text(text) => Write::Text(text.clone()),
            PointValue::Number(number) => Write::Number(*number),
        };
        match self.workbook.as_mut() {
            Some((_, workbook)) => workbook
                .set_value(&range.sheet, cell, &given, None)
                .map_err(sheets_error),
            None => Ok(()),
        }
    }

    /// The workbook's bytes among `writes` where it changed (its part among `read` where it
    /// was opened); the reason the edit changed the cache only, if it did.
    fn finish(self, writes: &mut Vec<PartWrite>, read: &mut Vec<String>) -> Result<Option<String>, EditError> {
        if let Some((part, writer)) = self.workbook {
            read.push(part.clone());
            if let Some(bytes) = writer.finish().map_err(sheets_error)? {
                writes.push(PartWrite::Bytes(part, Arc::from(bytes)));
            }
        }
        Ok(self.cache_only)
    }
}

/// A chart edit ooxml-edit refuses (`ChartDataError`, `IndexError`), as docx-agent words it:
/// `invalid_arguments` with the message stripped of quotes at its ends.
fn refused(message: &str) -> EditError {
    EditError::Argument {
        field: String::new(),
        message: message.trim_matches(['\'', '"']).to_owned(),
        options: Vec::new(),
    }
}

fn data_refusal(error: DataError) -> EditError {
    refused(&error.0)
}

fn unported(what: &str) -> EditError {
    EditError::Unsupported(format!("edit_chart: {what}"))
}

fn sheets_error(error: SheetsError) -> EditError {
    match error {
        SheetsError::Unsupported(what) => unported(&what),
        // ooxml-edit words a workbook it cannot write with its exceptions' texts
        _ => unported("an embedded workbook ooxml-sheets cannot write"),
    }
}
