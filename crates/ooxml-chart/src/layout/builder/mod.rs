//! Owns: the chart builder -- one chart part laid out in its frame and lowered to drawable
//! elements, `chart/layout.py`'s `ChartBuilder` -- split by concern: the series and fonts it
//! reads ([`series`]), the frame's bands, title and plot rectangle ([`frame`]), pies and
//! of-pies ([`polar`]), radars ([`radar`]), cartesian charts ([`cartesian`]), scatter and bubble
//! charts ([`scatter`]), marks, gridlines and axes ([`marks`]), the 3-D scene ([`scene`]),
//! labels ([`labels`]), the legend
//! ([`legend`]) and the primitives every part draws with ([`primitives`]).
//!
//! A combination chart draws each of its plots as a *group*: the same builder with that
//! plot and its own per-plot state swapped in ([`GroupState`], [`Builder::in_group`]), the
//! elements shared -- as the Python's shallow copies share their list.
//!
//! Must not: write SVG, resolve a theme (the [`Resolve`] port does), or hold state beyond one
//! chart.

mod cartesian;
mod frame;
mod labels;
mod legend;
mod marks;
mod polar;
mod primitives;
mod radar;
mod scatter;
mod scene;
mod series;

pub use primitives::{chart_text_body, chart_wrapped_text_body};

use super::data::{AxisScale, ChartData, ChartSeries};
use super::scale::Scale;
use super::text::{ChartFont, FontBox};
use super::three_d::{Rect, Scene};
use crate::model::{flat_chart_kind, is_three_d_kind, Axis, Chart, ChartText, Plot};
use crate::rules::ChartRules;
use ooxml_drawingml::drawable::{Colour, Element, Fill, Outline, TextBody};
use ooxml_drawingml::source;
use ooxml_drawingml::text::RunProperties as SourceRun;
use ooxml_numfmt::Locale;
use std::collections::BTreeMap;

/// What a chart's consumer resolves: fills, outlines and titles in its theme and colour
/// map, and a theme typeface (`+mn-lt`) to a face.
pub trait Resolve {
    /// A stated fill in concrete colours; `None` for none or one that cannot be drawn.
    fn fill(&self, fill: Option<&source::Fill>) -> Option<Fill>;
    /// A stated outline in concrete colours.
    fn outline(&self, outline: Option<&source::Outline>) -> Option<Outline>;
    /// A title's text body: `rich` resolved, else `text` at `size`, aligned `align`.
    fn text(&self, rich: Option<&ooxml_drawingml::text::TextBody>, text: &str, size: f64, align: &str) -> TextBody;
    /// A typeface as drawn (a theme font resolved).
    fn typeface(&self, face: &str) -> Option<String>;
}

/// Everything the drawing needs that is not geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartStyle {
    /// The default face.
    pub font_family: Option<String>,
    /// The default size.
    pub font_size: f64,
    /// The default text colour.
    pub colour: Colour,
    /// The accents, in order.
    pub accents: Vec<Colour>,
    /// The theme's East Asian face.
    pub font_family_ea: Option<String>,
    /// Whether the file was written by Office 2007.
    pub office_2007: bool,
    /// The axis, tick and gridline colour where none is stated.
    pub line_colour: Option<String>,
}

/// One chart to lay out.
pub struct ChartInput<'a> {
    /// The chart.
    pub chart: &'a Chart,
    /// The plot laid out (the first drawable one).
    pub plot: &'a Plot,
    /// Every drawable plot (a combination chart's).
    pub plots: Vec<&'a Plot>,
    /// The frame's width, points.
    pub width_pt: f64,
    /// Its height.
    pub height_pt: f64,
    /// The style.
    pub style: ChartStyle,
    /// The consumer's resolution.
    pub resolve: &'a dyn Resolve,
    /// The application's rules.
    pub rules: &'a ChartRules,
    /// The locale numbers are written in.
    pub locale: &'a Locale,
    /// Word's own "Chart Title" in its interface's language, where known.
    pub default_title: Option<String>,
}

