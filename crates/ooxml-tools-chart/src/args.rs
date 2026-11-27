//! Owns: `edit_chart`'s definition (ooxml-edit's `tools/shared.py`, `edit_chart`, as
//! docx-agent serves it): its arguments, byte-equal to today's, and which of its calls only
//! read (`action: "read"`, today's `reads=_reading_action`).
//!
//! Must not: change the document but through a command.

use ooxml_tools::{ArgEnum, PyNumber, ReadsWhen, ToolArgs};

/// What `edit_chart` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartAction {
    /// Read the chart: changes nothing.
    Read,
    /// A new chart.
    Add,
    /// One value per category of a series.
    SetValues,
    /// One value.
    SetValue,
    /// A new category.
    AddCategory,
    /// Remove a category.
    RemoveCategory,
    /// Rename a category.
    RenameCategory,
    /// A new series.
    AddSeries,
    /// Remove a series.
    RemoveSeries,
    /// Rename a series.
    RenameSeries,
    /// The chart's title.
    SetTitle,
    /// An axis' title.
    SetAxisTitle,
    /// The legend.
    SetLegend,
    /// Values as labels.
    ShowDataLabels,
    /// No value labels.
    HideDataLabels,
    /// The gap between bars.
    SetGapWidth,
    /// An axis' scale.
    SetAxisScale,
    /// Which gridlines.
    SetGridlines,
    /// A series' look.
    FormatSeries,
    /// One point's look.
    FormatPoint,
    /// A series drawn as a line or columns.
    SetSeriesType,
    /// The text's font.
    SetTextStyle,
    /// A series' own point labels.
    SetPointLabels,
}

/// The kind of chart.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartType {
    /// Columns.
    Column,
    /// Stacked columns.
    StackedColumn,
    /// Bars.
    Bar,
    /// Stacked bars.
    StackedBar,
    /// Lines.
    Line,
    /// A pie.
    Pie,
    /// Points by x and y.
    Scatter,
    /// A radar.
    Radar,
    /// A filled radar.
    RadarFilled,
    /// Bubbles.
    Bubble,
    /// Dots on lines.
    Dot,
}

/// How a series of a column chart draws.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesType {
    /// Columns.
    Column,
    /// A line.
    Line,
}

/// An axis by its name.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartAxis {
    /// The category axis.
    Category,
    /// The value axis.
    Value,
    /// A scatter chart's x axis.
    X,
    /// A scatter chart's y axis.
    Y,
}

/// A scale setting back to automatic.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleKey {
    /// The minimum.
    Min,
    /// The maximum.
    Max,
    /// The major unit.
    MajorUnit,
    /// The minor unit.
    MinorUnit,
    /// The number format.
    NumberFormat,
}

/// Which gridlines.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gridlines {
    /// None.
    None,
    /// Major.
    Major,
    /// Minor.
    Minor,
    /// Both.
    Both,
}

/// A line's dash.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dash {
    /// Solid.
    Solid,
    /// Dashes.
    Dash,
    /// Dots.
    Dot,
    /// Dash and dot.
    DashDot,
    /// Long dashes.
    LongDash,
}

/// A marker's shape.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    /// None.
    None,
    /// A circle.
    Circle,
    /// A diamond.
    Diamond,
    /// A square.
    Square,
    /// A triangle.
    Triangle,
    /// A dash.
    Dash,
    /// A dot.
    Dot,
    /// An x.
    X,
    /// A plus.
    Plus,
    /// A star.
    Star,
}

/// Which text `set_text_style` styles.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPart {
    /// The title.
    Title,
    /// The axis titles.
    AxisTitles,
    /// The axis labels.
    AxisLabels,
    /// The data labels.
    DataLabels,
    /// The legend.
    Legend,
}

/// A legend's or a label's position.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartPosition {
    /// Right.
    Right,
    /// Left.
    Left,
    /// Top.
    Top,
    /// Bottom.
    Bottom,
    /// No legend.
    None,
    /// Centred.
    Center,
    /// Inside the end.
    InsideEnd,
    /// Inside the base.
    InsideBase,
    /// Outside the end.
    OutsideEnd,
    /// Where it fits best.
    BestFit,
}

/// How blank values draw.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blanks {
    /// A gap.
    Gap,
    /// Zero.
    Zero,
    /// Lines joined across.
    Span,
}

/// One series of `add`.
#[allow(missing_docs)]
#[derive(ToolArgs, Debug, Clone, PartialEq)]
pub struct SeriesArg {
    pub name: String,
    pub values: Vec<Option<f64>>,
    /// line: drawn as a line (column charts).
    #[arg(rename = "type")]
    pub kind: Option<SeriesType>,
    /// Bubble sizes.
    pub size: Option<Vec<f64>>,
}

