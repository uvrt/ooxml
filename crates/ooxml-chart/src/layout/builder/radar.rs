//! Owns: radars -- the web, spokes and ticks, the series as lines or filled areas, and
//! the value, category and data labels round them.
//!
//! Must not: lay out a pie or a cartesian plot.

use super::labels::LabelPoint;
use super::series::{minor_tick_values, tick_values};
use super::{at, blanks_as, Builder, Series};
use crate::layout::consts::{
    RADAR_LABEL_FOOT_PT, RADAR_LABEL_GAP_FRACTION, RADAR_LABEL_GAP_PT, RADAR_LABEL_MAX_FRACTION,
    RADAR_LABEL_RESERVE_LINES, RADAR_LABEL_RESERVE_PT, RADAR_VALUE_LABEL_DIGITS, TICK_MAJOR_ASCENT, TICK_MINOR_ASCENT,
};
use crate::layout::data::ChartData;
use crate::layout::numbers::format_number;
use crate::layout::paths::{rotate_past_blank, split_runs, Point};
use crate::layout::scale::{py_max, py_min, Scale};
use crate::layout::sources::{labels_shown, tick_marks, tick_span};
use crate::layout::text::{wrap_to_width, ChartFont};
use crate::layout::three_d::Rect;
use crate::model::Axis;
use ooxml_drawingml::drawable::Fill;

impl<'a> Builder<'a> {
    /// A radar chart.
    pub fn build_radar(&mut self) -> ChartData {
        let series = self.series();
        let categories = self.categories(&series, true);
        let value_axis = self.axis_for(1).or_else(|| self.axis_of_kind("valAx"));
        let category_axis = self.axis_for(0).or_else(|| self.axis_of_kind("catAx"));
        let region = self.polar_region();
        let category_font = self.label_font(category_axis);
        let value_font = self.label_font(value_axis);
        let labels = self.radar_category_labels(&categories, category_axis, &category_font, region);
        let (centre, radius) = self.radar_geometry(region, &labels, &category_font, categories.len());
        let scale = self.scale(&series, value_axis, Some(radius), None);
        let plot = Rect::new(
            centre.0 - radius,
            centre.1 - radius,
            centre.0 + radius,
            centre.1 + radius,
        );
        self.draw_background(region, Some(plot));
        self.draw_title();
        let count = categories.len();
        self.draw_radar_web(centre, radius, scale, count, value_axis);
        let filled = self.radar_style() == "filled";
        if !filled {
            self.draw_radar_spokes(centre, radius, count, value_axis);
            self.draw_radar_ticks(centre, radius, scale, count, value_axis, &value_font);
        }
        self.draw_radar_series(centre, radius, &series, count, scale);
        if filled {
            self.draw_radar_spokes(centre, radius, count, value_axis);
            self.draw_radar_ticks(centre, radius, scale, count, value_axis, &value_font);
        }
        if labels_shown(value_axis) {
            self.draw_radar_value_labels(centre, radius, scale, value_axis, &value_font);
        }
        self.draw_radar_category_labels(centre, radius, &labels, &category_font);
        self.draw_radar_data_labels(centre, radius, &series, &categories, scale);
        self.draw_legend(region, &series, false, None, 0.0);
        self.chart_data(&series, None, Some(scale))
    }

    fn radar_direction(index: usize, count: usize) -> Point {
        #[allow(clippy::cast_precision_loss)]
        let angle = 2.0 * std::f64::consts::PI * index as f64 / count.max(1) as f64;
        (angle.sin(), -angle.cos())
    }

    fn radar_category_labels(
        &self,
        categories: &[String],
        axis: Option<&Axis>,
        font: &ChartFont,
        region: Rect,
    ) -> Vec<Vec<String>> {
        if categories.is_empty() || !labels_shown(axis) {
            return Vec::new();
        }
        let cap = region.width() * RADAR_LABEL_MAX_FRACTION;
        categories.iter().map(|name| wrap_to_width(name, font, cap)).collect()
    }

    fn radar_geometry(&self, region: Rect, labels: &[Vec<String>], font: &ChartFont, count: usize) -> (Point, f64) {
        let centre = ((region.left + region.right) / 2.0, (region.top + region.bottom) / 2.0);
        let (half_width, half_height) = (region.width() / 2.0, region.height() / 2.0);
        let mut radius = py_min(half_width, half_height);
        if !labels.is_empty() {
            let box_ = font.box_for(labels.iter().flatten().map(String::as_str));
            let reserve = py_max(
                RADAR_LABEL_RESERVE_LINES * box_.line_height() - RADAR_LABEL_RESERVE_PT,
                0.0,
            );
            radius = py_min(radius, half_height - reserve);
            for (index, lines) in labels.iter().enumerate() {
                let sideways = Self::radar_direction(index, count).0.abs();
                if sideways < 1e-3 {
                    continue;
                }
                let width = super::frame::widest(font, lines.iter().map(String::as_str));
                let room = half_width - RADAR_LABEL_GAP_PT - width;
                radius = py_min(radius, room / sideways);
            }
        }
        (centre, py_max(radius, 1.0))
    }

