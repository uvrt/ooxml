//! Owns: the layout's measured constants, each as `chart/layout.py` states it (where it was
//! measured, and its residual, are documented there).
//!
//! Must not: hold a value that was fitted here rather than measured there.

#![allow(missing_docs)]

pub const EMU_PER_POINT: f64 = 12700.0;
pub const FRAME_PADDING_PT: f64 = 6.5;
pub const EDGE_INSET_PT: f64 = 11.0;
pub const LABEL_INK_CENTRE_EM: f64 = 0.27;
pub const VALUE_LABEL_GAP_EM: f64 = 0.645;
pub const CATEGORY_LABEL_GAP_ASCENT: f64 = 2.0 / 3.0;
pub const CATEGORY_LABEL_GAP_EM: f64 = 0.615;
pub const TOP_INSET_BASE_PT: f64 = 5.0;
pub const LEGEND_ROW_PITCH_RATIO: f64 = 0.99;
pub const LEGEND_ROW_PITCH_PT: f64 = 6.0;
pub const LEGEND_BAND_PAD_PT: f64 = 6.0;
pub const LEGEND_WRAP_BAND_TRIM_PT: f64 = 1.5;
pub const LEGEND_SWATCH_EM: f64 = 0.549;
pub const LEGEND_SWATCH_GAP_EM: f64 = 0.237;
pub const LEGEND_SIDE_MAX_FRACTION: f64 = 0.40;
pub const LEGEND_SIDE_LEAD_EM: f64 = 1.60;
pub const LEGEND_SIDE_TRAIL_EM: f64 = 1.01;
pub const WORD_LEGEND_LEAD_PT: f64 = 13.25;
pub const WORD_LEGEND_TRAIL_PT: f64 = 10.13;
pub const WORD_LEGEND_LEFT_PT: f64 = 8.25;
pub const LEGEND_ENTRY_KEY_EM: f64 = 2.0 * LEGEND_SWATCH_EM;
pub const LINE_LEGEND_ENTRY_KEY_PT: f64 = 24.000;
pub const LEGEND_ENTRY_SLACK: f64 = 0.2;
pub const LEGEND_BAND_MAX_FRACTION: f64 = 0.9;
pub const LEGEND_HORIZONTAL_OFFSET_PT: f64 = 0.75;
pub const TITLE_BAND_LINES: f64 = 1.4769;
pub const TITLE_BASELINE_ASCENTS: f64 = 1.5046;
pub const WORD_TITLE_PAD_PT: f64 = 9.0;
pub const WORD_TITLE_BASELINE_PT: f64 = 7.5;
pub const WORD_TITLE_BASELINE_EM: f64 = 0.9412;
pub const WORD_AXIS_TITLE_INSET_PT: f64 = 12.5;
pub const WORD_SHORT_PLOT_EM: f64 = 1.1;
pub const WORD_SHORT_PLOT_FLOOR_PT: f64 = 0.01;
pub const DEFAULT_CHART_FONT_PT: f64 = 10.0;
pub const DEFAULT_AXIS_LINE_EMU: f64 = 6350.0;
pub const DEFAULT_AXIS_COLOR: &str = "#000000";
pub const TICK_MAJOR_ASCENT: f64 = 1.0 / 3.0;
pub const TICK_MINOR_ASCENT: f64 = 1.0 / 4.0;
pub const DEFAULT_TICK_MARK: &str = "cross";
pub const OFFICE_2007_TICK_MARKS: (&str, &str) = ("out", "none");
pub const OFFICE_2007_AXIS_COLOR: &str = "#898989";
pub const MINOR_TICKS_PER_MAJOR: f64 = 5.0;
pub const MAX_MINOR_TICKS: usize = 5000;
pub const VIEW_3D_FLOOR_COLOR: &str = "#898989";
pub const DEFAULT_LINE_SERIES_WIDTH_EMU: f64 = 19050.0;
pub const DEFAULT_MARKER_OUTLINE_EMU: f64 = 6350.0;
pub const DEFAULT_MARKER_CYCLE: [&str; 6] = ["diamond", "square", "triangle", "x", "star", "dot"];
pub const DEFAULT_MARKER_SIZE_PT: f64 = 6.0;
pub const DATA_LABEL_GAP_PT: f64 = 4.85;
pub const DATA_LABEL_INNER_GAP_PT: f64 = 4.05;
pub const DATA_LABEL_LINE_GAP_EM: f64 = 0.6;
pub const ROTATED_LABEL_RATIO: f64 = 1.0;
pub const WRAPPED_LABEL_MAX_LINES: usize = 6;
pub const ROTATED_LABEL_DEGREES: f64 = -45.0;
pub const ROTATED_LABEL_HEADROOM_PT: f64 = 6.25;
pub const LABEL_ELLIPSIS: &str = "\u{2026}";
pub const ROTATED_LABEL_OFFSET_X_PT: f64 = 2.0;
pub const BAND_COLOR_CYCLE_SLACK: usize = 2;
pub const COMBO_CHART_KINDS: [&str; 3] = ["areaChart", "barChart", "lineChart"];
pub const DEFAULT_AREA_CROSS_BETWEEN: &str = "midCat";
pub const RADAR_LABEL_RESERVE_LINES: f64 = 1.2578;
pub const RADAR_LABEL_RESERVE_PT: f64 = 5.2501;
pub const RADAR_LABEL_GAP_PT: f64 = 2.85;
pub const RADAR_LABEL_GAP_FRACTION: f64 = 0.04;
pub const RADAR_LABEL_FOOT_PT: f64 = 1.35;
pub const RADAR_LABEL_MAX_FRACTION: f64 = 0.25;
pub const RADAR_VALUE_LABEL_DIGITS: f64 = 2.0;
pub const RADAR_MARKER_SIZE_PT: f64 = 6.0;
pub const LINE_LEGEND_KEY_PT: f64 = 19.200;
pub const LINE_LEGEND_KEY_GAP_PT: f64 = 2.025;
pub const DEFAULT_HOLE_SIZE: f64 = 0.0;
pub const INVERTED_BAR_OUTLINE_EMU: f64 = 9525.0;
pub const BUBBLE_REGION_INSET_PT: f64 = 5.0;
pub const BUBBLE_SCALE_HALF: f64 = 1000.0 / 3.0;
pub const DEFAULT_BUBBLE_SCALE: f64 = 100.0;
pub const DEFAULT_SIZE_REPRESENTS: &str = "area";
pub const BUBBLE_LABEL_SIDE_GAP_PT: f64 = 8.494;
pub const BUBBLE_LABEL_EDGE_GAP_PT: f64 = 7.0;
pub const DEFAULT_OF_PIE_SPLIT: &str = "auto";
pub const OF_PIE_AUTO_DIVISOR: usize = 3;
pub const DEFAULT_SECOND_PIE_SIZE: f64 = 75.0;
pub const DEFAULT_OF_PIE_GAP_WIDTH: f64 = 100.0;
pub const OF_PIE_BAR_GAP_DIVISOR: f64 = 2.0;
pub const OF_PIE_OTHER_ANGLE: f64 = 90.0;
pub const DEFAULT_UP_DOWN_GAP_WIDTH: f64 = 150.0;
pub const DEFAULT_UP_BAR_FILL: &str = "#F9F9F9";
pub const DEFAULT_DOWN_BAR_FILL: &str = "#3F3F3F";
pub const MIN_BAR_SLOTS: f64 = 0.01;
pub const MAX_MAJOR_TICKS: usize = 1000;
pub const DEFAULT_GAP_WIDTH: f64 = 150.0;
pub const ACCENT_KEYS: [&str; 6] = ["accent1", "accent2", "accent3", "accent4", "accent5", "accent6"];
pub const VARY_COLOR_CYCLE_SHADE: f64 = 0.76;
pub const VARY_COLOR_CYCLE_TINT: f64 = 0.23;
pub const AXIS_MAX_INTERVALS: i64 = 10;
pub const AXIS_HEADROOM: f64 = 0.05;
pub const AXIS_END_LABEL_LINES: i64 = 2;
pub const AXIS_EDGE_RESERVE_PT: f64 = 2.0 * EDGE_INSET_PT;
pub const VIEW_3D_DEPTH_PROJECTION: f64 = 0.2030;
pub const VIEW_3D_SCENE_MARGIN: f64 = 0.0079;
pub const VIEW_3D_DEPTH_ROW_GAP: f64 = DEFAULT_GAP_WIDTH / 100.0;
pub const DEFAULT_GAP_DEPTH: f64 = 150.0;
pub const VIEW_3D_LIGHT: [f64; 3] = [0.3238, 0.4876, 0.8108];
pub const VIEW_3D_AMBIENT: f64 = 0.3947;
pub const VIEW_3D_DIFFUSE: f64 = 0.7465;
pub const SURFACE_WIREFRAME_WIDTH_EMU: f64 = 6350.0;
pub const SURFACE_SEAM_WIDTH_EMU: f64 = 3175.0;
pub const LEGEND_SIDE_TITLE_GAP_PT: f64 = 8.99;
pub const VIEW_3D_RIBBON_THICKNESS: f64 = 0.0025;
pub const VIEW_3D_FRONT_LIFT_MAX: f64 = 11.0;
pub const RADIAL_AXIS_SLACK_EM: f64 = 0.875;
pub const BOTTOM_AXIS_RUNG_EM: f64 = 4.0;
pub const BOTTOM_AXIS_RESERVE_PT: f64 = 23.0;
pub const BOTTOM_AXIS_LABEL_GAP_EM: f64 = 0.4;
pub const AXIS_ZERO_ANCHOR_RATIO: f64 = 5.0 / 6.0;
pub const EXTENT_SLACK: f64 = 1e-9;
pub const DECADE_SLACK: f64 = 1e-12;
pub const MAX_ARC_DEGREES: f64 = 90.0;
pub const RICH_TITLE_FALLBACK_FACE: &str = "Arial";
/// `text.measure.DEFAULT_LINE_HEIGHT_RATIO`.
pub const DEFAULT_LINE_HEIGHT_RATIO: f64 = 1.2;
pub const FALLBACK_DESCENT: f64 = 0.25;
pub const FALLBACK_ASCENT: f64 = DEFAULT_LINE_HEIGHT_RATIO - FALLBACK_DESCENT;

