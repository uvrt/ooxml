//! Owns: scatter and bubble charts -- two value axes and where they cross, the plot
//! rectangle, gridlines, series and bubbles sized by area or width, and point labels.
//!
//! Must not: lay out a category axis.

use super::cartesian::Along;

use super::frame::{texts, widest};
use super::labels::LabelPoint;
use super::marks::TickRun;
use super::series::{minor_tick_values, tick_values};
use super::{Builder, Series};
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

/// A scatter's plot rectangle, axes, scales and where the axes cross.
#[derive(Clone, Copy)]
struct ScatterAxes<'a> {
    rect: Rect,
    axes: (Option<&'a Axis>, Option<&'a Axis>),
    scales: (Scale, Scale),
    cross: (f64, f64),
}

impl<'a> Builder<'a> {
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
