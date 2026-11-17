//! Owns: what a laid-out chart plots, independent of how it was drawn (`ChartData` and its
//! series and value axis, `drawingml/scene.py`'s chart types), for a caller that reads the
//! data back.
//!
//! Must not: hold anything drawn.

use crate::model::View3D;
use ooxml_drawingml::drawable::Colour;

/// One plotted series, its blanks kept.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartSeries {
    /// Its name.
    pub name: Option<String>,
    /// Its values, `None` for a blank.
    pub values: Vec<Option<f64>>,
    /// Its categories.
    pub categories: Vec<String>,
    /// Its colour.
    pub colour: Option<Colour>,
    /// Its values' format code.
    pub format_code: Option<String>,
}

/// The value axis as drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisScale {
    /// The minimum.
    pub minimum: f64,
    /// The maximum.
    pub maximum: f64,
    /// The major unit.
    pub major_unit: f64,
}

/// What a chart plots.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartData {
    /// The (flattened) kind.
    pub kind: String,
    /// The series.
    pub series: Vec<ChartSeries>,
    /// The categories.
    pub categories: Vec<String>,
    /// The title shown.
    pub title: Option<String>,
    /// `c:grouping`.
    pub grouping: Option<String>,
    /// `c:barDir` (`col` where unstated, on a cartesian chart).
    pub bar_direction: Option<String>,
    /// The value axis drawn.
    pub value_axis: Option<AxisScale>,
    /// The legend's position.
    pub legend_position: Option<String>,
    /// Whether any plot is a 3-D spelling.
    pub three_d: bool,
    /// The camera as stated.
    pub view_3d: Option<View3D>,
}
