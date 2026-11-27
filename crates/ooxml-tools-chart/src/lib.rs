//! The chart tools (docs/architecture/README.md, section 3 `ooxml-tools-*`; layer 4.2, under
//! each format's tools).
//!
//! Owns: `edit_chart`'s definition ([`EditChart`], its description as a Markdown file
//! beside it) and its handlers over the format whose document holds the chart: a read over
//! a [`GraphicHost`] (the version the call pins), a change over a [`GraphicWriter`] (the
//! call's editor), each a function of the typed arguments and the host; what a chart
//! holds and how it looks as the reply gives it; the data and style actions planned on a
//! chart's data and applied to its parts, its embedded workbook kept in step and its record
//! of where its values came from kept true; and a new chart made and finished.
//!
//! Must not: know a format's model, frames or store (each format's tools crate implements
//! the hosts and registers the tool), hold sessions, or keep state between calls.
#![forbid(unsafe_code)]

mod add;
mod add_plan;
mod args;
mod block;
mod data;
mod edit_chart;
mod fold;
mod host;
mod keeper;
mod names;
mod read;
mod record;
mod style;
mod workbook;

pub use add::ChartEdit;

pub use args::{
    Blanks, ChartAction, ChartAxis, ChartBox, ChartPosition, ChartType, Dash, EditChart, Gridlines, Marker, ScaleArg,
    ScaleKey, SeriesArg, SeriesType, StyleArg, TextPart,
};
pub use data::{ChartDataEdit, DataAction};
pub use edit_chart::{edit_chart, edit_chart_read, planned, planner};
pub use host::{FinishGraphic, Found, Graphic, GraphicHost, GraphicWriter, Inserted, NewGraphic, NoChart, Written};
pub use names::{category_names, series_names, Stop};
pub use style::ChartStyleEdit;

/// What a chart edit says beside its result, one line each.
pub type Warnings = Vec<String>;
