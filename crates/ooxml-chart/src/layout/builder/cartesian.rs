//! Owns: cartesian charts -- bars, lines, areas, stock and surface charts and their
//! combinations, with a secondary value axis -- and scatter and bubble charts (two value
//! axes, their crossing, bubbles and point labels).
//!
//! Must not: lay out a polar chart.

use super::frame::{texts, widest, PlotRectInput};
use super::labels::LabelPoint;
use super::marks::TickRun;
use super::series::{minor_tick_values, tick_values};
use super::{same_axis, Builder, Group, Series};
use crate::layout::consts::{
    BUBBLE_LABEL_EDGE_GAP_PT, BUBBLE_LABEL_SIDE_GAP_PT, BUBBLE_REGION_INSET_PT, BUBBLE_SCALE_HALF, DATA_LABEL_GAP_PT,
    DATA_LABEL_LINE_GAP_EM, DEFAULT_AXIS_COLOR, DEFAULT_BUBBLE_SCALE, DEFAULT_SIZE_REPRESENTS, EDGE_INSET_PT,
    FRAME_PADDING_PT, INVERTED_BAR_OUTLINE_EMU, VALUE_LABEL_GAP_EM,
};
use crate::layout::data::ChartData;
use crate::layout::numbers::format_number;
use crate::layout::paths::{split_runs, Point};
use crate::layout::scale::{apply_axis_limits, nice_axis_scale, py_max, py_min, NiceOptions, Scale};
use crate::layout::sources::labels_shown;
use crate::layout::text::ChartFont;
use crate::layout::three_d::Rect;
use crate::model::Axis;
use ooxml_drawingml::drawable::{Colour, Fill, Outline};

/// The span of `numbers`: `(0, 0)` for none.
fn span(numbers: &[f64]) -> (f64, f64) {
    if numbers.is_empty() {
        (0.0, 0.0)
    } else {
        super::series::min_max(numbers)
    }
}

/// A cartesian chart's plots and axes: each plot drawn as a group, its series, the shared
/// categories, the value (and secondary) and category axes with the groups on each, and
/// their scales, fonts and tick texts.
struct Combo<'a> {
    groups: Vec<Group<'a>>,
    drawn: Vec<Vec<Series>>,
    series: Vec<Series>,
    categories: Vec<String>,
    axes: (Option<&'a Axis>, Option<&'a Axis>, Option<&'a Axis>),
    primary: Vec<usize>,
    secondary: Vec<usize>,
    scale: Scale,
    second_scale: Option<Scale>,
    tick_texts: Vec<(f64, String)>,
    second_texts: Option<Vec<(f64, String)>>,
    value_font: ChartFont,
    category_font: ChartFont,
    second_font: Option<ChartFont>,
    second_shown: bool,
}

/// A scatter's plot rectangle, axes, scales and where the axes cross.
#[derive(Clone, Copy)]
struct ScatterAxes<'a> {
    rect: Rect,
    axes: (Option<&'a Axis>, Option<&'a Axis>),
    scales: (Scale, Scale),
    cross: (f64, f64),
}

/// Which way a value maps to a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Along {
    X,
    Y,
}

impl<'a> Builder<'a> {
    /// A position along `along` for `value`.
    pub fn to_position(&self, along: Along, rect: Rect, value: f64, scale: Scale) -> f64 {
        match along {
            Along::X => self.value_to_x(rect, value, scale),
            Along::Y => self.value_to_y(rect, value, scale),
        }
    }

    /// A bar, line, area, stock or surface chart, or a combination of them.
    pub fn build_cartesian(&mut self) -> ChartData {
        let mut combo = self.combo();
        self.combo_scales(&mut combo);
        let second = match (&combo.second_texts, &combo.second_font) {
            (Some(t), Some(f)) if combo.second_shown => Some((t.as_slice(), f)),
            _ => None,
        };
        let plot_rect = self.plot_rect(&PlotRectInput {
            tick_texts: &combo.tick_texts,
            categories: &combo.categories,
            value_font: &combo.value_font,
            category_font: &combo.category_font,
            scale: combo.scale,
            second,
            flat: false,
        });
        let second_band = match second {
            Some((t, f)) if !t.is_empty() => self.value_label_band(&texts(t), f) - EDGE_INSET_PT,
            _ => 0.0,
        };
        for group in &mut combo.groups {
            group.state.scene = self.group.scene;
        }
        self.draw_background(plot_rect, None);
        self.draw_title();
        self.draw_axis_titles(plot_rect);
        self.draw_combo_back(&mut combo, plot_rect);
        self.draw_combo_axes(&combo, plot_rect);
        self.draw_combo_front(&mut combo, plot_rect, second_band);
        let mut data = self.chart_data(&combo.series, None, Some(combo.scale));
        data.bar_direction = Some(self.plot.bar_direction.clone().unwrap_or_else(|| "col".into()));
        data
    }