/// A laid-out chart.
#[derive(Debug, Clone, PartialEq)]
pub struct Laid {
    /// Its elements, in the frame's own space.
    pub elements: Vec<Element>,
    /// What it plots.
    pub data: ChartData,
    /// The 3-D kinds drawn flat.
    pub flattened_three_d: Vec<String>,
    /// Whether Word's own title was wanted and not supplied.
    pub default_title_wanted: bool,
    /// Why axis titles were not drawn.
    pub axis_titles_not_drawn: Vec<String>,
}

/// What a data label shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Labels {
    pub show_value: bool,
    pub show_category: bool,
    pub show_series: bool,
    pub show_percent: bool,
    pub show_bubble_size: bool,
    pub position: Option<String>,
    pub number_format: Option<String>,
    pub font: Option<ChartFont>,
    pub colour: Option<Colour>,
    pub text: Option<ChartText>,
    pub show_range: bool,
}

impl Labels {
    /// Whether it shows anything.
    pub fn anything(&self) -> bool {
        self.show_value
            || self.show_category
            || self.show_series
            || self.show_percent
            || self.show_bubble_size
            || self.show_range
            || self.text.is_some()
    }
}

/// One line of a label's own rich text: its runs' text and stated properties.
pub(crate) type RichLine = Vec<(String, Option<SourceRun>)>;

/// A label's text, and its own runs where it states rich text (`_LabelText`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LabelText {
    pub text: String,
    pub lines: Option<Vec<RichLine>>,
}

impl LabelText {
    pub fn plain(text: String) -> LabelText {
        LabelText { text, lines: None }
    }

    pub fn rich(lines: Vec<RichLine>) -> LabelText {
        let text = lines
            .iter()
            .map(|line| line.iter().map(|(t, _)| t.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        LabelText {
            text,
            lines: Some(lines),
        }
    }
}

/// One series as drawn (`_Series`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Series {
    pub name: Option<String>,
    pub values: Vec<Option<f64>>,
    pub colour: Colour,
    pub fill: Option<Fill>,
    pub outline: Option<Outline>,
    pub format_code: Option<String>,
    pub invert_if_negative: bool,
    pub point_fills: BTreeMap<i64, Fill>,
    pub point_outlines: BTreeMap<i64, Outline>,
    pub vary_fills: Vec<Fill>,
    pub line: Option<Outline>,
    pub line_keyed: bool,
    pub marker_symbol: Option<String>,
    pub marker_size: f64,
    pub marker_fill: Option<Fill>,
    pub marker_outline: Option<Outline>,
    pub smooth: bool,
    pub labels: Option<Labels>,
    pub point_labels: BTreeMap<i64, Labels>,
    pub label_range: Vec<Option<String>>,
}

impl Series {
    pub fn new(name: Option<String>, colour: Colour, fill: Option<Fill>, outline: Option<Outline>) -> Series {
        Series {
            name,
            values: Vec::new(),
            colour,
            fill,
            outline,
            format_code: None,
            invert_if_negative: false,
            point_fills: BTreeMap::new(),
            point_outlines: BTreeMap::new(),
            vary_fills: Vec::new(),
            line: None,
            line_keyed: false,
            marker_symbol: None,
            marker_size: super::consts::DEFAULT_MARKER_SIZE_PT,
            marker_fill: None,
            marker_outline: None,
            smooth: false,
            labels: None,
            point_labels: BTreeMap::new(),
            label_range: Vec::new(),
        }
    }

    /// The labels of point `index`: its own, else the series'.
    pub fn labels_at(&self, index: usize) -> Option<&Labels> {
        let key = i64::try_from(index).unwrap_or(i64::MAX);
        self.point_labels.get(&key).or(self.labels.as_ref())
    }

    /// The outline of point `index`: its own, else the series'.
    pub fn outline_at(&self, index: usize) -> Option<Outline> {
        let key = i64::try_from(index).unwrap_or(i64::MAX);
        self.point_outlines.get(&key).cloned().or_else(|| self.outline.clone())
    }

    /// Whether point `index` states its own fill.
    pub fn has_point_fill(&self, index: usize) -> bool {
        i64::try_from(index).is_ok_and(|k| self.point_fills.contains_key(&k))
    }
}

/// A 3-D bar waiting to be painted in depth order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Prism {
    pub order: (f64, f64, f64),
    pub rect: Rect,
    pub fill: Option<Fill>,
    pub outline: Option<Outline>,
    pub base: Colour,
}

