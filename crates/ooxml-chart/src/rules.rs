//! Owns: where Word and PowerPoint lay out and draw the same chart differently
//! (`chart/rules.py`): each difference a field measured on the application it describes,
//! with PowerPoint's as pptx2svg reproduces it ([`POWERPOINT`]) and Word's as docx2svg
//! measured it ([`WORD`]). The measurements are documented on the Python fields.
//!
//! Must not: hold a rule that was not measured, or decide anything itself.

/// How a title takes its band and baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleRule {
    /// PowerPoint's: a band of line boxes, the baseline a number of ascents down.
    Lines,
    /// Word's: the line pitch plus 9 pt, the baseline 7.5 pt + 0.9412 em down.
    Pitch,
}

/// The pads round a legend at the side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideLegendRule {
    /// PowerPoint's: em-based pads, placed a pad off the plot.
    Em,
    /// Word's: fixed pads and half the key, standing against the frame.
    Word,
}

/// What a `legendPos="tr"` legend is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopRightLegendRule {
    /// A top one.
    Band,
    /// Word's: a column of entries at the right.
    Column,
}

/// The order of legend entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegendOrderRule {
    /// Series order.
    Series,
    /// Reversed where the series stand bottom to top.
    Stack,
}

/// Where a text body's first baseline stands (the drawing rule a chart's text boxes are
/// placed against).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstBaseline {
    /// The measurer's ascent ratio (PowerPoint's).
    Box,
    /// The line box less the face's descent (Word's).
    Descent,
}

/// How one application lays out and draws a chart (fields as `ChartRules` documents them).
#[derive(Debug, Clone, PartialEq)]
pub struct ChartRules {
    /// `powerpoint` or `word`.
    pub name: &'static str,
    /// The drawing rule for a text box's first baseline.
    pub first_baseline: FirstBaseline,
    /// The chart space's fill where its `c:spPr` states none (`RRGGBB`).
    pub chart_fill: Option<&'static str>,
    /// The chart space's line where it states none: `RRGGBB` and width in EMU.
    pub chart_line: Option<(&'static str, f64)>,
    /// The plot area's fill where it states none.
    pub plot_fill: Option<&'static str>,
    /// Whether the plot area's own line is drawn.
    pub plot_line: bool,
    /// The title's band and baseline.
    pub title: TitleRule,
    /// The pads round a side legend.
    pub side_legend: SideLegendRule,
    /// Whether a top legend stands under the title.
    pub legend_under_title: bool,
    /// What a top-right legend is.
    pub top_right_legend: TopRightLegendRule,
    /// Legend entry order.
    pub legend_order: LegendOrderRule,
    /// Whether clustered horizontal bars put the first series at the bottom.
    pub bars_upward: bool,
    /// Whether a line series' stated width without colour is drawn at that width.
    pub stated_line_width: bool,
    /// Whether a legend key is outlined with its series' line.
    pub legend_key_outlines: bool,
    /// Word's automatic titles.
    pub auto_title: bool,
    /// Whether axis titles are laid out and drawn.
    pub axis_titles: bool,
    /// Whether a short plot takes half the shortfall from the band under it.
    pub short_plot: bool,
    /// Whether a rich title takes PowerPoint's own fonts.
    pub rich_title_fonts: bool,
}

/// PowerPoint, as pptx2svg reproduces it.
pub const POWERPOINT: ChartRules = ChartRules {
    name: "powerpoint",
    first_baseline: FirstBaseline::Box,
    chart_fill: None,
    chart_line: None,
    plot_fill: None,
    plot_line: false,
    title: TitleRule::Lines,
    side_legend: SideLegendRule::Em,
    legend_under_title: true,
    top_right_legend: TopRightLegendRule::Band,
    legend_order: LegendOrderRule::Stack,
    bars_upward: true,
    stated_line_width: false,
    legend_key_outlines: false,
    auto_title: false,
    axis_titles: true,
    short_plot: false,
    rich_title_fonts: true,
};

/// Word, as docx2svg measured it.
pub const WORD: ChartRules = ChartRules {
    name: "word",
    first_baseline: FirstBaseline::Descent,
    chart_fill: Some("FFFFFF"),
    chart_line: Some(("898989", 6350.0)),
    plot_fill: Some("FFFFFF"),
    plot_line: true,
    title: TitleRule::Pitch,
    side_legend: SideLegendRule::Word,
    legend_under_title: true,
    top_right_legend: TopRightLegendRule::Column,
    legend_order: LegendOrderRule::Stack,
    bars_upward: true,
    stated_line_width: true,
    legend_key_outlines: true,
    auto_title: true,
    axis_titles: true,
    short_plot: true,
    rich_title_fonts: false,
};
