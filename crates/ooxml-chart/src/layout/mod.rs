//! Owns: a chart laid out in its frame and lowered to drawable elements (`chart/layout.py`):
//! the measured constants ([`consts`]), chart text measurement ([`text`]), number writing
//! ([`numbers`]), value axis scales ([`scale`]), the 3-D scene ([`three_d`]), path data
//! ([`paths`]), what the layout reads off the chart's text and axes ([`sources`]), the data
//! it reports ([`data`]), and the builder that does the laying out ([`builder`]).
//!
//! Must not: write SVG (a scene renderer draws the elements), or resolve a theme.

mod builder;
pub mod consts;
pub mod data;
pub mod numbers;
pub mod paths;
pub mod scale;
pub mod sources;
pub mod text;
pub mod three_d;

pub use builder::{chart_text_body, chart_wrapped_text_body, ChartInput, ChartStyle, Laid, Resolve};
pub use data::{AxisScale, ChartData, ChartSeries};

use crate::model::{flat_chart_kind, Chart, Plot};
use consts::{ACCENT_KEYS, DRAWABLE_CHART_KINDS};
use ooxml_drawingml::drawable::Colour;

/// The plots the layout can draw, in document order.
pub fn drawable_plots(chart: &Chart) -> Vec<&Plot> {
    chart
        .plots
        .iter()
        .filter(|p| DRAWABLE_CHART_KINDS.contains(&flat_chart_kind(&p.kind)))
        .collect()
}

/// The accents a theme resolves, in order (those that resolve).
pub fn accent_colours(resolve: impl Fn(&str) -> Option<Colour>) -> Vec<Colour> {
    ACCENT_KEYS.iter().filter_map(|key| resolve(key)).collect()
}

/// Whether a part was written by Office 2007 (its `AppVersion` is 12).
pub fn written_by_office_2007(app_version: Option<&str>) -> bool {
    app_version
        .filter(|v| !v.is_empty())
        .is_some_and(|v| text::py_strip(v).split('.').next() == Some("12"))
}

/// One chart laid out.
pub fn lay_out(input: &ChartInput<'_>) -> Laid {
    builder::Builder::new(input).build()
}