/// `PIE_LABEL_RADIUS`: a slice label's distance by position, as a fraction of the radius.
pub fn pie_label_radius(position: &str) -> Option<f64> {
    Some(match position {
        "ctr" => 0.500,
        "inEnd" => 0.856,
        "outEnd" => 1.020,
        "bestFit" => 0.710,
        _ => return None,
    })
}

/// `VIEW_3D_FACE_SHADES`.
pub fn face_shade(face: &str) -> Option<f64> {
    Some(match face {
        "front" => 1.0,
        "top" => 0.7587,
        "right" => 0.6364,
        "left" | "bottom" => 0.3947,
        _ => return None,
    })
}

/// `_MARKER_PRESETS`: a marker symbol's preset geometry.
pub fn marker_preset(symbol: &str) -> Option<&'static str> {
    Some(match symbol {
        "circle" | "dot" => "ellipse",
        "square" | "dash" => "rect",
        "diamond" => "diamond",
        "triangle" => "triangle",
        "x" => "mathMultiply",
        "plus" => "mathPlus",
        "star" => "star5",
        _ => return None,
    })
}

/// `VIEW_3D_SURFACE_LIGHT`: `3.0 ** -0.5` thrice.
pub fn surface_light() -> [f64; 3] {
    let v = 3.0_f64.powf(-0.5);
    [v, v, v]
}

/// `_SIN_45`.
pub fn sin_45() -> f64 {
    45.0_f64.to_radians().sin()
}

/// The chart kinds the layout draws (`DRAWABLE_CHART_KINDS`), after flattening.
pub const DRAWABLE_CHART_KINDS: [&str; 11] = [
    "barChart",
    "lineChart",
    "areaChart",
    "scatterChart",
    "bubbleChart",
    "pieChart",
    "doughnutChart",
    "ofPieChart",
    "radarChart",
    "stockChart",
    "surfaceChart",
];
