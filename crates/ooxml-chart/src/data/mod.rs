//! Owns: a chart part's data and labels as ooxml-edit's `chart_model` reads them -- the
//! plot types, the title, the axis titles, whether there is a legend, the values' format
//! code, the categories (and, where they are dates kept as serials, those dates), and each
//! series' name and values -- and the one-line summary docx-agent and pptx-agent write of it
//! (`column; title: Sales; series: North, South; categories: Q1, Q2`).
//!
//! Beside them, what ooxml-edit's `Chart` reads for `edit_chart`'s `read` beyond the model:
//! how blanks draw, the gap width, dot charts, number formats, axis names and scales and a
//! series' own look (`look`), and its point labels (`labels`); and each series' references,
//! for a reader of the chart's workbook (`references`).
//!
//! **Why a second view beside [`crate::read`].** The render model reads what a chart looks
//! like, as ooxml-common's `chart/read.py` does; this reads what an editor shows of its data,
//! as ooxml-edit does, and the two differ where their Python sources differ:
//! - values keep how they were written: `3` is an integer and `3.0` a float in the JSON
//!   (the render model holds every value as a float);
//! - a source's length is its `c:ptCount` where that is digits, else its highest index plus
//!   one (the render model takes the larger of the two);
//! - a series' name is the first point of its `c:tx` reference, a number where the
//!   reference is numeric (the render model keeps its cached string);
//! - categories are the first series' with a category reference, each point as written,
//!   text unstripped (the render model reads any of four cache spellings into strings).
//!
//! The format code inside `mc:AlternateContent` is found by the walk both share
//! (`read::cache::alternate_format_codes`); the plot and axis element names are the model's.
//!
//! Must not: lay anything out, resolve styling, or know which document format holds the
//! chart.

mod json;
mod labels;
mod look;
mod references;
mod serial;
mod source;

pub use json::{point_json, points_json};
pub use labels::point_labels;
pub(crate) use look::axis_named;
pub use look::{
    axis_names, axis_scale, blanks, gap_width, is_dot, number_formats, series_look, Look, NumberFormats, Said, Scale,
    SeriesLook,
};
pub use references::{series_elements, series_references, Reference, SeriesReferences};
pub use serial::iso_of_serial;
pub use source::{decode_number, Point, Unreadable};

use crate::model::{AXIS_ELEMENTS, CHART_GROUP_ELEMENTS};
use crate::read::cache::alternate_format_codes;
use ooxml_drawingml::read::text::paragraph_text;
use ooxml_numfmt::is_date_code;
use ooxml_xml::view::{attr, child, children, elements, local, py_flat as flat, text};
use ooxml_xml::Element;
pub(crate) use source::Source;

/// One series' name and values.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesData {
    /// The first point of its `c:tx`, where it has a reference.
    pub name: Option<Point>,
    /// Its values, `None` for a blank.
    pub values: Vec<Option<Point>>,
}

/// A chart's data and labels (ooxml-edit's `chart_model`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChartData {
    /// Each plot's type, in the plot area's order (`bar`, `line`, `pie3D`...).
    pub types: Vec<String>,
    /// The chart title's text.
    pub title: Option<Point>,
    /// `category` and `value`, for each axis the chart has: its title's text.
    pub axis_titles: Vec<(&'static str, Option<Point>)>,
    /// Whether there is a legend.
    pub legend: bool,
    /// The first series' values' format code.
    pub format: Option<String>,
    /// The categories, `None` for a blank.
    pub categories: Vec<Option<Point>>,
    /// The series, plot by plot.
    pub series: Vec<SeriesData>,
    /// Where the categories are dates kept as serials: each as an ISO date.
    pub category_dates: Option<Vec<Option<String>>>,
    /// Where `category_dates` is given: the categories' format code.
    pub category_format: Option<String>,
}

/// One `c:ser` and its sources.
pub(crate) struct Ser<'a> {
    pub(crate) name: Source<'a>,
    pub(crate) categories: Source<'a>,
    pub(crate) values: Source<'a>,
}

