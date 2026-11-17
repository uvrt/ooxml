//! Owns: a chart part as data plus styling -- plots and their series with the cached
//! values, axes, the legend, titles, data labels, the 3-D view and the chart-wide text and
//! shape properties -- equal to ooxml-common's `chart/read.py` source types. `None` means
//! "the file does not state it": `<c:overlap/>` absent and `<c:overlap val="0"/>` are
//! different things to a bar chart, so nothing is defaulted here.
//!
//! Must not: read XML (`crate::read`), resolve a colour or a font, or lay anything out.

use ooxml_drawingml::source::{Fill, Outline};
use ooxml_drawingml::text::TextBody;

/// The `c:*Chart` group elements a chart is read with, in the reader's order.
pub const CHART_GROUP_ELEMENTS: [&str; 16] = [
    "barChart",
    "bar3DChart",
    "lineChart",
    "line3DChart",
    "areaChart",
    "area3DChart",
    "pieChart",
    "pie3DChart",
    "doughnutChart",
    "ofPieChart",
    "radarChart",
    "scatterChart",
    "bubbleChart",
    "stockChart",
    "surfaceChart",
    "surface3DChart",
];

/// The four axis elements.
pub const AXIS_ELEMENTS: [&str; 4] = ["catAx", "valAx", "dateAx", "serAx"];

/// A cache claiming more points than this is malformed or hostile: an allocation bound.
pub const MAX_CACHE_POINTS: usize = 100_000;

/// The 2-D chart a 3-D one degrades to; every other name unchanged.
pub fn flat_chart_kind(kind: &str) -> &str {
    match kind {
        "bar3DChart" => "barChart",
        "line3DChart" => "lineChart",
        "area3DChart" => "areaChart",
        "pie3DChart" => "pieChart",
        "surface3DChart" => "surfaceChart",
        other => other,
    }
}

/// Whether a group element is a 3-D spelling. `surfaceChart` is one although its name does
/// not say so: with no `c:view3D` it draws the same projected surface (measured).
pub fn is_three_d_kind(kind: &str) -> bool {
    matches!(
        kind,
        "bar3DChart" | "line3DChart" | "area3DChart" | "pie3DChart" | "surface3DChart" | "surfaceChart"
    )
}

/// Whether a group element is a surface (either spelling).
pub fn is_surface_kind(kind: &str) -> bool {
    matches!(kind, "surfaceChart" | "surface3DChart")
}

/// `c:tx` / `c:title`: rich DrawingML, the cached string behind a reference, or both.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChartText {
    /// `c:rich`.
    pub rich: Option<TextBody>,
    /// The cached plain string (`c:strRef` or `c:v`).
    pub cached: Option<String>,
}

impl ChartText {
    /// The text as one plain string: the cache, else the rich runs joined; `None` for none.
    pub fn plain(&self) -> Option<String> {
        if let Some(cached) = &self.cached {
            return Some(cached.clone());
        }
        let rich = self.rich.as_ref()?;
        let text: String = rich
            .paragraphs
            .iter()
            .flat_map(|p| p.runs.iter())
            .map(|r| r.text.as_str())
            .collect();
        (!text.is_empty()).then_some(text)
    }
}

/// `c:dPt`: a point's own styling.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DataPoint {
    /// `c:idx`.
    pub index: i64,
    /// Its fill.
    pub fill: Option<Fill>,
    /// Its outline.
    pub outline: Option<Outline>,
    /// `c:invertIfNegative`.
    pub invert_if_negative: Option<bool>,
    /// `c:explosion`, percent of the radius.
    pub explosion: Option<f64>,
}

