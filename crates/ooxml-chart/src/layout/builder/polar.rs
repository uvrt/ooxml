//! Owns: pies, doughnuts and of-pies (slices, explosion, the second plot and its
//! connector, slice labels) and radars (the web, spokes, ticks, series, value, category and
//! data labels).
//!
//! Must not: lay out a cartesian plot.

use super::labels::LabelPoint;
use super::primitives::Slice;
use super::series::{minor_tick_values, tick_values};
use super::{at, blanks_as, Builder, Series};
use crate::layout::consts::{
    pie_label_radius, DEFAULT_HOLE_SIZE, DEFAULT_OF_PIE_GAP_WIDTH, DEFAULT_OF_PIE_SPLIT, DEFAULT_SECOND_PIE_SIZE,
    MIN_BAR_SLOTS, OF_PIE_AUTO_DIVISOR, OF_PIE_BAR_GAP_DIVISOR, OF_PIE_OTHER_ANGLE, RADAR_LABEL_FOOT_PT,
    RADAR_LABEL_GAP_FRACTION, RADAR_LABEL_GAP_PT, RADAR_LABEL_MAX_FRACTION, RADAR_LABEL_RESERVE_LINES,
    RADAR_LABEL_RESERVE_PT, RADAR_VALUE_LABEL_DIGITS, TICK_MAJOR_ASCENT, TICK_MINOR_ASCENT,
};
use crate::layout::data::ChartData;
use crate::layout::numbers::format_number;
use crate::layout::paths::{percent_shares, rotate_past_blank, split_runs, tangent_point, Point};
use crate::layout::scale::{py_max, py_min, Scale};
use crate::layout::sources::{labels_shown, tick_marks, tick_span};
use crate::layout::text::{wrap_to_width, ChartFont};
use crate::layout::three_d::Rect;
use crate::model::Axis;
use crate::pymath;
use ooxml_drawingml::drawable::Fill;
use std::collections::BTreeSet;

/// Slice label lines and their place.
struct SliceLabel<'s> {
    item: &'s Series,
    index: usize,
    value: Option<f64>,
    percent: String,
    x: f64,
    y: f64,
}

impl<'a> Builder<'a> {
    /// A pie, doughnut or of-pie chart.
    pub fn build_polar(&mut self, of_pie: bool) -> ChartData {
        let series = self.series();
        let categories = self.categories(&series, true);
        let region = self.polar_region();
        self.draw_background(region, None);
        self.draw_title();
        if of_pie {
            self.draw_of_pie(region, &series, &categories);
        } else {
            self.draw_pie(region, &series, &categories);
        }
        self.draw_legend(region, &series, true, Some(&categories), 0.0);
        self.chart_data(&series, None, None)
    }