impl<'a> Ser<'a> {
    pub(crate) fn of(ser: &'a Element) -> Ser<'a> {
        let either = |a: &str, b: &str| child(ser, a).or_else(|| child(ser, b));
        Ser {
            name: Source::of(child(ser, "tx")),
            categories: Source::of(either("cat", "xVal")),
            values: Source::of(either("val", "yVal")),
        }
    }
}

/// The chart part `root` (`c:chartSpace`) read as ooxml-edit reads it.
///
/// # Errors
/// [`Unreadable`] where a data source holds more points than the model reads.
pub fn chart_data(root: &Element) -> Result<ChartData, Unreadable> {
    let plot_area = child(root, "chart").and_then(|c| child(c, "plotArea"));
    let plots: Vec<&Element> = plot_area
        .into_iter()
        .flat_map(elements)
        .filter(|e| CHART_GROUP_ELEMENTS.contains(&local(e)))
        .collect();
    let series: Vec<Ser<'_>> = plots.iter().flat_map(|p| children(p, "ser")).map(Ser::of).collect();
    let with_categories = series.iter().find(|s| s.categories.is_some());
    let categories = with_categories.map_or(Ok(Vec::new()), |s| s.categories.points())?;
    let format = series
        .iter()
        .find_map(|s| s.values.cache().and_then(|c| child(c, "formatCode")))
        .map(text);
    let mut axis_titles = Vec::new();
    for axis in ["category", "value"] {
        if let Some(node) = plot_area.and_then(|p| axis_of(p, axis)) {
            axis_titles.push((axis, title_text(child(node, "title"))?));
        }
    }
    let mut named = Vec::with_capacity(series.len());
    for ser in &series {
        let name = if ser.name.is_some() {
            ser.name.points()?.into_iter().next().flatten()
        } else {
            None
        };
        named.push(SeriesData {
            name,
            values: ser.values.points()?,
        });
    }
    let category_format = with_categories.and_then(|s| category_format(&s.categories));
    let category_dates = is_date_code(category_format.as_deref(), None).then(|| {
        categories
            .iter()
            .map(|c| c.as_ref().and_then(Point::number).and_then(iso_of_serial))
            .collect()
    });
    Ok(ChartData {
        types: plots.iter().map(|p| plot_type(local(p)).to_owned()).collect(),
        title: title_text(child(root, "chart").and_then(|c| child(c, "title")))?,
        axis_titles,
        legend: child(root, "chart").and_then(|c| child(c, "legend")).is_some(),
        format,
        categories,
        series: named,
        category_format: category_dates.as_ref().and(category_format),
        category_dates,
    })
}

/// `barChart` -> `bar`, `pie3DChart` -> `pie3D`.
fn plot_type(name: &str) -> &str {
    name.strip_suffix("Chart").unwrap_or(name)
}

/// The category (x) or value (y) axis: a scatter or bubble chart's x is a value axis too,
/// the one at the bottom or top.
pub(crate) fn axis_of<'a>(plot_area: &'a Element, axis: &str) -> Option<&'a Element> {
    let axes: Vec<&Element> = elements(plot_area)
        .filter(|e| AXIS_ELEMENTS.contains(&local(e)))
        .collect();
    let mut categorical: Vec<&Element> = axes
        .iter()
        .copied()
        .filter(|a| matches!(local(a), "catAx" | "dateAx"))
        .collect();
    let mut values: Vec<&Element> = axes.iter().copied().filter(|a| local(a) == "valAx").collect();
    if categorical.is_empty() && values.len() >= 2 {
        let horizontal = values
            .iter()
            .copied()
            .find(|a| matches!(child(a, "axPos").and_then(|p| attr(p, "val")), Some("b" | "t")));
        let first = horizontal.or_else(|| values.first().copied());
        categorical = first.into_iter().collect();
        values.retain(|a| !first.is_some_and(|f| std::ptr::eq(*a, f)));
    }
    let chosen = if axis == "category" { categorical } else { values };
    chosen.first().copied()
}

/// A title's text: its rich text's paragraphs, one per line, else the first point of its
/// reference.
fn title_text(title: Option<&Element>) -> Result<Option<Point>, Unreadable> {
    let Some(tx) = title.and_then(|t| child(t, "tx")) else {
        return Ok(None);
    };
    if let Some(rich) = child(tx, "rich") {
        let lines: Vec<String> = children(rich, "p").map(paragraph_text).collect();
        return Ok(Some(Point::Text(lines.join("\n"))));
    }
    Ok(Source::of(Some(tx)).points()?.into_iter().next().flatten())
}

/// The categories' number format code, where they are numbers with a cache: its
/// `c:formatCode`, else the first Office wrote inside `mc:AlternateContent`.
fn category_format(categories: &Source<'_>) -> Option<String> {
    let cache = categories.cache().filter(|_| categories.numeric())?;
    child(cache, "formatCode")
        .or_else(|| alternate_format_codes(cache).next())
        .map(text)
}

impl ChartData {
    /// One line: `column; title: Sales; series: North, South; categories: Q1, Q2`, dates
    /// kept as serials listed as their ISO dates (docx-agent's `chart_summary`).
    #[must_use]
    pub fn summary(&self) -> String {
        let mut types: Vec<&str> = Vec::new();
        for name in &self.types {
            if !types.contains(&name.as_str()) {
                types.push(name);
            }
        }
        let mut words = vec![if types.is_empty() {
            "chart".to_owned()
        } else {
            types.join(" + ")
        }];
        if let Some(title) = self.title.as_ref().filter(|t| t.truthy()) {
            words.push(format!("title: {}", flat(&title.text())));
        }
        if !self.series.is_empty() {
            let names: Vec<String> = self
                .series
                .iter()
                .map(|s| {
                    s.name
                        .as_ref()
                        .filter(|n| n.truthy())
                        .map(|n| flat(&n.text()))
                        .unwrap_or_default()
                })
                .collect();
            words.push(format!("series: {}", names.join(", ")));
        }
        let shown: Vec<String> = match self.category_dates.as_ref().filter(|d| !d.is_empty()) {
            Some(dates) => dates
                .iter()
                .zip(&self.categories)
                .map(|(date, category)| date.clone().unwrap_or_else(|| shown(category.as_ref())))
                .collect(),
            None => self.categories.iter().map(|c| shown(c.as_ref())).collect(),
        };
        if !shown.is_empty() {
            let shown: Vec<String> = shown.iter().map(|c| flat(c)).collect();
            words.push(format!("categories: {}", shown.join(", ")));
        }
        words.join("; ")
    }
}

/// A category as the summary shows it: blank as empty.
fn shown(category: Option<&Point>) -> String {
    category.map(Point::text).unwrap_or_default()
}

#[cfg(test)]
mod look_tests;
#[cfg(test)]
mod tests;
