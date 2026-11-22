//! Owns: `edit_chart`'s data actions (docx-agent's `_edit_chart` through ooxml-edit's
//! `apply_data_action`): `set_values`, `set_value`, `rename_series` and `rename_category`
//! (points changed in place), `add_category`, `remove_category`, `add_series` and
//! `remove_series` (the data's shape: `chart_block` keeps the workbook's lines in step),
//! `set_title` and `set_axis_title`, `set_legend`, `show_data_labels`, `hide_data_labels`
//! and `set_gap_width` -- the chart part's caches changed (`ooxml_chart::edit`) and the
//! embedded workbook kept in step (`_Editor`, with `ooxml-sheets`' writer), or, where it
//! cannot be, the chart changed in its cache only and the result warned
//! (`ChartDataWarning`, in ooxml-edit's words); the chart's record of where its values
//! came from kept true (`chart_record`).
//!
//! Must not: resolve a series or category by name (the handler does, `chart_names`), or
//! write but through the [`EditGraphic`](ooxml_word_edit::EditGraphic) command.

use super::chart_block::{all_sources, Block};
use super::chart_keeper::{refused, unported, Keeper};
use super::chart_record::kept;
use ooxml_chart::data::Point;
use ooxml_chart::edit::{ChartTree, DataError, LabelLook, PointValue, TitleEdit, TitleLook, Which};
use ooxml_opc::Package;
use ooxml_word_edit::{EditError, GraphicEdit, GraphicParts, PartWrite};
use std::sync::Arc;

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
    /// A category added last, with a value per series (`None`: blanks).
    AddCategory {
        text: String,
        values: Option<Vec<Option<f64>>>,
    },
    /// A category taken out.
    RemoveCategory { category: usize },
    /// A series added last (`None`: blank values).
    AddSeries {
        text: String,
        values: Option<Vec<Option<f64>>>,
    },
    /// A series taken out.
    RemoveSeries { series: usize },
    /// The legend shown at a position (`r`, `l`, `t`, `b`) or, with `None`, hidden.
    SetLegend { position: Option<&'static str> },
    /// Value labels shown or hidden on one series or all.
    DataLabels {
        show: bool,
        series: Option<usize>,
        number_format: Option<String>,
        /// The labels' size (Word's look) and the document's language.
        size: Option<u32>,
        lang: Option<String>,
    },
    /// The bar plots' gap width.
    GapWidth { percent: i64 },
    /// The chart's title (`axis` none) or an axis' (`category`, `value`) given text or, with
    /// `None`, taken away; new title text at the application's sizes in `lang`.
    Title {
        axis: Option<&'static str>,
        text: Option<String>,
        sizes: (u32, u32),
        lang: Option<String>,
    },
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
            DataAction::AddCategory { text, values } => edit.add_category(&self.address, text, values.as_deref())?,
            DataAction::RemoveCategory { category } => edit.remove_category(&self.address, *category)?,
            DataAction::AddSeries { text, values } => {
                edit.add_series(&self.address, part, text, values.as_deref())?;
            }
            DataAction::RemoveSeries { series } => edit.remove_series(&self.address, *series)?,
            DataAction::SetLegend { position } => {
                edit.changed = edit.chart.set_legend(*position);
                edit.ran = edit.changed;
            }
            DataAction::DataLabels {
                show,
                series,
                number_format,
                size,
                lang,
            } => {
                let look = LabelLook {
                    size: *size,
                    number_format: number_format.as_deref(),
                    lang: lang.as_deref(),
                };
                edit.data_labels(&self.address, *show, *series, &look)?;
            }
            DataAction::GapWidth { percent } => edit.gap_width(&self.address, *percent)?,
            DataAction::Title {
                axis,
                text,
                sizes,
                lang,
            } => {
                let look = TitleLook {
                    title: sizes.0,
                    axis_title: sizes.1,
                    lang: lang.as_deref(),
                };
                edit.title(*axis, text.as_deref(), &look)?;
            }
        }
        // the record of where the values came from kept true (ooxml-edit's `_edit_record`)
        let (recorded, record) = kept(edit.chart.root(), &self.action);
        let mut writes = Vec::new();
        if edit.changed || recorded.is_some() {
            let root = recorded.map_or_else(|| Arc::clone(edit.chart.root()), Arc::new);
            writes.push(PartWrite::Tree(part.to_owned(), tree.with_root(root)));
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
            record,
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

impl Edit<'_> {
    /// `add_category` (last): a value per series, the cells beside the data checked free,
    /// the caches and the workbook's lines grown.
    fn add_category(&mut self, address: &str, text: &str, values: Option<&[Option<f64>]>) -> Result<(), EditError> {
        let count = self.chart.point_count();
        let series = self.chart.series_count();
        let per_series = match values {
            None => vec![None; series],
            Some(values) if values.len() == series => values.to_vec(),
            Some(values) => return Err(refused(&format!("{} values for {series} series", values.len()))),
        };
        self.ran = true;
        let sources = all_sources(&self.chart)?;
        let block = Block::find(address, &sources, &mut self.keeper, count)?;
        if let Some(block) = &block {
            let free = block.start + u32::try_from(count).unwrap_or(u32::MAX);
            block.check_free(address, &mut self.keeper, &sources, &[free], None)?;
        }
        let label = PointValue::Text(text.to_owned());
        self.chart
            .insert_category(count, Some(&label), &per_series)
            .map_err(data_refusal)?;
        self.changed = true;
        if let Some(block) = block {
            let position = u32::try_from(count).unwrap_or(u32::MAX);
            block.insert_line(
                &mut self.chart,
                &mut self.keeper,
                &sources,
                position,
                Some(text),
                &per_series,
            )?;
        }
        Ok(())
    }

    /// `remove_category`.
    fn remove_category(&mut self, address: &str, index: usize) -> Result<(), EditError> {
        let count = self.chart.point_count();
        if index >= count {
            return Err(refused(&format!("{address}: no category {index} (have {count})")));
        }
        self.ran = true;
        if count <= 1 {
            return Err(refused(&format!("{address}: a chart keeps at least one category")));
        }
        let sources = all_sources(&self.chart)?;
        let block = Block::find(address, &sources, &mut self.keeper, count)?;
        self.chart.delete_category(index).map_err(data_refusal)?;
        self.changed = true;
        if let Some(block) = block {
            let index = u32::try_from(index).unwrap_or(u32::MAX);
            block.delete_line(&mut self.chart, &mut self.keeper, &sources, index)?;
        }
        Ok(())
    }

    /// `add_series` (last): a copy of the last series with its own values, given the next
    /// free line of the workbook -- or, without one to keep in step, literal data.
    fn add_series(
        &mut self,
        address: &str,
        part: &str,
        text: &str,
        values: Option<&[Option<f64>]>,
    ) -> Result<(), EditError> {
        self.ran = true;
        let series = self.chart.series_count();
        if series == 0 {
            return Err(refused(&format!(
                "{address}: a chart with no series has no template to add one from"
            )));
        }
        let count = self.chart.point_count();
        let values = values.map_or_else(|| vec![None; count], <[Option<f64>]>::to_vec);
        if values.len() != count {
            return Err(refused(&format!(
                "{address}: {} values for {count} categories",
                values.len()
            )));
        }
        let sources = all_sources(&self.chart)?;
        let block = Block::find(address, &sources, &mut self.keeper, count)?;
        self.chart
            .append_series(Some(text), &values, part, &ooxml_word_model::ids::sha256)
            .map_err(data_refusal)?;
        self.changed = true;
        match block {
            Some(block) => {
                block.append_series(
                    address,
                    &mut self.chart,
                    &mut self.keeper,
                    &sources,
                    series,
                    Some(text),
                    &values,
                )?;
            }
            None => {
                self.chart.name_to_literal(series, Some(text));
                self.chart.to_literal(series, Which::Values);
                self.chart.to_literal(series, Which::Sizes);
            }
        }
        Ok(())
    }

    /// `remove_series`: from the workbook first, then from the chart with its legend entries.
    fn remove_series(&mut self, address: &str, series: usize) -> Result<(), EditError> {
        self.ran = true;
        if self.chart.series_count() <= 1 {
            return Err(refused(&format!("{address}: a chart keeps at least one series")));
        }
        let sources = all_sources(&self.chart)?;
        let count = self.chart.point_count();
        if let Some(block) = Block::find(address, &sources, &mut self.keeper, count)? {
            block.delete_series(&mut self.chart, &mut self.keeper, &sources, series)?;
        }
        self.chart.delete_series(series);
        self.changed = true;
        Ok(())
    }

    /// `set_data_labels`, its number format checked.
    fn data_labels(
        &mut self,
        address: &str,
        show: bool,
        series: Option<usize>,
        look: &LabelLook<'_>,
    ) -> Result<(), EditError> {
        if let Some(format) = look.number_format {
            if format.trim_matches(ooxml_xml::view::py_space).is_empty() || format.chars().count() > 255 {
                return Err(refused(&format!(
                    "{address}: a number format is a format code, such as #,##0.0"
                )));
            }
            if !show {
                return Err(refused(&format!("{address}: a number format is for labels shown")));
            }
        }
        let chosen: Vec<usize> = match series {
            Some(series) => vec![series],
            None => (0..self.chart.series_count()).collect(),
        };
        self.ran = true;
        self.chart.set_data_labels(show, &chosen, look);
        self.changed = true;
        Ok(())
    }

    /// `set_title` (no `axis`) or `set_axis_title`: a title drawn from a cell has its cell
    /// written too.
    fn title(&mut self, axis: Option<&str>, text: Option<&str>, look: &TitleLook<'_>) -> Result<(), EditError> {
        let done = match axis {
            None => self.chart.set_title(text, look),
            Some(axis) => self
                .chart
                .set_axis_title(axis, text, look)
                .map(|found| found.unwrap_or_else(|| Ok(TitleEdit::default()))),
        };
        let edit = done
            .map_err(|_| {
                unported("a new title in a document whose defaults name no language (the session's locale decides)")
            })?
            .map_err(data_refusal)?;
        if edit.changed {
            self.ran = true;
            self.changed = true;
        }
        if let (Some(source), Some(text)) = (edit.written, text) {
            self.keeper
                .write(Some(&source), 0, &PointValue::Text(text.to_owned()))?;
        }
        Ok(())
    }

    /// `set_gap_width`, past the handler's whole-number check.
    fn gap_width(&mut self, address: &str, percent: i64) -> Result<(), EditError> {
        if !(0..=500).contains(&percent) {
            return Err(refused(&format!("{address}: a gap width is a whole percentage, 0-500")));
        }
        if self.chart.gap_width().is_none() {
            return Err(refused(&format!(
                "{address}: only bar and column charts have a gap width"
            )));
        }
        if self.chart.set_gap_width(percent) {
            self.ran = true;
            self.changed = true;
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

fn data_refusal(error: DataError) -> EditError {
    refused(&error.0)
}