/// A resolved title: its body and box.
pub(crate) type Title = (TextBody, FontBox);

/// An axis title on one side: its axis, body and box.
pub(crate) type AxisTitle<'a> = (&'a Axis, TextBody, FontBox);

/// What a plot's builder holds of its own (a Python shallow copy's own attributes).
#[derive(Debug, Clone, Default)]
pub(crate) struct GroupState<'a> {
    pub line_legend_keys: bool,
    pub scene: Option<Scene>,
    pub bands_scale: Option<Scale>,
    pub prisms: Option<Vec<Prism>>,
    pub title: Option<Option<Title>>,
    pub title_hidden: bool,
    pub default_title_wanted: bool,
    pub axis_titles: Option<Vec<(String, AxisTitle<'a>)>>,
}

/// One plot of a combination chart, drawn by the shared builder in turn.
pub(crate) struct Group<'a> {
    pub plot: &'a Plot,
    pub state: GroupState<'a>,
}

/// The chart builder.
pub(crate) struct Builder<'a> {
    pub chart: &'a Chart,
    pub plot: &'a Plot,
    pub plots: Vec<&'a Plot>,
    pub frame: Rect,
    pub style: ChartStyle,
    pub resolve: &'a dyn Resolve,
    pub rules: &'a ChartRules,
    pub locale: &'a Locale,
    pub default_title: Option<String>,
    pub group: GroupState<'a>,
    pub elements: Vec<Element>,
    pub axis_titles_not_drawn: Vec<String>,
}

/// Whether two axis references are the same axis (Python's `is`).
pub(crate) fn same_axis(a: Option<&Axis>, b: Option<&Axis>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => std::ptr::eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

/// `values[index]` or `None` past the end.
pub(crate) fn at(values: &[Option<f64>], index: usize) -> Option<f64> {
    values.get(index).copied().flatten()
}

/// How blanks are drawn: as stated, `zero` where unstated or unknown, and an area's `span`
/// as `zero`.
pub(crate) fn blanks_as(stated: Option<&str>, area: bool) -> &'static str {
    let found = match stated {
        Some("gap") => "gap",
        Some("span") => "span",
        _ => "zero",
    };
    if area && found == "span" {
        "zero"
    } else {
        found
    }
}

/// Each point's total of absolute values across `series`.
pub(crate) fn percent_totals(series: &[Series]) -> Vec<f64> {
    let length = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    (0..length)
        .map(|index| {
            let mut total = 0.0;
            for item in series {
                if let Some(value) = at(&item.values, index) {
                    total += value.abs();
                }
            }
            total
        })
        .collect()
}

impl<'a> Builder<'a> {
    /// A builder for `input`.
    pub fn new(input: &ChartInput<'a>) -> Builder<'a> {
        let plots = if input.plots.is_empty() {
            vec![input.plot]
        } else {
            input.plots.clone()
        };
        Builder {
            chart: input.chart,
            plot: input.plot,
            plots,
            frame: Rect::new(0.0, 0.0, input.width_pt, input.height_pt),
            style: input.style.clone(),
            resolve: input.resolve,
            rules: input.rules,
            locale: input.locale,
            default_title: input.default_title.clone(),
            group: GroupState::default(),
            elements: Vec::new(),
            axis_titles_not_drawn: Vec::new(),
        }
    }

    /// `plot` as a group, with this builder's per-plot state as it stands (`_for_plot`).
    pub fn group_for(&self, plot: &'a Plot) -> Group<'a> {
        Group {
            plot,
            state: self.group.clone(),
        }
    }

    /// `f` run as `group`, its state kept in it after.
    pub fn in_group<R>(&mut self, group: &mut Group<'a>, f: impl FnOnce(&mut Self) -> R) -> R {
        let plot = std::mem::replace(&mut self.plot, group.plot);
        std::mem::swap(&mut self.group, &mut group.state);
        let result = f(self);
        std::mem::swap(&mut self.group, &mut group.state);
        self.plot = plot;
        result
    }

    /// `f` run for `plot` on a throwaway group (`self._for_plot(plot).f()`).
    pub fn for_plot<R>(&mut self, plot: &'a Plot, f: impl FnOnce(&mut Self) -> R) -> R {
        let mut group = self.group_for(plot);
        self.in_group(&mut group, f)
    }

    fn kind(&self) -> &'a str {
        flat_chart_kind(&self.plot.kind)
    }

    pub fn is_polar(&self) -> bool {
        matches!(self.kind(), "pieChart" | "doughnutChart" | "radarChart" | "ofPieChart")
    }

    pub fn is_radar(&self) -> bool {
        self.kind() == "radarChart"
    }

    pub fn is_area(&self) -> bool {
        self.kind() == "areaChart"
    }

    pub fn is_scatter(&self) -> bool {
        matches!(self.kind(), "scatterChart" | "bubbleChart")
    }

    pub fn is_bubble(&self) -> bool {
        self.kind() == "bubbleChart"
    }

    pub fn is_of_pie(&self) -> bool {
        self.kind() == "ofPieChart"
    }

    pub fn is_stock(&self) -> bool {
        self.kind() == "stockChart"
    }

    pub fn is_surface(&self) -> bool {
        self.kind() == "surfaceChart"
    }

    pub fn is_line(&self) -> bool {
        matches!(self.kind(), "lineChart" | "stockChart")
    }

    pub fn is_three_d(&self) -> bool {
        self.plots.iter().any(|p| is_three_d_kind(&p.kind))
    }

    pub fn radar_style(&self) -> &'static str {
        let style = self.plot.radar_style.as_deref().unwrap_or("standard");
        if super::text::py_strip(style) == "filled" {
            "filled"
        } else {
            "marker"
        }
    }

