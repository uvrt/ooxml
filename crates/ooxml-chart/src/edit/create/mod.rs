//! Owns: a new chart from data, as ooxml-edit's `charts/create.py` makes one -- [`NewChart`],
//! checked as `chart_data` and `check_options` check it ([`NewChart::check`]: each refusal in
//! its words and order), its chart part (`c:chartSpace`, [`NewChart::chart_space`]: the look
//! Office gives a new chart of each kind in the document's theme, measured there) and the
//! workbook columns its data goes in ([`NewChart::data_columns`]).
//!
//! The data's layout is Office's: categories in column A from row 2, each series' name in
//! row 1 and its values below it (a bubble chart's sizes in the next column), every
//! series' formulas pointing there, so the caches and the cells are equal from the start.
//!
//! Must not: know a document format (the frame around the chart, the parts it goes in, the
//! workbook's bytes are the caller's), or take dates as categories (ooxml-edit's
//! `DateCategory`: a later step).

mod parts;
mod space;

use super::DataError;

/// The kinds of chart a new one is (`CHART_KINDS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewKind {
    /// Clustered columns.
    Column,
    /// Stacked columns.
    StackedColumn,
    /// Clustered bars.
    Bar,
    /// Stacked bars.
    StackedBar,
    /// Lines.
    Line,
    /// A pie.
    Pie,
    /// Points by x and y.
    Scatter,
    /// A radar of lines.
    Radar,
    /// A filled radar.
    RadarFilled,
    /// Bubbles.
    Bubble,
    /// A marker per category, no line: a line chart whose lines are off.
    Dot,
}

/// A plot's element and its settings (`CHART_KINDS`: plot, bar direction, grouping).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Plot {
    Bar,
    Line,
    Pie,
    Scatter,
    Radar,
    Bubble,
}

impl NewKind {
    /// The kind as ooxml-edit names it (`stacked_column`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            NewKind::Column => "column",
            NewKind::StackedColumn => "stacked_column",
            NewKind::Bar => "bar",
            NewKind::StackedBar => "stacked_bar",
            NewKind::Line => "line",
            NewKind::Pie => "pie",
            NewKind::Scatter => "scatter",
            NewKind::Radar => "radar",
            NewKind::RadarFilled => "radar_filled",
            NewKind::Bubble => "bubble",
            NewKind::Dot => "dot",
        }
    }

    /// Its plot, bar direction and grouping (`CHART_KINDS`).
    pub(crate) fn plot(self) -> (Plot, Option<&'static str>, Option<&'static str>) {
        match self {
            NewKind::Column => (Plot::Bar, Some("col"), Some("clustered")),
            NewKind::StackedColumn => (Plot::Bar, Some("col"), Some("stacked")),
            NewKind::Bar => (Plot::Bar, Some("bar"), Some("clustered")),
            NewKind::StackedBar => (Plot::Bar, Some("bar"), Some("stacked")),
            NewKind::Line | NewKind::Dot => (Plot::Line, None, Some("standard")),
            NewKind::Pie => (Plot::Pie, None, None),
            NewKind::Scatter => (Plot::Scatter, None, None),
            NewKind::Radar => (Plot::Radar, None, Some("marker")),
            NewKind::RadarFilled => (Plot::Radar, None, Some("filled")),
            NewKind::Bubble => (Plot::Bubble, None, None),
        }
    }

    /// Whether its categories are x values, numbers (`XY_KINDS`).
    #[must_use]
    pub fn is_xy(self) -> bool {
        matches!(self, NewKind::Scatter | NewKind::Bubble)
    }

    /// Whether a series of it may be drawn as a line (`COMBO_KINDS`).
    #[must_use]
    pub fn combines(self) -> bool {
        matches!(self, NewKind::Column | NewKind::StackedColumn)
    }

    /// Where Office puts a new chart's legend (`default_legend`): a radar's at the top.
    fn default_legend(self) -> Legend {
        if matches!(self, NewKind::Radar | NewKind::RadarFilled) {
            Legend::Top
        } else {
            Legend::Bottom
        }
    }
}

/// A new chart's categories: text labels, or a scatter or bubble chart's x values.
#[derive(Debug, Clone, PartialEq)]
pub enum Labels {
    /// Text labels.
    Text(Vec<String>),
    /// X values.
    Numbers(Vec<f64>),
}

impl Labels {
    fn len(&self) -> usize {
        match self {
            Labels::Text(labels) => labels.len(),
            Labels::Numbers(labels) => labels.len(),
        }
    }
}

/// A new series: its name, a value per category (`None` a blank), whether a column chart
/// draws it as a line, and a bubble chart's sizes.
#[derive(Debug, Clone, PartialEq)]
pub struct NewSeries {
    /// Its name (`None`: no `c:tx`).
    pub name: Option<String>,
    /// One value per category.
    pub values: Vec<Option<f64>>,
    /// `type` given: `Some(true)` a line, `Some(false)` columns.
    pub line: Option<bool>,
    /// A bubble chart's sizes, one per point.
    pub size: Option<Vec<Option<f64>>>,
}

/// Where the legend goes (`LEGEND_POSITIONS`, or Office's default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Legend {
    /// Office's: the bottom, a radar's top.
    Default,
    /// Below the plot.
    Bottom,
    /// Right of it.
    Right,
    /// Above it.
    Top,
    /// Left of it.
    Left,
}

impl Legend {
    fn code(self) -> &'static str {
        match self {
            Legend::Default | Legend::Bottom => "b",
            Legend::Right => "r",
            Legend::Top => "t",
            Legend::Left => "l",
        }
    }
}