/// `c:dLbls` (or one `c:dLbl`): what to print on or beside each point.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DataLabels {
    /// `c:delete`.
    pub delete: Option<bool>,
    /// `c:showVal`.
    pub show_value: Option<bool>,
    /// `c:showCatName`.
    pub show_category_name: Option<bool>,
    /// `c:showSerName`.
    pub show_series_name: Option<bool>,
    /// `c:showPercent`.
    pub show_percent: Option<bool>,
    /// `c:showBubbleSize`.
    pub show_bubble_size: Option<bool>,
    /// `c:showLegendKey`.
    pub show_legend_key: Option<bool>,
    /// `c:showLeaderLines`.
    pub show_leader_lines: Option<bool>,
    /// `c:dLblPos`.
    pub position: Option<String>,
    /// `c:numFmt`'s code.
    pub number_format: Option<String>,
    /// `c:numFmt@sourceLinked`.
    pub number_format_source_linked: Option<bool>,
    /// `c:txPr`.
    pub text_properties: Option<TextBody>,
    /// `c:spPr`'s fill.
    pub fill: Option<Fill>,
    /// `c:spPr`'s outline.
    pub outline: Option<Outline>,
    /// `c:dLbl` children by `c:idx`, in the order first stated (a later one with the same
    /// index replaces the earlier in place, as a Python dict does).
    pub overrides: Vec<(i64, DataLabels)>,
    /// A `c:dLbl`'s own text, drawn in place of the composed label.
    pub text: Option<ChartText>,
    /// `c15:showDataLabelsRange`.
    pub show_range: Option<bool>,
}

impl DataLabels {
    /// The override for point `index`.
    pub fn override_for(&self, index: i64) -> Option<&DataLabels> {
        self.overrides.iter().find(|(i, _)| *i == index).map(|(_, l)| l)
    }
}

/// `c:marker`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Marker {
    /// `c:symbol`.
    pub symbol: Option<String>,
    /// `c:size`, points.
    pub size: Option<f64>,
    /// Its fill.
    pub fill: Option<Fill>,
    /// Its outline.
    pub outline: Option<Outline>,
}

/// `c:serLines` / `c:hiLowLines`: present means draw; `c:spPr` only styles.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lines {
    /// Its outline.
    pub outline: Option<Outline>,
}

/// `c:upDownBars`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpDownBars {
    /// `c:gapWidth`.
    pub gap_width: Option<f64>,
    /// `c:upBars` fill.
    pub up_fill: Option<Fill>,
    /// `c:upBars` outline.
    pub up_outline: Option<Outline>,
    /// `c:downBars` fill.
    pub down_fill: Option<Fill>,
    /// `c:downBars` outline.
    pub down_outline: Option<Outline>,
}

/// One `c:ser`. `values` is dense and `ptCount` long with `None` for blanks; `categories`
/// is padded with empty strings to the same length.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Series {
    /// `c:idx` (its position among the plot's series where unstated).
    pub index: i64,
    /// `c:order` (likewise).
    pub order: i64,
    /// `c:tx`.
    pub name: Option<ChartText>,
    /// The category labels.
    pub categories: Vec<String>,
    /// The values (`c:yVal` before `c:val`).
    pub values: Vec<Option<f64>>,
    /// `c:xVal`'s numbers.
    pub x_values: Option<Vec<Option<f64>>>,
    /// `c:bubbleSize`'s numbers.
    pub bubble_sizes: Option<Vec<Option<f64>>>,
    /// The value cache's format code.
    pub format_code: Option<String>,
    /// The category cache's format code, when the categories are numbers.
    pub category_format_code: Option<String>,
    /// The numbers of a numeric category cache.
    pub category_values: Option<Vec<Option<f64>>>,
    /// Each category point's own format code, where any point has one.
    pub category_point_format_codes: Option<Vec<Option<String>>>,
    /// Its fill.
    pub fill: Option<Fill>,
    /// Its outline.
    pub outline: Option<Outline>,
    /// `c:invertIfNegative`.
    pub invert_if_negative: Option<bool>,
    /// `c:smooth`.
    pub smooth: Option<bool>,
    /// `c:marker`.
    pub marker: Option<Marker>,
    /// `c:explosion`.
    pub explosion: Option<f64>,
    /// `c:dPt`s.
    pub data_points: Vec<DataPoint>,
    /// `c:dLbls`.
    pub data_labels: Option<DataLabels>,
    /// `c15:datalabelsRange`'s cache, by point.
    pub label_range: Option<Vec<Option<String>>>,
}