    /// The plots drawn, their series, categories and axes.
    fn combo(&mut self) -> Combo<'a> {
        let mut groups: Vec<Group<'a>> = self.drawn_plots().into_iter().map(|p| self.group_for(p)).collect();
        if self.any_line_keyed() {
            self.group.line_legend_keys = true;
            for group in &mut groups {
                group.state.line_legend_keys = true;
            }
        }
        let mut drawn: Vec<Vec<Series>> = Vec::new();
        for group in &mut groups {
            drawn.push(self.in_group(group, |b| b.series()));
        }
        let series: Vec<Series> = drawn.iter().flatten().cloned().collect();
        let categories = self.categories(&series, true);
        let (value_axis, second_axis) = self.combo_axes().unwrap_or((self.value_axis(), None));
        let category_axis = self.category_axis();
        let attached = |b: &Self, axis: Option<&Axis>| -> Vec<usize> {
            let on = |g: &Group<'a>| same_axis(b.plot_value_axis(g.plot), axis);
            (0..groups.len()).filter(|&i| groups.get(i).is_some_and(on)).collect()
        };
        let mut primary = attached(self, value_axis);
        if primary.is_empty() {
            primary = vec![0];
        }
        let secondary = if second_axis.is_some() {
            attached(self, second_axis)
        } else {
            Vec::new()
        };
        let value_font = self.label_font(value_axis);
        let category_font = self.label_font(category_axis);
        Combo {
            groups,
            drawn,
            series,
            categories,
            axes: (value_axis, second_axis, category_axis),
            primary,
            secondary,
            scale: Scale::new(0.0, 1.0, 1.0),
            second_scale: None,
            tick_texts: Vec::new(),
            second_texts: None,
            value_font,
            category_font,
            second_font: None,
            second_shown: false,
        }
    }

    /// The primary and secondary scales, and their tick texts.
    fn combo_scales(&mut self, combo: &mut Combo<'a>) {
        let (value_axis, second_axis, _) = combo.axes;
        let scale = self.axis_scale(&mut combo.groups, &combo.drawn, &combo.primary, value_axis);
        combo.scale = scale;
        self.group.bands_scale = Some(scale);
        for group in &mut combo.groups {
            group.state.bands_scale = Some(scale);
        }
        let lead = combo.primary.first().copied().unwrap_or(0);
        combo.tick_texts = match combo.groups.get_mut(lead) {
            Some(group) => self.in_group(group, |b| b.tick_texts(scale, value_axis)),
            None => self.tick_texts(scale, value_axis),
        };
        if combo.secondary.is_empty() {
            return;
        }
        let second_scale = self.axis_scale(&mut combo.groups, &combo.drawn, &combo.secondary, second_axis);
        combo.second_scale = Some(second_scale);
        combo.second_font = Some(self.label_font(second_axis));
        let first = combo.secondary.first().copied().unwrap_or(0);
        combo.second_texts = combo
            .groups
            .get_mut(first)
            .map(|g| self.in_group(g, |b| b.tick_texts(second_scale, second_axis)));
        combo.second_shown = labels_shown(second_axis);
    }

    /// The scale group `index` is drawn on.
    fn group_scale(combo: &Combo<'a>, index: usize) -> Scale {
        if combo.secondary.contains(&index) {
            combo.second_scale.unwrap_or(combo.scale)
        } else {
            combo.scale
        }
    }

    /// The gridlines and every plot's marks.
    fn draw_combo_back(&mut self, combo: &mut Combo<'a>, plot_rect: Rect) {
        let (value_axis, second_axis, category_axis) = combo.axes;
        if let (Some(&i), Some(s)) = (combo.secondary.first(), combo.second_scale) {
            if let Some(group) = combo.groups.get_mut(i) {
                self.in_group(group, |b| b.draw_gridlines(plot_rect, s, second_axis, false));
            }
        }
        if self.draws_a_scene() {
            self.draw_scene_gridlines(plot_rect, combo.scale, value_axis);
            self.draw_scene_floor(plot_rect, combo.scale, category_axis);
        } else {
            self.draw_gridlines(plot_rect, combo.scale, value_axis, true);
            self.draw_category_gridlines(plot_rect, category_axis, combo.categories.len());
        }
        for index in 0..combo.groups.len() {
            let scale = Self::group_scale(combo, index);
            let items = combo.drawn.get(index).cloned().unwrap_or_default();
            let categories = combo.categories.clone();
            if let Some(group) = combo.groups.get_mut(index) {
                self.in_group(group, |b| b.draw_marks(plot_rect, &items, &categories, scale));
            }
        }
    }

    /// The axis lines, ticks and labels, the secondary axis' too.
    fn draw_combo_axes(&mut self, combo: &Combo<'a>, plot_rect: Rect) {
        let (value_axis, second_axis, category_axis) = combo.axes;
        let fonts = (&combo.value_font, &combo.category_font);
        self.draw_axis_lines(plot_rect, combo.scale, value_axis, category_axis);
        let count = combo.categories.len();
        self.draw_axis_ticks(plot_rect, combo.scale, (value_axis, category_axis), count, fonts);
        if !combo.secondary.is_empty() {
            self.draw_second_axis_line(plot_rect, second_axis);
            if let Some(s) = combo.second_scale.filter(|_| !self.is_three_d()) {
                let (majors, minors) = self.value_tick_positions(plot_rect, s, second_axis, false);
                let font = self.label_font(second_axis);
                let run = TickRun {
                    vertical: true,
                    at: plot_rect.right,
                    outward: 1.0,
                    majors: &majors,
                    minors: &minors,
                };
                self.draw_ticks(second_axis, &font, &run);
            }
        }
        let axes = (value_axis, category_axis);
        self.draw_labels(
            plot_rect,
            combo.scale,
            &combo.tick_texts,
            &combo.categories,
            axes,
            fonts,
        );
        let second = (
            combo.second_shown,
            &combo.second_texts,
            &combo.second_font,
            combo.second_scale,
        );
        if let (true, Some(t), Some(f), Some(s)) = second {
            if !t.is_empty() {
                let placed: Vec<(f64, String)> = t
                    .iter()
                    .map(|(v, text)| (self.value_to_y(plot_rect, *v, s), text.clone()))
                    .collect();
                self.labels_down_right(plot_rect, &placed, f);
            }
        }
    }

    /// Every plot's data labels, and the legend.
    fn draw_combo_front(&mut self, combo: &mut Combo<'a>, plot_rect: Rect, second_band: f64) {
        for index in 0..combo.groups.len() {
            let scale = Self::group_scale(combo, index);
            let items = combo.drawn.get(index).cloned().unwrap_or_default();
            let categories = combo.categories.clone();
            if let Some(group) = combo.groups.get_mut(index) {
                self.in_group(group, |b| b.draw_data_labels(plot_rect, &items, &categories, scale));
            }
        }
        let legend_rect = if self.group.scene.is_none() {
            plot_rect
        } else {
            self.three_d_region(combo.scale)
        };
        if self.is_surface() {
            let mut bands = self.band_series(combo.scale, combo.axes.0);
            if matches!(self.legend_position(), Some("l" | "r")) {
                bands.reverse();
            }
            self.draw_legend(legend_rect, &bands, false, None, second_band);
        } else {
            let series = combo.series.clone();
            self.draw_legend(legend_rect, &series, false, None, second_band);
        }
    }

    /// The scale an axis takes over the plots attached to it, from the first's settings.
    fn axis_scale(
        &mut self,
        groups: &mut [Group<'a>],
        drawn: &[Vec<Series>],
        pairs: &[usize],
        axis: Option<&'a Axis>,
    ) -> Scale {
        let mut numbers = Vec::new();
        for &index in pairs {
            let items = drawn.get(index).cloned().unwrap_or_default();
            if let Some(group) = groups.get_mut(index) {
                numbers.extend(self.in_group(group, |b| b.axis_reach(&items)));
            }
        }
        let lead = pairs.first().copied().unwrap_or(0);
        let lead_series = drawn.get(lead).cloned().unwrap_or_default();
        let scale = match groups.get_mut(lead) {
            Some(group) => self.in_group(group, |b| b.scale(&lead_series, axis, None, Some(numbers))),
            None => self.scale(&lead_series, axis, None, Some(numbers)),
        };
        if self.is_three_d() {
            scale
        } else {
            scale.reversed_for(axis)
        }
    }

    fn draw_marks(&mut self, rect: Rect, series: &[Series], categories: &[String], scale: Scale) {
        if self.is_surface() {
            if self.draws_a_scene() {
                self.draw_scene_surface(rect, series, categories, scale);
            }
        } else if self.draws_a_scene() && (self.is_area() || self.is_line()) {
            self.draw_scene_ribbons(rect, series, categories, scale);
        } else if self.is_area() {
            self.draw_areas(rect, series, categories, scale);
        } else if self.is_line() {
            self.draw_lines(rect, series, categories, scale);
            if self.is_stock() {
                self.draw_hi_low_lines(rect, series, categories, scale);
                self.draw_up_down_bars(rect, series, categories, scale);
            }
        } else {
            self.draw_bars(rect, series, categories, scale);
        }
    }

    fn draw_second_axis_line(&mut self, rect: Rect, axis: Option<&Axis>) {
        let Some(axis) = axis.filter(|a| !a.delete) else { return };
        let outline = self.axis_outline(axis.outline.as_ref());
        self.line(rect.right, rect.top, rect.right, rect.bottom, outline.as_ref());
    }

    /// A scatter's (x, y) axes: the plot's, else by position.
    pub fn scatter_axes(&self) -> (Option<&'a Axis>, Option<&'a Axis>) {
        let (mut x, mut y) = (self.axis_for(0), self.axis_for(1));
        if x.is_none() || y.is_none() {
            // `{axis.position: axis ...}`: the last axis at a position wins.
            let at = |position: &str| {
                self.chart
                    .axes
                    .iter()
                    .rev()
                    .find(|a| a.position.as_deref() == Some(position))
            };
            x = x.or_else(|| at("b")).or_else(|| at("t"));
            y = y.or_else(|| at("l")).or_else(|| at("r"));
        }
        (x, y)
    }

    fn x_values(&self, index: usize, item: &Series) -> Vec<Option<f64>> {
        let mut xs: Vec<Option<f64>> = self
            .plot
            .series
            .get(index)
            .and_then(|s| s.x_values.clone())
            .filter(|x| !x.is_empty())
            .unwrap_or_default();
        #[allow(clippy::cast_precision_loss)]
        for position in xs.len()..item.values.len() {
            xs.push(Some((position + 1) as f64));
        }
        xs.truncate(item.values.len());
        xs
    }

    fn crossing(&self, low: f64, high: f64, axis: Option<&Axis>, scale: Scale, along: Along, rect: Rect) -> f64 {
        let (low, high) = if scale.reversed { (high, low) } else { (low, high) };
        if axis.is_some_and(|a| a.tick_label_position.as_deref() == Some("low")) {
            return low;
        }
        let crosses = axis.and_then(|a| a.crosses.as_deref());
        match crosses {
            Some("max") => return high,
            Some("min") => return low,
            _ => {}
        }
        let value = if crosses == Some("val") {
            axis.and_then(|a| a.crosses_at).unwrap_or(0.0)
        } else {
            0.0
        };
        let position = self.to_position(along, rect, value, scale);
        py_min(py_max(position, py_min(low, high)), py_max(low, high))
    }

    /// A scatter or bubble chart.
    pub fn build_scatter(&mut self) -> ChartData {
        let series = self.series();
        let (x_axis, y_axis) = self.scatter_axes();
        let x_values: Vec<Vec<Option<f64>>> = series.iter().enumerate().map(|(i, s)| self.x_values(i, s)).collect();
        let xs: Vec<f64> = x_values.iter().flatten().flatten().copied().collect();
        let ys: Vec<f64> = series.iter().flat_map(|s| s.values.iter().flatten().copied()).collect();
        let fonts = (self.label_font(x_axis), self.label_font(y_axis));
        let (x_scale, y_scale) = self.scatter_scales(&xs, &ys, (x_axis, y_axis), 0.0);
        let mut scales = (x_scale.reversed_for(x_axis), y_scale.reversed_for(y_axis));
        let mut rect = self.scatter_plot_for(scales.0, scales.1, (&fonts.0, &fonts.1));
        if self.is_bubble() {
            let radius = self.largest_bubble() / 2.0;
            let (x2, y2) = self.scatter_scales(&xs, &ys, (x_axis, y_axis), radius / py_max(rect.height(), 1.0));
            scales = (x2.reversed_for(x_axis), y2.reversed_for(y_axis));
            rect = self.scatter_plot_for(scales.0, scales.1, (&fonts.0, &fonts.1));
        }
        let cross = (
            self.crossing(rect.left, rect.right, y_axis, scales.0, Along::X, rect),
            self.crossing(rect.bottom, rect.top, x_axis, scales.1, Along::Y, rect),
        );
        self.draw_background(rect, None);
        self.draw_title();
        self.draw_axis_titles(rect);
        let layout = ScatterAxes {
            rect,
            axes: (x_axis, y_axis),
            scales,
            cross,
        };
        self.draw_scatter_back(&layout, &series, &x_values);
        self.draw_scatter_axes(&layout, (&fonts.0, &fonts.1));
        self.draw_scatter_labels(rect, &series, &x_values, scales);
        self.draw_legend(rect, &series, false, None, 0.0);
        let categories: Vec<Vec<String>> = x_values
            .iter()
            .map(|column| {
                column
                    .iter()
                    .map(|v| v.map_or_else(String::new, |v| format_number(v, None, &ooxml_numfmt::EN_US)))
                    .collect()
            })
            .collect();
        self.chart_data(&series, Some(categories), Some(scales.1))
    }

    /// A scatter's gridlines and series.
    fn draw_scatter_back(&mut self, layout: &ScatterAxes<'a>, series: &[Series], x_values: &[Vec<Option<f64>>]) {
        let ScatterAxes {
            rect,
            axes: (x_axis, y_axis),
            scales: (x_scale, y_scale),
            cross: (cross_x, cross_y),
        } = *layout;
        for (axis, axis_scale, along, crossing, across) in [
            (y_axis, y_scale, Along::Y, cross_y, false),
            (x_axis, x_scale, Along::X, cross_x, true),
        ] {
            let Some(axis) = axis else { continue };
            self.scatter_gridlines(rect, axis, axis_scale, (along, crossing, across));
        }
        for (index, item) in series.iter().enumerate() {
            let xs = x_values.get(index).cloned().unwrap_or_default();
            self.draw_scatter_series(rect, index, item, &xs, (x_scale, y_scale));
        }
    }

    /// A scatter's axis lines, ticks and labels.
    fn draw_scatter_axes(&mut self, layout: &ScatterAxes<'a>, fonts: (&ChartFont, &ChartFont)) {
        let ScatterAxes {
            rect,
            axes: (x_axis, y_axis),
            scales: (x_scale, y_scale),
            cross: (cross_x, cross_y),
        } = *layout;
        let (x_font, y_font) = fonts;
        let x_ticks = self.tick_texts(x_scale, x_axis);
        let y_ticks = self.tick_texts(y_scale, y_axis);
        if let Some(axis) = y_axis.filter(|a| !a.delete) {
            let outline = self.axis_outline(axis.outline.as_ref());
            self.line(cross_x, rect.top, cross_x, rect.bottom, outline.as_ref());
        }
        if let Some(axis) = x_axis.filter(|a| !a.delete) {
            let outline = self.axis_outline(axis.outline.as_ref());
            self.line(rect.left, cross_y, rect.right, cross_y, outline.as_ref());
        }
        let far = |axis: &Axis| axis.crosses.as_deref() == Some("max");
        if let Some(axis) = y_axis {
            let (majors, minors) = self.value_tick_positions(rect, y_scale, Some(axis), false);
            let outward = if far(axis) { 1.0 } else { -1.0 };
            let run = TickRun {
                vertical: true,
                at: cross_x,
                outward,
                majors: &majors,
                minors: &minors,
            };
            self.draw_ticks(Some(axis), y_font, &run);
        }
        if let Some(axis) = x_axis {
            let (majors, minors) = self.value_tick_positions(rect, x_scale, Some(axis), true);
            let outward = if far(axis) { -1.0 } else { 1.0 };
            let run = TickRun {
                vertical: false,
                at: cross_y,
                outward,
                majors: &majors,
                minors: &minors,
            };
            self.draw_ticks(Some(axis), x_font, &run);
        }
        if labels_shown(y_axis) {
            let placed: Vec<(f64, String)> = y_ticks
                .iter()
                .map(|(v, t)| (self.value_to_y(rect, *v, y_scale), t.clone()))
                .collect();
            self.labels_down_left(rect, &placed, y_font, Some(cross_x));
        }
        if labels_shown(x_axis) {
            let placed: Vec<(f64, String)> = x_ticks
                .iter()
                .map(|(v, t)| (self.value_to_x(rect, *v, x_scale), t.clone()))
                .collect();
            self.labels_along_bottom(&placed, x_font, cross_y, None, true);
        }
    }

    fn scatter_scales(
        &mut self,
        xs: &[f64],
        ys: &[f64],
        axes: (Option<&'a Axis>, Option<&'a Axis>),
        clearance: f64,
    ) -> (Scale, Scale) {
        let (x_axis, y_axis) = axes;
        let (x_low, x_high) = span(xs);
        let (y_low, y_high) = span(ys);
        let floating = NiceOptions {
            anchor_zero: false,
            ..NiceOptions::default()
        };
        let x_base = nice_axis_scale(x_low, x_high, floating);
        let x_intervals = self.value_axis_intervals(x_axis, true, Some(x_base), None);
        let x_scale = nice_axis_scale(
            x_low,
            x_high,
            NiceOptions {
                intervals: x_intervals,
                clearance,
                ..floating
            },
        );
        let y_intervals = self.value_axis_intervals(y_axis, false, None, None);
        let y_scale = nice_axis_scale(
            y_low,
            y_high,
            NiceOptions {
                intervals: y_intervals,
                clearance,
                ..floating
            },
        );
        (apply_axis_limits(x_scale, x_axis), apply_axis_limits(y_scale, y_axis))
    }

    fn scatter_plot_for(&mut self, x_scale: Scale, y_scale: Scale, fonts: (&ChartFont, &ChartFont)) -> Rect {
        let (x_axis, y_axis) = self.scatter_axes();
        let x_ticks = self.tick_texts(x_scale, x_axis);
        let y_ticks = self.tick_texts(y_scale, y_axis);
        self.scatter_plot_rect(&x_ticks, &y_ticks, fonts, (x_scale, y_scale))
    }

    fn scatter_plot_rect(
        &mut self,
        x_ticks: &[(f64, String)],
        y_ticks: &[(f64, String)],
        fonts: (&ChartFont, &ChartFont),
        scales: (Scale, Scale),
    ) -> Rect {
        let frame = self.frame;
        let (x_font, y_font) = fonts;
        let (x_axis, y_axis) = self.scatter_axes();
        let (show_x, show_y) = (labels_shown(x_axis), labels_shown(y_axis));
        let x_labels = texts(x_ticks);
        let mut left = frame.left + EDGE_INSET_PT;
        if show_y && scales.0.minimum >= 0.0 {
            let wide = widest(y_font, texts(y_ticks));
            left = frame.left + FRAME_PADDING_PT + wide + y_font.box_.descent + VALUE_LABEL_GAP_EM * y_font.size();
        }
        let mut overhang = 0.0;
        if show_x && !x_labels.is_empty() {
            let first = x_labels.first().copied().unwrap_or("");
            left = py_max(left, frame.left + EDGE_INSET_PT + x_font.width(first) / 2.0);
            overhang = x_font.width(x_labels.last().copied().unwrap_or("")) / 2.0;
        }
        left += self.axis_title_band("l");
        let mut right = frame.right - EDGE_INSET_PT - overhang;
        let mut top = frame.top + self.top_inset(&y_font.box_);
        if let Some(title) = self.title_box() {
            top += self.title_band(&title);
        }
        let mut legend_bottom = 0.0;
        let legend = self.legend_position();
        if legend.is_some() && !self.legend_overlays() {
            let font = self.legend_font();
            let band = self.legend_band_height(&font, false);
            match legend {
                Some("b") => legend_bottom = band,
                Some("t" | "tr") => top += band,
                Some("r") => right = frame.right - self.legend_side_width(&font, false) - overhang,
                Some("l") => left += self.legend_side_width(&font, false) - EDGE_INSET_PT,
                _ => {}
            }
        }
        if right - left < 1.0 {
            right = left + 1.0;
        }
        let mut bottom = if show_x && scales.1.minimum >= 0.0 {
            frame.bottom - legend_bottom - self.bottom_label_band(x_font, &[], right - left)
        } else {
            frame.bottom - legend_bottom - self.top_inset(&y_font.box_)
        };
        bottom -= self.axis_title_band("b");
        if bottom - top < 1.0 {
            bottom = top + 1.0;
        }
        Rect::new(left, top, right, bottom)
    }

    fn scatter_gridlines(&mut self, rect: Rect, axis: &Axis, scale: Scale, line: (Along, f64, bool)) {
        let (along, crossing, across) = line;
        let majors: Vec<f64> = tick_values(scale)
            .into_iter()
            .map(|v| self.to_position(along, rect, v, scale))
            .collect();
        let draw = |b: &mut Self, at: f64, outline: Option<&Outline>| {
            if across {
                b.line(at, rect.top, at, rect.bottom, outline);
            } else {
                b.line(rect.left, at, rect.right, at, outline);
            }
        };
        if axis.minor_gridlines {
            let outline = self.axis_outline(axis.minor_gridline_outline.as_ref());
            for value in minor_tick_values(scale, Some(axis)) {
                let at = self.to_position(along, rect, value, scale);
                if (at - crossing).abs() < 0.01 || majors.iter().any(|m| (at - m).abs() < 0.01) {
                    continue;
                }
                draw(self, at, outline.as_ref());
            }
        }
        if axis.major_gridlines {
            let outline = self.axis_outline(axis.major_gridline_outline.as_ref());
            for &at in &majors {
                if (at - crossing).abs() < 0.01 {
                    continue;
                }
                draw(self, at, outline.as_ref());
            }
        }
    }

    fn scatter_points(
        &self,
        rect: Rect,
        item: &Series,
        xs: &[Option<f64>],
        scales: (Scale, Scale),
    ) -> Vec<Option<Point>> {
        let blanks = super::blanks_as(self.chart.display_blanks_as.as_deref(), false);
        let mut points = Vec::new();
        for (index, value) in item.values.iter().enumerate() {
            let x = xs.get(index).copied().flatten();
            let value = match (value, x) {
                (Some(v), Some(_)) => *v,
                (_, Some(_)) if blanks == "zero" => 0.0,
                _ if blanks == "span" => continue,
                _ => {
                    points.push(None);
                    continue;
                }
            };
            let x = x.unwrap_or(0.0);
            points.push(Some((
                self.value_to_x(rect, x, scales.0),
                self.value_to_y(rect, value, scales.1),
            )));
        }
        points
    }

    fn draw_scatter_series(
        &mut self,
        rect: Rect,
        index: usize,
        item: &Series,
        xs: &[Option<f64>],
        scales: (Scale, Scale),
    ) {
        let points = self.scatter_points(rect, item, xs, scales);
        if self.is_bubble() {
            self.draw_bubbles(index, item, &points);
            return;
        }
        if let Some(line) = &item.line {
            for run in split_runs(&points) {
                if run.len() > 1 {
                    self.polyline(&run, line, item.smooth);
                }
            }
        }
        for point in points.iter().flatten() {
            if item.marker_symbol.is_some() {
                self.marker(*point, item);
            }
        }
    }

    fn bubble_region(&mut self) -> Rect {
        let frame = self.frame;
        let inset = BUBBLE_REGION_INSET_PT;
        let (mut left, mut right) = (frame.left + inset, frame.right - inset);
        let (mut top, mut bottom) = (frame.top + inset, frame.bottom - inset);
        if let Some(title) = self.title_box() {
            top += self.title_band(&title);
        }
        let legend = self.legend_position();
        if legend.is_some() && !self.legend_overlays() {
            let font = self.legend_font();
            let band = self.legend_band_height(&font, false);
            match legend {
                Some("b") => bottom -= band,
                Some("t" | "tr") => top += band,
                Some(side @ ("l" | "r")) => {
                    let width = self.legend_side_width(&font, false) - EDGE_INSET_PT;
                    if side == "r" {
                        right -= width;
                    } else {
                        left += width;
                    }
                }
                _ => {}
            }
        }
        Rect::new(left, top, py_max(right, left + 1.0), py_max(bottom, top + 1.0))
    }

    fn bubble_scale(&self) -> f64 {
        py_max(self.plot.bubble_scale.unwrap_or(DEFAULT_BUBBLE_SCALE), 0.0)
    }

    fn largest_bubble(&mut self) -> f64 {
        let region = self.bubble_region();
        let scale = self.bubble_scale();
        if scale <= 0.0 {
            return 0.0;
        }
        py_min(region.width(), region.height()) * scale / (scale + BUBBLE_SCALE_HALF)
    }

    fn bubble_reference(&self) -> f64 {
        self.plot
            .series
            .iter()
            .flat_map(|s| s.bubble_sizes.iter().flatten().flatten())
            .map(|s| s.abs())
            .fold(None, |m: Option<f64>, s| Some(m.map_or(s, |m| py_max(m, s))))
            .unwrap_or(0.0)
    }

    fn bubble_diameter(&self, size: Option<f64>, reference: f64, largest: f64) -> f64 {
        let Some(size) = size else { return 0.0 };
        if reference <= 0.0 || largest <= 0.0 {
            return 0.0;
        }
        let share = py_min(size.abs() / reference, 1.0);
        if self
            .plot
            .size_represents
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_SIZE_REPRESENTS)
            == "w"
        {
            return largest * share;
        }
        largest * share.sqrt()
    }

    fn bubble_sizes(&self, index: usize, count: usize) -> Vec<Option<f64>> {
        let mut sizes: Vec<Option<f64>> = self
            .plot
            .series
            .get(index)
            .and_then(|s| s.bubble_sizes.clone())
            .filter(|b| !b.is_empty())
            .unwrap_or_default();
        if sizes.len() < count {
            sizes.resize(count, Some(1.0));
        }
        sizes.truncate(count);
        sizes
    }

    fn draw_bubbles(&mut self, index: usize, item: &Series, points: &[Option<Point>]) {
        let largest = self.largest_bubble();
        let reference = self.bubble_reference();
        let sizes = self.bubble_sizes(index, points.len());
        let show_negative = self.plot.show_negative_bubbles;
        for (point, centre) in points.iter().enumerate() {
            let Some(centre) = centre else { continue };
            let Some(size) = sizes.get(point).copied().flatten().filter(|s| *s != 0.0) else {
                continue;
            };
            if size < 0.0 && show_negative == Some(false) {
                continue;
            }
            let diameter = self.bubble_diameter(Some(size), reference, largest);
            if diameter <= 0.0 {
                continue;
            }
            let mut fill = self.point_fill(item, point);
            let mut outline = item.outline_at(point);
            if size < 0.0 && !item.has_point_fill(point) {
                fill = Some(Fill::solid("#FFFFFF"));
                outline = outline.or_else(|| {
                    Some(Outline::solid(
                        INVERTED_BAR_OUTLINE_EMU,
                        Colour::opaque(DEFAULT_AXIS_COLOR),
                    ))
                });
            }
            let half = diameter / 2.0;
            let rect = Rect::new(centre.0 - half, centre.1 - half, centre.0 + half, centre.1 + half);
            self.rect(rect, fill, outline, "ellipse");
        }
    }

    fn draw_scatter_labels(
        &mut self,
        rect: Rect,
        series: &[Series],
        x_values: &[Vec<Option<f64>>],
        scales: (Scale, Scale),
    ) {
        let categories: Vec<Vec<String>> = x_values
            .iter()
            .map(|column| {
                column
                    .iter()
                    .map(|v| v.map_or_else(String::new, |v| format_number(v, None, self.locale)))
                    .collect()
            })
            .collect();
        let sizes: Vec<Vec<Option<f64>>> = series
            .iter()
            .enumerate()
            .map(|(o, s)| self.bubble_sizes(o, s.values.len()))
            .collect();
        let bubble = self.is_bubble();
        let reference = if bubble { self.bubble_reference() } else { 0.0 };
        let largest = if bubble { self.largest_bubble() } else { 0.0 };
        let totals = super::percent_totals(series);
        for (order, item) in series.iter().enumerate() {
            let xs = x_values.get(order).cloned().unwrap_or_default();
            for (point, value) in item.values.iter().enumerate() {
                let Some(labels) = item
                    .labels_at(point)
                    .filter(|l| l.anything() && l.font.is_some())
                    .cloned()
                else {
                    continue;
                };
                let (Some(value), Some(x)) = (*value, xs.get(point).copied().flatten()) else {
                    continue;
                };
                let size = sizes.get(order).and_then(|s| s.get(point)).copied().flatten();
                let empty = Vec::new();
                let row = categories.get(order).unwrap_or(&empty);
                let at_point = LabelPoint {
                    item,
                    categories: row,
                    point,
                    value: Some(value),
                };
                let text = self.label_text(&labels, &at_point, &totals, if bubble { size } else { None });
                if text.text.is_empty() {
                    continue;
                }
                let centre = (
                    self.value_to_x(rect, x, scales.0),
                    self.value_to_y(rect, value, scales.1),
                );
                let position = labels
                    .position
                    .clone()
                    .unwrap_or_else(|| self.label_default().to_owned())
                    .to_lowercase();
                let font_size = labels.font.as_ref().map_or(0.0, ChartFont::size);
                let (mut radius, mut side, mut edge) = (
                    item.marker_size / 2.0,
                    DATA_LABEL_LINE_GAP_EM * font_size,
                    DATA_LABEL_GAP_PT,
                );
                if bubble {
                    radius = self.bubble_diameter(size, reference, largest) / 2.0;
                    (side, edge) = (BUBBLE_LABEL_SIDE_GAP_PT, BUBBLE_LABEL_EDGE_GAP_PT);
                }
                let geometry = match position.as_str() {
                    "ctr" => (centre.0, centre.1, "centre"),
                    "l" => (centre.0 - radius, centre.1, "left"),
                    "t" => (centre.0, centre.1 - radius, "outside-y"),
                    "b" => (centre.0, centre.1 + radius, "below"),
                    _ => (centre.0 + radius, centre.1, "right"),
                };
                self.place_label(&text, &labels, geometry, Some(side), Some(edge));
            }
        }
    }
}