    fn draw_pie(&mut self, region: Rect, series: &[Series], categories: &[String]) {
        if series.is_empty() {
            return;
        }
        let mut radius = py_min(region.width(), region.height()) / 2.0;
        let centre_x = (region.left + region.right) / 2.0;
        let centre_y = (region.top + region.bottom) / 2.0;
        if radius <= 0.0 {
            return;
        }
        let mut explosion = self
            .plot
            .series
            .iter()
            .map(|s| s.explosion.unwrap_or(0.0))
            .fold(None, |m: Option<f64>, e| Some(m.map_or(e, |m| py_max(m, e))))
            .unwrap_or(0.0);
        for source in &self.plot.series {
            for stated in &source.data_points {
                if let Some(e) = stated.explosion.filter(|e| *e != 0.0) {
                    explosion = py_max(explosion, e);
                }
            }
        }
        if explosion > 0.0 {
            radius /= 1.0 + explosion / 100.0;
        }
        let hole = self.plot.hole_size.unwrap_or(DEFAULT_HOLE_SIZE) / 100.0;
        let hole = py_min(py_max(hole, 0.0), 0.95);
        let inner_base = radius * hole;
        #[allow(clippy::cast_precision_loss)]
        let band = (radius - inner_base) / series.len().max(1) as f64;
        let start_angle = self.plot.first_slice_angle.filter(|a| *a != 0.0).unwrap_or(0.0);
        for (ring, item) in series.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let inner = inner_base + ring as f64 * band;
            let outer = inner + band;
            let total = pymath::sum(item.values.iter().flatten().map(|v| v.abs()));
            if total <= 0.0 {
                continue;
            }
            let mut angle = start_angle;
            for (point, value) in item.values.iter().enumerate() {
                let Some(value) = value.filter(|v| *v != 0.0) else {
                    continue;
                };
                let sweep = value.abs() / total * 360.0;
                let offset = self.slice_offset(point, angle + sweep / 2.0, radius);
                let fill = self.point_fill(item, point);
                self.slice(
                    &Slice {
                        cx: centre_x + offset.0,
                        cy: centre_y + offset.1,
                        inner,
                        outer,
                        start: angle,
                        sweep,
                    },
                    fill,
                    item.outline_at(point),
                );
                angle += sweep;
            }
        }
        self.draw_pie_labels(
            series,
            categories,
            (centre_x, centre_y),
            radius,
            inner_base,
            start_angle,
        );
    }

    fn slice_offset(&self, point: usize, mid_angle: f64, radius: f64) -> (f64, f64) {
        let explosion = self.point_explosion(point);
        if explosion == 0.0 {
            return (0.0, 0.0);
        }
        let distance = radius * explosion / 100.0;
        let radians = mid_angle.to_radians();
        (distance * radians.sin(), -distance * radians.cos())
    }

    fn point_explosion(&self, point: usize) -> f64 {
        let key = i64::try_from(point).unwrap_or(i64::MAX);
        for source in &self.plot.series {
            for stated in &source.data_points {
                if stated.index == key {
                    if let Some(e) = stated.explosion.filter(|e| *e != 0.0) {
                        return e;
                    }
                }
            }
            if let Some(e) = source.explosion.filter(|e| *e != 0.0) {
                return e;
            }
        }
        0.0
    }

    /// A point's fill: its own, else its varied accent, else the series'.
    pub fn point_fill(&self, item: &Series, point: usize) -> Option<Fill> {
        let key = i64::try_from(point).unwrap_or(i64::MAX);
        item.point_fills
            .get(&key)
            .or_else(|| item.vary_fills.get(point))
            .cloned()
            .or_else(|| item.fill.clone())
    }

    fn draw_pie_labels(
        &mut self,
        series: &[Series],
        categories: &[String],
        centre: Point,
        radius: f64,
        inner_base: f64,
        start: f64,
    ) {
        #[allow(clippy::cast_precision_loss)]
        let band = (radius - inner_base) / series.len().max(1) as f64;
        for (ring, item) in series.iter().enumerate() {
            let total = pymath::sum(item.values.iter().flatten().map(|v| v.abs()));
            if total <= 0.0 {
                continue;
            }
            let shares = percent_shares(&item.values.iter().map(|v| v.map_or(0.0, f64::abs)).collect::<Vec<_>>());
            let mut angle = start;
            for (point, value) in item.values.iter().enumerate() {
                let Some(value) = value.filter(|v| *v != 0.0) else {
                    continue;
                };
                let sweep = value.abs() / total * 360.0;
                let mid = angle + sweep / 2.0;
                angle += sweep;
                let Some(labels) = item.labels_at(point).filter(|l| l.anything() && l.font.is_some()) else {
                    continue;
                };
                let percent = format!("{}%", shares.get(point).copied().unwrap_or(0));
                let own = self.own_label(
                    labels,
                    &LabelPoint {
                        item,
                        categories,
                        point,
                        value: Some(value),
                    },
                    Some(percent.clone()),
                    None,
                );
                let parts = self.slice_parts(labels, item, categories, point, Some(value), &percent);
                if parts.is_empty() && own.is_none() {
                    continue;
                }
                let fraction = pie_label_radius(labels.position.as_deref().unwrap_or("bestFit")).unwrap_or(0.710);
                #[allow(clippy::cast_precision_loss)]
                let outer = inner_base + (ring + 1) as f64 * band;
                let distance = outer * fraction;
                let offset = self.slice_offset(point, mid, radius);
                let radians = mid.to_radians();
                let x = centre.0 + offset.0 + distance * radians.sin();
                let y = centre.1 + offset.1 - distance * radians.cos();
                let font = labels.font.clone();
                if let Some(font) = font {
                    self.centred_label(&parts, &font, x, y, own.as_ref());
                }
            }
        }
    }

    /// A slice label's parts: series, category, percent and value, with the range.
    fn slice_parts(
        &self,
        labels: &super::Labels,
        item: &Series,
        categories: &[String],
        point: usize,
        value: Option<f64>,
        percent: &str,
    ) -> Vec<String> {
        let mut parts = Vec::new();
        if labels.show_series {
            if let Some(name) = item.name.as_ref().filter(|n| !n.is_empty()) {
                parts.push(name.clone());
            }
        }
        if labels.show_category {
            if let Some(category) = categories.get(point).filter(|c| !c.is_empty()) {
                parts.push(category.clone());
            }
        }
        if labels.show_percent {
            parts.push(percent.to_owned());
        }
        if labels.show_value {
            if let Some(value) = value {
                let code = labels.number_format.as_deref().or(item.format_code.as_deref());
                parts.push(format_number(value, code, self.locale));
            }
        }
        Self::with_range(labels, item, point, parts)
    }

    fn of_pie_split(&self, values: &[Option<f64>]) -> BTreeSet<usize> {
        let count = values.len();
        if count == 0 {
            return BTreeSet::new();
        }
        let kind = crate::layout::text::py_strip(self.plot.split_type.as_deref().unwrap_or(DEFAULT_OF_PIE_SPLIT));
        let position = self.plot.split_position;
        match kind {
            "cust" => self
                .plot
                .custom_split
                .iter()
                .filter_map(|i| usize::try_from(*i).ok())
                .filter(|i| *i < count)
                .collect(),
            "val" => {
                let threshold = position.unwrap_or(0.0);
                values
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.is_some_and(|v| v < threshold))
                    .map(|(i, _)| i)
                    .collect()
            }
            "percent" => {
                let total = pymath::sum(values.iter().flatten().map(|v| v.abs()));
                if total <= 0.0 {
                    return BTreeSet::new();
                }
                let threshold = position.unwrap_or(0.0);
                values
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.is_some_and(|v| v.abs() / total * 100.0 < threshold))
                    .map(|(i, _)| i)
                    .collect()
            }
            _ => {
                let moved = if kind == "pos" {
                    #[allow(clippy::cast_possible_truncation)]
                    let m = position.map_or(0, |p| p.trunc() as i64);
                    m
                } else {
                    i64::try_from(count.div_ceil(OF_PIE_AUTO_DIVISOR)).unwrap_or(0)
                };
                let moved = usize::try_from(moved.max(0)).unwrap_or(0).min(count);
                (count - moved..count).collect()
            }
        }
    }

    fn of_pie_geometry(&self, region: Rect) -> (f64, f64, f64, f64, bool) {
        let is_bar = crate::layout::text::py_strip(self.plot.of_pie_type.as_deref().unwrap_or("pie")) == "bar";
        let fraction = py_max(self.plot.second_pie_size.unwrap_or(DEFAULT_SECOND_PIE_SIZE), 0.0) / 100.0;
        let gap = py_max(self.plot.gap_width.unwrap_or(DEFAULT_OF_PIE_GAP_WIDTH), 0.0) / 100.0;
        let divisor = if is_bar {
            2.0 + fraction + gap / OF_PIE_BAR_GAP_DIVISOR
        } else {
            2.0 + 2.0 * fraction + gap
        };
        let divisor = py_max(divisor, MIN_BAR_SLOTS);
        let mut radius = region.width() / divisor;
        radius = py_min(radius, region.height() / 2.0);
        if is_bar && fraction > 0.0 {
            radius = py_min(radius, region.height() / (2.0 * fraction));
        }
        let left = region.left + (region.width() - divisor * radius) / 2.0;
        let right = left + divisor * radius;
        let second_x = if is_bar {
            right - fraction * radius / 2.0
        } else {
            right - fraction * radius
        };
        (radius, left + radius, fraction, second_x, is_bar)
    }

    fn draw_of_pie(&mut self, region: Rect, series: &[Series], categories: &[String]) {
        let Some(item) = series.first() else { return };
        let values = &item.values;
        let total = pymath::sum(values.iter().flatten().map(|v| v.abs()));
        if total <= 0.0 {
            return;
        }
        let moved = self.of_pie_split(values);
        let (radius, first_x, fraction, second_x, is_bar) = self.of_pie_geometry(region);
        if radius <= 0.0 {
            return;
        }
        let middle_y = (region.top + region.bottom) / 2.0;
        let other = pymath::sum(moved.iter().filter_map(|i| at(values, *i)).map(f64::abs));
        let other_sweep = other / total * 360.0;
        let start = OF_PIE_OTHER_ANGLE + other_sweep / 2.0;
        let kept: Vec<usize> = (0..values.len()).filter(|i| !moved.contains(i)).collect();
        let mut angle = start;
        let mut corners: Vec<Point> = Vec::new();
        for &index in &kept {
            let Some(value) = at(values, index).filter(|v| *v != 0.0) else {
                continue;
            };
            let sweep = value.abs() / total * 360.0;
            let fill = self.point_fill(item, index);
            self.slice(
                &Slice {
                    cx: first_x,
                    cy: middle_y,
                    inner: 0.0,
                    outer: radius,
                    start: angle,
                    sweep,
                },
                fill,
                item.outline_at(index),
            );
            angle += sweep;
        }
        if other > 0.0 {
            let fill = self.of_pie_other_fill(item, values.len());
            self.slice(
                &Slice {
                    cx: first_x,
                    cy: middle_y,
                    inner: 0.0,
                    outer: radius,
                    start: angle,
                    sweep: other_sweep,
                },
                fill,
                item.outline.clone(),
            );
            for edge in [angle, angle + other_sweep] {
                let radians = edge.to_radians();
                corners.push((first_x + radius * radians.sin(), middle_y - radius * radians.cos()));
            }
        }
        let second_radius = fraction * radius;
        let moved: Vec<usize> = moved.into_iter().collect();
        if is_bar {
            self.draw_of_pie_bar(item, &moved, other, (second_x, middle_y), second_radius);
        } else {
            self.draw_of_pie_second(item, &moved, other, (second_x, middle_y), second_radius, start);
        }
        if self.plot.series_lines.is_some() && !corners.is_empty() {
            self.draw_of_pie_connector(&corners, (second_x, middle_y), second_radius, is_bar);
        }
        let geometry = OfPie {
            first_x,
            middle_y,
            radius,
            second_x,
            second_radius,
            start,
            is_bar,
            total,
            other,
        };
        self.draw_of_pie_labels(item, categories, &kept, &moved, &geometry);
    }

    fn of_pie_other_fill(&self, item: &Series, index: usize) -> Option<Fill> {
        if !self.style.accents.is_empty() && self.vary_colors() {
            return Some(Fill::Solid(self.cycle_accent(index, index + 1)));
        }
        item.fill.clone()
    }

    fn draw_of_pie_second(
        &mut self,
        item: &Series,
        moved: &[usize],
        other: f64,
        centre: Point,
        radius: f64,
        start: f64,
    ) {
        if other <= 0.0 || radius <= 0.0 {
            return;
        }
        let mut angle = start;
        for &index in moved {
            let Some(value) = at(&item.values, index).filter(|v| *v != 0.0) else {
                continue;
            };
            let sweep = value.abs() / other * 360.0;
            let fill = self.point_fill(item, index);
            self.slice(
                &Slice {
                    cx: centre.0,
                    cy: centre.1,
                    inner: 0.0,
                    outer: radius,
                    start: angle,
                    sweep,
                },
                fill,
                item.outline_at(index),
            );
            angle += sweep;
        }
    }

    fn draw_of_pie_bar(&mut self, item: &Series, moved: &[usize], other: f64, centre: Point, radius: f64) {
        if other <= 0.0 || radius <= 0.0 {
            return;
        }
        let (width, height) = (radius, 2.0 * radius);
        let left = centre.0 - width / 2.0;
        let mut top = centre.1 - height / 2.0;
        for &index in moved {
            let Some(value) = at(&item.values, index).filter(|v| *v != 0.0) else {
                continue;
            };
            let span = value.abs() / other * height;
            let fill = self.point_fill(item, index);
            self.rect(
                Rect::new(left, top, left + width, top + span),
                fill,
                item.outline_at(index),
                "rect",
            );
            top += span;
        }
    }

    fn draw_of_pie_connector(&mut self, corners: &[Point], centre: Point, radius: f64, is_bar: bool) {
        let outline = self
            .plot
            .series_lines
            .as_ref()
            .and_then(|l| self.axis_outline(l.outline.as_ref()));
        let Some(outline) = outline else { return };
        if radius <= 0.0 {
            return;
        }
        let mut sorted = corners.to_vec();
        sorted.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let (Some(&upper), Some(&lower)) = (sorted.first(), sorted.get(1)) else {
            return;
        };
        if is_bar {
            let left = centre.0 - radius / 2.0;
            self.line(upper.0, upper.1, left, centre.1 - radius, Some(&outline));
            self.line(lower.0, lower.1, left, centre.1 + radius, Some(&outline));
            return;
        }
        for (corner, want_upper) in [(upper, true), (lower, false)] {
            if let Some(point) = tangent_point(corner, centre, radius, want_upper) {
                self.line(corner.0, corner.1, point.0, point.1, Some(&outline));
            }
        }
    }

    fn draw_of_pie_labels(&mut self, item: &Series, categories: &[String], kept: &[usize], moved: &[usize], g: &OfPie) {
        let shares = percent_shares(&item.values.iter().map(|v| v.map_or(0.0, f64::abs)).collect::<Vec<_>>());
        let fraction = pie_label_radius(
            item.labels
                .as_ref()
                .and_then(|l| l.position.as_deref())
                .unwrap_or("bestFit"),
        )
        .unwrap_or(0.710);
        let mut places: Vec<SliceLabel<'_>> = Vec::new();
        let mut place = |index: usize, x: f64, y: f64| {
            let percent = format!("{}%", shares.get(index).copied().unwrap_or(0));
            places.push(SliceLabel {
                item,
                index,
                value: at(&item.values, index),
                percent,
                x,
                y,
            });
        };
        let mut angle = g.start;
        for &index in kept {
            let Some(value) = at(&item.values, index).filter(|v| *v != 0.0) else {
                continue;
            };
            let sweep = value.abs() / g.total * 360.0;
            let radians = (angle + sweep / 2.0).to_radians();
            place(
                index,
                g.first_x + g.radius * fraction * radians.sin(),
                g.middle_y - g.radius * fraction * radians.cos(),
            );
            angle += sweep;
        }
        if g.other > 0.0 {
            if g.is_bar {
                let mut top = g.middle_y - g.second_radius;
                for &index in moved {
                    let Some(value) = at(&item.values, index).filter(|v| *v != 0.0) else {
                        continue;
                    };
                    let span = value.abs() / g.other * 2.0 * g.second_radius;
                    place(index, g.second_x, top + span / 2.0);
                    top += span;
                }
            } else {
                let mut angle = g.start;
                for &index in moved {
                    let Some(value) = at(&item.values, index).filter(|v| *v != 0.0) else {
                        continue;
                    };
                    let sweep = value.abs() / g.other * 360.0;
                    let radians = (angle + sweep / 2.0).to_radians();
                    place(
                        index,
                        g.second_x + g.second_radius * fraction * radians.sin(),
                        g.middle_y - g.second_radius * fraction * radians.cos(),
                    );
                    angle += sweep;
                }
            }
        }
        for label in places {
            self.emit_slice_label(&label, categories);
        }
    }

    fn emit_slice_label(&mut self, label: &SliceLabel<'_>, categories: &[String]) {
        let item = label.item;
        let Some(labels) = item.labels_at(label.index).filter(|l| l.anything() && l.font.is_some()) else {
            return;
        };
        let Some(font) = labels.font.clone() else { return };
        let at_point = LabelPoint {
            item,
            categories,
            point: label.index,
            value: label.value,
        };
        let own = self.own_label(labels, &at_point, Some(label.percent.clone()), None);
        if let Some(own) = own {
            self.centred_label(&[], &font, label.x, label.y, Some(&own));
            return;
        }
        let parts = self.slice_parts(labels, item, categories, label.index, label.value, &label.percent);
        if !parts.is_empty() {
            self.centred_label(&parts, &font, label.x, label.y, None);
        }
    }
}

/// An of-pie's geometry, for its labels.
struct OfPie {
    first_x: f64,
    middle_y: f64,
    radius: f64,
    second_x: f64,
    second_radius: f64,
    start: f64,
    is_bar: bool,
    total: f64,
    other: f64,
}

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