/// One `c:catAx` / `c:valAx` / `c:dateAx` / `c:serAx`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Axis {
    /// The element's local name.
    pub kind: String,
    /// `c:axId`.
    pub axis_id: Option<String>,
    /// `c:axPos`.
    pub position: Option<String>,
    /// `c:delete`.
    pub delete: bool,
    /// `c:scaling/c:orientation` (`minMax` where unstated).
    pub orientation: String,
    /// `c:min`.
    pub minimum: Option<f64>,
    /// `c:max`.
    pub maximum: Option<f64>,
    /// `c:majorUnit`.
    pub major_unit: Option<f64>,
    /// `c:minorUnit`.
    pub minor_unit: Option<f64>,
    /// `c:logBase`.
    pub log_base: Option<f64>,
    /// `c:numFmt`'s code.
    pub number_format: Option<String>,
    /// `c:numFmt@sourceLinked`.
    pub number_format_source_linked: Option<bool>,
    /// `c:majorTickMark`: `None` when absent, `cross` when it states no `val`.
    pub major_tick_mark: Option<String>,
    /// `c:minorTickMark`.
    pub minor_tick_mark: Option<String>,
    /// `c:tickLblPos`.
    pub tick_label_position: Option<String>,
    /// `c:majorGridlines` present.
    pub major_gridlines: bool,
    /// Its outline.
    pub major_gridline_outline: Option<Outline>,
    /// `c:minorGridlines` present.
    pub minor_gridlines: bool,
    /// Its outline.
    pub minor_gridline_outline: Option<Outline>,
    /// `c:crossAx`.
    pub cross_axis_id: Option<String>,
    /// `c:crosses`.
    pub crosses: Option<String>,
    /// `c:crossesAt`.
    pub crosses_at: Option<f64>,
    /// `c:crossBetween`.
    pub cross_between: Option<String>,
    /// `c:lblOffset`.
    pub label_offset: Option<f64>,
    /// `c:lblAlgn`.
    pub label_align: Option<String>,
    /// `c:tickLblSkip`.
    pub tick_label_skip: Option<i64>,
    /// `c:tickMarkSkip`.
    pub tick_mark_skip: Option<i64>,
    /// `c:title`'s text.
    pub title: Option<ChartText>,
    /// Whether a `c:title` is stated at all.
    pub title_stated: bool,
    /// `c:txPr`.
    pub text_properties: Option<TextBody>,
    /// `c:spPr`'s outline.
    pub outline: Option<Outline>,
}

/// One `c:barChart` / `c:lineChart` / ... group inside `c:plotArea`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plot {
    /// The element's local name.
    pub kind: String,
    /// Its series, by `order`.
    pub series: Vec<Series>,
    /// `c:axId`s, in order.
    pub axis_ids: Vec<String>,
    /// `c:grouping`.
    pub grouping: Option<String>,
    /// `c:varyColors`.
    pub vary_colors: Option<bool>,
    /// `c:barDir`.
    pub bar_direction: Option<String>,
    /// `c:gapWidth`.
    pub gap_width: Option<f64>,
    /// `c:gapDepth`.
    pub gap_depth: Option<f64>,
    /// `c:overlap`.
    pub overlap: Option<f64>,
    /// `c:shape`.
    pub shape: Option<String>,
    /// `c:holeSize`.
    pub hole_size: Option<f64>,
    /// `c:firstSliceAng`.
    pub first_slice_angle: Option<f64>,
    /// `c:scatterStyle`.
    pub scatter_style: Option<String>,
    /// `c:radarStyle`.
    pub radar_style: Option<String>,
    /// `c:marker`.
    pub marker: Option<bool>,
    /// `c:dLbls`.
    pub data_labels: Option<DataLabels>,
    /// `c:bubbleScale`.
    pub bubble_scale: Option<f64>,
    /// `c:sizeRepresents`.
    pub size_represents: Option<String>,
    /// `c:showNegBubbles`.
    pub show_negative_bubbles: Option<bool>,
    /// `c:bubble3D`.
    pub bubble_3d: Option<bool>,
    /// `c:ofPieType`.
    pub of_pie_type: Option<String>,
    /// `c:splitType`.
    pub split_type: Option<String>,
    /// `c:splitPos`.
    pub split_position: Option<f64>,
    /// `c:custSplit`'s points.
    pub custom_split: Vec<i64>,
    /// `c:secondPieSize`.
    pub second_pie_size: Option<f64>,
    /// `c:serLines`.
    pub series_lines: Option<Lines>,
    /// `c:hiLowLines`.
    pub hi_low_lines: Option<Lines>,
    /// `c:upDownBars`.
    pub up_down_bars: Option<UpDownBars>,
    /// `c:wireframe`.
    pub wireframe: Option<bool>,
    /// `c:bandFmts` fills by `c:idx`, in the order first stated.
    pub band_fills: Vec<(i64, Fill)>,
    /// `c:bandFmts` outlines by `c:idx`.
    pub band_outlines: Vec<(i64, Outline)>,
}