    pub fn horizontal(&self) -> bool {
        self.plot.bar_direction.as_deref().unwrap_or("col") == "bar"
    }

    pub fn grouping_or(&self, default: &'static str) -> &str {
        self.plot.grouping.as_deref().unwrap_or(default)
    }

    pub fn stacked(&self, default: &'static str) -> bool {
        matches!(self.grouping_or(default), "stacked" | "percentStacked")
    }

    /// The chart laid out.
    pub fn build(mut self) -> Laid {
        let mut data = if self.is_radar() {
            self.build_radar()
        } else if self.is_of_pie() {
            self.build_polar(true)
        } else if self.is_polar() {
            self.build_polar(false)
        } else if self.is_scatter() {
            self.build_scatter()
        } else {
            self.build_cartesian()
        };
        data.three_d = self.is_three_d();
        data.view_3d = self.chart.view_3d.clone();
        let mut flattened: Vec<String> = Vec::new();
        for plot in self.drawn_plots() {
            if is_three_d_kind(&plot.kind) && !self.for_plot(plot, |b| b.draws_a_scene()) {
                flattened.push(plot.kind.clone());
            }
        }
        flattened.sort();
        flattened.dedup();
        Laid {
            elements: self.elements,
            data,
            flattened_three_d: flattened,
            default_title_wanted: self.group.default_title_wanted,
            axis_titles_not_drawn: self.axis_titles_not_drawn,
        }
    }

    /// `ChartData` for `series`, with this chart's title and legend.
    pub fn chart_data(
        &mut self,
        series: &[Series],
        categories_of: Option<Vec<Vec<String>>>,
        value_axis: Option<Scale>,
    ) -> ChartData {
        let shared = self.categories(series, false);
        let items = series
            .iter()
            .enumerate()
            .map(|(index, item)| ChartSeries {
                name: item.name.clone(),
                values: item.values.clone(),
                categories: categories_of
                    .as_ref()
                    .and_then(|c| c.get(index).cloned())
                    .unwrap_or_else(|| shared.clone()),
                colour: Some(item.colour.clone()),
                format_code: item.format_code.clone(),
            })
            .collect();
        ChartData {
            kind: flat_chart_kind(&self.plot.kind).to_owned(),
            series: items,
            categories: if categories_of.is_some() { Vec::new() } else { shared },
            title: self.title_text(),
            grouping: self.plot.grouping.clone(),
            bar_direction: None,
            value_axis: value_axis.map(|s| AxisScale {
                minimum: s.minimum,
                maximum: s.maximum,
                major_unit: s.unit,
            }),
            legend_position: self.legend_position().map(str::to_owned),
            three_d: false,
            view_3d: None,
        }
    }
}