    fn radar_radius(radius: f64, value: f64, scale: Scale) -> f64 {
        let span = scale.maximum - scale.minimum;
        if span <= 0.0 {
            return 0.0;
        }
        (value - scale.minimum) / span * radius
    }

    fn radar_point(centre: Point, index: usize, count: usize, distance: f64) -> Point {
        let (dx, dy) = Self::radar_direction(index, count);
        (centre.0 + dx * distance, centre.1 + dy * distance)
    }

    fn draw_radar_web(&mut self, centre: Point, radius: f64, scale: Scale, count: usize, axis: Option<&Axis>) {
        if count == 0 {
            return;
        }
        let ring = self.axis_outline(axis.and_then(|a| a.major_gridline_outline.as_ref()));
        for value in tick_values(scale).into_iter().skip(1) {
            let distance = Self::radar_radius(radius, value, scale);
            if distance <= 0.0 {
                continue;
            }
            let mut points: Vec<Point> = (0..count)
                .map(|i| Self::radar_point(centre, i, count, distance))
                .collect();
            if points.len() > 1 {
                if let Some(ring) = &ring {
                    if let Some(&first) = points.first() {
                        points.push(first);
                    }
                    self.polyline(&points, ring, false);
                }
            }
        }
    }

    fn draw_radar_spokes(&mut self, centre: Point, radius: f64, count: usize, axis: Option<&Axis>) {
        let Some(axis) = axis.filter(|a| count > 0 && !a.delete && a.outline.is_some()) else {
            return;
        };
        let Some(spoke) = self.resolve.outline(axis.outline.as_ref()) else {
            return;
        };
        if matches!(spoke.fill, None | Some(Fill::None)) {
            return;
        }
        for index in 0..count {
            let (x, y) = Self::radar_point(centre, index, count, radius);
            self.line(centre.0, centre.1, x, y, Some(&spoke));
        }
    }

    fn draw_radar_ticks(
        &mut self,
        centre: Point,
        radius: f64,
        scale: Scale,
        count: usize,
        axis: Option<&Axis>,
        font: &ChartFont,
    ) {
        let Some(axis) = axis.filter(|a| count > 0 && !a.delete) else {
            return;
        };
        let Some(outline) = self.axis_outline(axis.outline.as_ref()) else {
            return;
        };
        let (mut major, minor) = tick_marks(Some(axis), self.style.office_2007);
        if axis.major_gridlines {
            major = "none".into();
        }
        let ascent = font.box_.ascent;
        for (kind, length, values) in [
            (minor, ascent * TICK_MINOR_ASCENT, minor_tick_values(scale, Some(axis))),
            (major, ascent * TICK_MAJOR_ASCENT, tick_values(scale)),
        ] {
            let Some((low, high)) = tick_span(&kind, length, 1.0) else {
                continue;
            };
            for index in 0..count {
                let (dx, dy) = Self::radar_direction(index, count);
                let (nx, ny) = (dy, -dx);
                for &value in &values {
                    let distance = Self::radar_radius(radius, value, scale);
                    if distance <= 1e-9 {
                        continue;
                    }
                    let (x, y) = (centre.0 + dx * distance, centre.1 + dy * distance);
                    self.line(x + nx * low, y + ny * low, x + nx * high, y + ny * high, Some(&outline));
                }
            }
        }
    }

    fn draw_radar_series(&mut self, centre: Point, radius: f64, series: &[Series], count: usize, scale: Scale) {
        if count == 0 {
            return;
        }
        let filled = self.radar_style() == "filled";
        let blanks = blanks_as(self.chart.display_blanks_as.as_deref(), false);
        for item in series {
            let mut points: Vec<Option<Point>> = Vec::new();
            for index in 0..count {
                let value = match at(&item.values, index) {
                    Some(v) => v,
                    None if blanks == "zero" => scale.minimum,
                    None if blanks == "span" => continue,
                    None => {
                        points.push(None);
                        continue;
                    }
                };
                points.push(Some(Self::radar_point(
                    centre,
                    index,
                    count,
                    Self::radar_radius(radius, value, scale),
                )));
            }
            let drawn: Vec<Point> = points.iter().flatten().copied().collect();
            let closed = drawn.len() == count && count > 2;
            if filled {
                if drawn.len() > 2 {
                    self.polygon(&drawn, item.fill.clone(), item.outline.clone());
                }
                continue;
            }
            if let Some(line) = &item.line {
                if closed {
                    let mut ring = drawn.clone();
                    if let Some(&first) = drawn.first() {
                        ring.push(first);
                    }
                    self.polyline(&ring, line, false);
                } else {
                    for run in split_runs(&rotate_past_blank(&points)) {
                        if run.len() > 1 {
                            self.polyline(&run, line, false);
                        }
                    }
                }
            }
            for point in points.iter().flatten() {
                if item.marker_symbol.is_some() {
                    self.marker(*point, item);
                }
            }
        }
    }