/// `c:view3D`, every field as stated.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct View3D {
    /// `c:rotX`.
    pub rot_x: Option<f64>,
    /// `c:rotY`.
    pub rot_y: Option<f64>,
    /// `c:depthPercent`.
    pub depth_percent: Option<f64>,
    /// `c:hPercent`.
    pub h_percent: Option<f64>,
    /// `c:rAngAx`.
    pub right_angle_axes: Option<bool>,
    /// `c:perspective`.
    pub perspective: Option<f64>,
}

/// `c:legend`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Legend {
    /// `c:legendPos`.
    pub position: Option<String>,
    /// `c:overlay`.
    pub overlay: bool,
    /// `c:txPr`.
    pub text_properties: Option<TextBody>,
    /// `c:spPr`'s fill.
    pub fill: Option<Fill>,
    /// `c:spPr`'s outline.
    pub outline: Option<Outline>,
    /// `c:legendEntry` indices marked deleted, ascending.
    pub deleted_entries: Vec<i64>,
}

/// A whole `c:chartSpace`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chart {
    /// The plots, in document order.
    pub plots: Vec<Plot>,
    /// The axes, in document order.
    pub axes: Vec<Axis>,
    /// `c:title`'s text.
    pub title: Option<ChartText>,
    /// `c:autoTitleDeleted`.
    pub auto_title_deleted: bool,
    /// Whether `c:title` is stated.
    pub title_stated: bool,
    /// Whether `c:autoTitleDeleted` is stated.
    pub auto_title_deleted_stated: bool,
    /// `c:legend`.
    pub legend: Option<Legend>,
    /// `c:view3D`.
    pub view_3d: Option<View3D>,
    /// `c:dispBlanksAs` as stated.
    pub display_blanks_as: Option<String>,
    /// `c:plotVisOnly`.
    pub plot_visible_only: Option<bool>,
    /// `c:chartSpace/c:spPr`'s fill.
    pub fill: Option<Fill>,
    /// Its outline.
    pub outline: Option<Outline>,
    /// `c:plotArea/c:spPr`'s fill.
    pub plot_area_fill: Option<Fill>,
    /// Its outline.
    pub plot_area_outline: Option<Outline>,
    /// `c:chartSpace/c:txPr`.
    pub text_properties: Option<TextBody>,
    /// `c:roundedCorners`.
    pub round_corners: Option<bool>,
    /// `c:clrMapOvr`'s mapping, attribute by attribute (local names), in order.
    pub color_map_override: Option<Vec<(String, String)>>,
    /// `c:externalData@r:id`.
    pub external_data_rel_id: Option<String>,
    /// `c:date1904`.
    pub date1904: bool,
    /// `c:title/c:txPr`.
    pub title_text_properties: Option<TextBody>,
}