/// What differs between the applications' new charts (`ChartLook`): text sizes in
/// hundredths of a point, and whether the chart area is filled and outlined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChartLook {
    /// The chart's title.
    pub title: u32,
    /// Axis tick labels and the legend.
    pub labels: u32,
    /// The chart area: the background colour and a light border.
    pub framed: bool,
}

/// Word's new chart (`WORD_LOOK`, measured): a 14 pt title, 9 pt labels, a framed area.
pub const WORD_LOOK: ChartLook = ChartLook {
    title: 1400,
    labels: 900,
    framed: true,
};

/// A new chart.
#[derive(Debug, Clone, PartialEq)]
pub struct NewChart {
    /// Its kind.
    pub kind: NewKind,
    /// Its categories (x values).
    pub labels: Labels,
    /// Its series.
    pub series: Vec<NewSeries>,
    /// Its title.
    pub title: Option<String>,
    /// Its legend (`None`: none).
    pub legend: Option<Legend>,
    /// The values' Excel format code.
    pub number_format: Option<String>,
    /// The application's look.
    pub look: ChartLook,
    /// The text's language, and the chart's (`c:lang`).
    pub lang: String,
    /// The categories listed in reverse order (`category_order="reversed"`).
    pub reversed: bool,
    /// The value axis' major gridlines.
    pub gridlines: bool,
}

/// At most this many series (`chart_data`).
const MOST_SERIES: usize = 255;

impl NewChart {
    /// Whether some series is drawn as a line in a column chart (a combination chart).
    fn combo(&self) -> bool {
        self.series.iter().any(|s| s.line == Some(true))
    }

    /// The format code of the values (`number_format or "General"`).
    pub(crate) fn format_code(&self) -> &str {
        self.number_format
            .as_deref()
            .filter(|f| !f.is_empty())
            .unwrap_or("General")
    }

    /// The legend's position code, where there is one.
    fn legend_code(&self) -> Option<&'static str> {
        self.legend.map(|legend| match legend {
            Legend::Default => self.kind.default_legend().code(),
            other => other.code(),
        })
    }

    /// The data checked as `chart_data` and `check_options` check it, before anything is
    /// made.
    ///
    /// # Errors
    /// The first refusal, in ooxml-edit's words.
    pub fn check(&self) -> Result<(), DataError> {
        let refuse = |text: String| Err(DataError(text));
        for (index, series) in self.series.iter().enumerate() {
            if series.size.is_some() && self.kind != NewKind::Bubble {
                return refuse(format!(
                    "series {index}: unknown key 'size'; a series is {{'name', 'type', 'values'}}"
                ));
            }
        }
        let count = self.labels.len();
        if count == 0 {
            return refuse("a chart needs at least one category".to_owned());
        }
        if self.kind.is_xy() != matches!(self.labels, Labels::Numbers(_)) {
            return refuse(format!("a {} chart's categories are its x values", self.kind.name()));
        }
        for (index, series) in self.series.iter().enumerate() {
            self.check_series(index, series, count)?;
        }
        if self.series.is_empty() {
            return refuse("a chart needs at least one series".to_owned());
        }
        if self.kind == NewKind::Pie && self.series.len() > 1 {
            return refuse("a pie chart shows one series".to_owned());
        }
        if self.series.len() > MOST_SERIES {
            return refuse(format!("at most {MOST_SERIES} series"));
        }
        if self.series.iter().all(|s| s.line == Some(true)) {
            return refuse("every series is a line: make it a line chart".to_owned());
        }
        if self.reversed && self.kind == NewKind::Pie {
            return refuse("a pie chart has no category axis to reverse".to_owned());
        }
        Ok(())
    }

    fn check_series(&self, index: usize, series: &NewSeries, count: usize) -> Result<(), DataError> {
        let refuse = |text: String| Err(DataError(text));
        if series.values.len() != count {
            return refuse(format!(
                "series {index} has {} values for {count} categories",
                series.values.len()
            ));
        }
        match series.line {
            Some(true) if !self.kind.combines() => {
                let bars = if self.kind.name().contains("bar") {
                    "; PowerPoint cannot combine horizontal bars with a line"
                } else {
                    ""
                };
                return refuse(format!(
                    "series {index}: only a column chart draws a series as a line{bars}"
                ));
            }
            Some(false) if !self.kind.combines() => {
                return refuse(format!("series {index}: type 'column' is for a column chart"));
            }
            _ => {}
        }
        if self.kind == NewKind::Bubble {
            let Some(size) = &series.size else {
                return refuse(format!(
                    "series {index}: a bubble chart's series give size, one per point"
                ));
            };
            if size.len() != count {
                return refuse(format!("series {index} has {} sizes for {count} points", size.len()));
            }
            if size.iter().flatten().any(|v| *v < 0.0) {
                return refuse(format!("series {index}: a bubble size is 0 or more"));
            }
        }
        Ok(())
    }

    /// Each series' workbook columns, from 1 (`data_columns`): its values beside the
    /// categories (column A), a bubble chart's sizes right of its values.
    #[must_use]
    pub fn data_columns(&self) -> Vec<(u32, Option<u32>)> {
        (0..self.series.len())
            .map(|k| {
                let k = u32::try_from(k).unwrap_or(u32::MAX / 4);
                if self.kind == NewKind::Bubble {
                    (2 + 2 * k, Some(3 + 2 * k))
                } else {
                    (2 + k, None)
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