    fn draw_radar_value_labels(
        &mut self,
        centre: Point,
        radius: f64,
        scale: Scale,
        axis: Option<&Axis>,
        font: &ChartFont,
    ) {
        let box_ = font.box_;
        let right = centre.0 - RADAR_VALUE_LABEL_DIGITS * font.width("0");
        for (value, text) in self.tick_texts(scale, axis) {
            if text.is_empty() {
                continue;
            }
            let y = centre.1 - Self::radar_radius(radius, value, scale);
            let width = font.width(&text) + box_.size;
            let body = self.label_body(&text, font, "r", None);
            self.text(body, right - width, width, y + box_.ink_centre(), &box_);
        }
    }

    fn draw_radar_category_labels(&mut self, centre: Point, radius: f64, labels: &[Vec<String>], font: &ChartFont) {
        let count = labels.len();
        let box_ = font.box_;
        let distance = radius * (1.0 + RADAR_LABEL_GAP_FRACTION);
        for (index, lines) in labels.iter().enumerate() {
            if !lines.iter().any(|l| !l.is_empty()) {
                continue;
            }
            let (dx, dy) = Self::radar_direction(index, count);
            let (anchor_x, anchor_y) = (centre.0 + dx * distance, centre.1 + dy * distance);
            let width = lines.iter().map(|l| font.width(l)).fold(f64::NEG_INFINITY, py_max);
            #[allow(clippy::cast_precision_loss)]
            let block = box_.line_height() * lines.len() as f64;
            let left = if dx > 1e-3 {
                anchor_x
            } else if dx < -1e-3 {
                anchor_x - width
            } else {
                anchor_x - width / 2.0
            };
            let top = if dx.abs() > 1e-3 {
                anchor_y - (block + RADAR_LABEL_FOOT_PT) / 2.0
            } else if dy > 0.0 {
                anchor_y
            } else {
                anchor_y - block - RADAR_LABEL_FOOT_PT
            };
            for (line_index, line) in lines.iter().enumerate() {
                let body = self.label_body(line, font, "ctr", None);
                #[allow(clippy::cast_precision_loss)]
                let baseline = top + line_index as f64 * box_.line_height() + box_.ascent;
                self.text(body, left - box_.size / 2.0, width + box_.size, baseline, &box_);
            }
        }
    }

    fn draw_radar_data_labels(
        &mut self,
        centre: Point,
        radius: f64,
        series: &[Series],
        categories: &[String],
        scale: Scale,
    ) {
        let count = categories.len();
        for item in series {
            for index in 0..count {
                let Some(value) = at(&item.values, index) else { continue };
                let Some(labels) = item.labels_at(index).filter(|l| l.anything() && l.font.is_some()) else {
                    continue;
                };
                let own = self.own_label(
                    labels,
                    &LabelPoint {
                        item,
                        categories,
                        point: index,
                        value: Some(value),
                    },
                    None,
                    None,
                );
                let mut parts = Vec::new();
                if labels.show_series {
                    if let Some(name) = item.name.as_ref().filter(|n| !n.is_empty()) {
                        parts.push(name.clone());
                    }
                }
                if labels.show_category {
                    if let Some(category) = categories.get(index).filter(|c| !c.is_empty()) {
                        parts.push(category.clone());
                    }
                }
                if labels.show_value {
                    let code = labels.number_format.as_deref().or(item.format_code.as_deref());
                    parts.push(format_number(value, code, self.locale));
                }
                let parts = Self::with_range(labels, item, index, parts);
                if parts.is_empty() && own.is_none() {
                    continue;
                }
                let distance = Self::radar_radius(radius, value, scale) + item.marker_size;
                let (x, y) = Self::radar_point(centre, index, count, distance);
                if let Some(font) = labels.font.clone() {
                    self.centred_label(&parts, &font, x, y, own.as_ref());
                }
            }
        }
    }
}