/// A deck's frame, pt.
#[allow(missing_docs)]
#[derive(ToolArgs, Debug, Clone, Copy, PartialEq)]
pub struct ChartBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// An axis' scale; keys left out stay. Numbers keep int or float, as messages name them.
#[allow(missing_docs)]
#[derive(ToolArgs, Debug, Clone, PartialEq)]
pub struct ScaleArg {
    pub min: Option<PyNumber>,
    pub max: Option<PyNumber>,
    pub major_unit: Option<PyNumber>,
    pub minor_unit: Option<PyNumber>,
    pub number_format: Option<String>,
    /// Categories top-down.
    pub reverse: Option<bool>,
    /// Back to automatic.
    pub auto: Option<Vec<ScaleKey>>,
}

/// A series', a point's or the text's look.
#[allow(missing_docs)]
#[derive(ToolArgs, Debug, Clone, PartialEq)]
pub struct StyleArg {
    pub fill: Option<String>,
    pub line: Option<String>,
    pub line_width: Option<PyNumber>,
    pub dash: Option<Dash>,
    pub marker: Option<Marker>,
    #[arg(minimum = 2, maximum = 72)]
    pub marker_size: Option<i64>,
    pub marker_fill: Option<String>,
    pub font: Option<String>,
    pub size: Option<PyNumber>,
    pub bold: Option<bool>,
    pub color: Option<String>,
}

/// `edit_chart`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
#[tool(
    name = "edit_chart",
    description = include_str!("edit_chart.md"),
    group = "shared_misc",
    mutates,
    aliases_first("slide", "from_columns")
)]
pub struct EditChart {
    /// Document id.
    pub doc: String,
    /// The chart; add: decks the slide, documents the paragraph to follow. Or $ref.
    #[arg(
        refs,
        alias_when("slide", "action", "add"),
        description = "The chart; add: decks the slide, documents the paragraph to follow. Or $ref."
    )]
    pub target: String,
    /// What to do; read changes nothing.
    pub action: ChartAction,
    /// add: the kind of chart; set_series_type: line or column.
    pub chart_type: Option<ChartType>,
    /// add: category labels, in order.
    pub categories: Option<Vec<String>>,
    /// add: each series' name and one value per category, null for a blank.
    pub data: Option<Vec<SeriesArg>>,
    /// add, decks: the frame, pt.
    #[arg(rename = "box")]
    pub frame: Option<ChartBox>,
    /// add, documents: width in pt. Default 432.
    #[arg(minimum = 36, maximum = 1584)]
    pub width: Option<f64>,
    /// add: the values'; show_data_labels: the labels'. Excel code, e.g. #,##0.0.
    pub number_format: Option<String>,
    /// Series name, or number from 0.
    pub series: Option<String>,
    /// Category label, or number from 0.
    pub category: Option<String>,
    /// One per category (set_values, add_series) or series (add_category), null for a blank; add: scatter, bubble x values.
    pub values: Option<Vec<Option<f64>>>,
    /// set_value: the value; set_gap_width: % of a bar's width.
    pub value: Option<f64>,
    /// New name, label or title; add: the chart title.
    pub text: Option<String>,
    /// set_axis_title, set_axis_scale, set_gridlines; add with scale.
    pub axis: Option<ChartAxis>,
    /// set_axis_scale, add: the axis' scale; keys left out stay.
    pub scale: Option<ScaleArg>,
    /// set_gridlines, add: which gridlines.
    pub gridlines: Option<Gridlines>,
    /// add: values as labels.
    pub data_labels: Option<bool>,
    /// add: series colours in order (pie: slices').
    #[arg(alias = "colours")]
    pub colors: Option<Vec<String>>,
    /// format_series, format_point: fill to marker_fill; set_text_style, add: font to color. Colours as shapes take them, or none.
    pub style: Option<StyleArg>,
    /// set_text_style, add: only this text. Default all.
    pub part: Option<TextPart>,
    /// set_point_labels: one per point.
    pub labels: Option<Vec<String>>,
    /// set_point_labels: also in the workbook (Value From Cells).
    pub cells: Option<bool>,
    /// set_series_type: on a second value axis.
    pub secondary: Option<bool>,
    /// set_legend, add: legend position, none hides it; set_point_labels: label position.
    pub position: Option<ChartPosition>,
    /// add: ref name for the new chart.
    #[arg(rename = "ref")]
    pub ref_name: Option<String>,
    /// add, set_values, add_series: rows of `d:<id>` or table `t:<name>`.
    #[arg(
        rename = "from",
        sources,
        description = "add, set_values, add_series: rows of d:<id> or table t:<name>."
    )]
    pub from: Option<String>,
    /// from's category column, then series columns; default all.
    #[arg(alias = "from_columns")]
    pub columns: Option<Vec<String>>,
    /// add from: category dates' pattern, e.g. d mmmm yyyy.
    pub date_format: Option<String>,
    /// How null values draw; span joins lines across. New charts: gap.
    pub blanks: Option<Blanks>,
}

impl ReadsWhen for EditChart {
    fn reads(&self) -> bool {
        self.action == ChartAction::Read
    }
}
