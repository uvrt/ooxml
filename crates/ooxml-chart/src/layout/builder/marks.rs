//! Owns: a cartesian plot's marks -- bars (clustered, stacked, inverted, 3-D prisms queued),
//! areas, lines with markers, a stock chart's high-low lines and up-down bars -- and its
//! gridlines, axis lines and tick marks, with where each category stands.
//!
//! Must not: draw labels or the legend.

use super::series::{minor_tick_values, tick_values};
use super::{at, blanks_as, percent_totals, Builder, Prism, Series};
use crate::layout::consts::{
    marker_preset, DEFAULT_AREA_CROSS_BETWEEN, DEFAULT_AXIS_COLOR, DEFAULT_AXIS_LINE_EMU, DEFAULT_DOWN_BAR_FILL,
    DEFAULT_GAP_WIDTH, DEFAULT_UP_BAR_FILL, DEFAULT_UP_DOWN_GAP_WIDTH, INVERTED_BAR_OUTLINE_EMU, MIN_BAR_SLOTS,
    TICK_MAJOR_ASCENT, TICK_MINOR_ASCENT,
};
use crate::layout::paths::{split_runs, Point};
use crate::layout::scale::{py_max, py_min, Scale};
use crate::layout::sources::{tick_marks, tick_span};
use crate::layout::text::ChartFont;
use crate::layout::three_d::Rect;
use crate::model::Axis;
use ooxml_drawingml::drawable::{Colour, Fill, Outline};

/// Tick marks along one axis line: which way it runs, where it stands, which side is out,
/// and the major and minor positions.
#[derive(Clone, Copy)]
pub(crate) struct TickRun<'t> {
    pub vertical: bool,
    pub at: f64,
    pub outward: f64,
    pub majors: &'t [f64],
    pub minors: &'t [f64],
}

/// One bar to draw: where it stands across and along.
struct Bar {
    point: usize,
    offset: f64,
    size: f64,
    start: f64,
    end: f64,
}

impl<'a> Builder<'a> {
    /// The value gridlines (minor, then major), skipping the category axis' own line.
    pub fn draw_gridlines(&mut self, rect: Rect, scale: Scale, axis: Option<&Axis>, skip_crossing: bool) {
        let Some(axis) = axis else { return };
        let horizontal = self.horizontal();
        let crossing = if skip_crossing {
            Some(self.category_axis_position(rect, scale, None, true))
        } else {
            None
        };
        let place = |b: &Self, v: f64| {
            if horizontal {
                b.value_to_x(rect, v, scale)
            } else {
                b.value_to_y(rect, v, scale)
            }
        };
        let draw = |b: &mut Self, at: f64, outline: Option<&Outline>| {
            if horizontal {
                b.line(at, rect.top, at, rect.bottom, outline);
            } else {
                b.line(rect.left, at, rect.right, at, outline);
            }
        };
        if axis.minor_gridlines {
            let outline = self.axis_outline(axis.minor_gridline_outline.as_ref());
            let majors: Vec<f64> = tick_values(scale).into_iter().map(|v| place(self, v)).collect();
            for value in minor_tick_values(scale, Some(axis)) {
                let at = place(self, value);
                if majors.iter().any(|m| (at - m).abs() < 0.01) || crossing.is_some_and(|c| (at - c).abs() < 0.01) {
                    continue;
                }
                draw(self, at, outline.as_ref());
            }
        }
        if !axis.major_gridlines {
            return;
        }
        let outline = self.axis_outline(axis.major_gridline_outline.as_ref());
        for value in tick_values(scale) {
            let at = place(self, value);
            if crossing.is_some_and(|c| (at - c).abs() < 0.01) {
                continue;
            }
            draw(self, at, outline.as_ref());
        }
    }

    /// The category gridlines: between bands, or on each point.
    pub fn draw_category_gridlines(&mut self, rect: Rect, axis: Option<&Axis>, count: usize) {
        let Some(axis) = axis.filter(|a| a.major_gridlines && count > 0 && !self.is_three_d()) else {
            return;
        };
        let outline = self.axis_outline(axis.major_gridline_outline.as_ref());
        let horizontal = self.horizontal();
        let length = if horizontal { rect.height() } else { rect.width() };
        let positions: Vec<f64> = if self.points_on_ticks() && !horizontal {
            self.category_positions(rect, count)
        } else {
            #[allow(clippy::cast_precision_loss)]
            let step = length / count as f64;
            let start = if horizontal { rect.bottom } else { rect.left };
            let sign = if horizontal { -1.0 } else { 1.0 };
            #[allow(clippy::cast_precision_loss)]
            (0..=count).map(|i| start + sign * step * i as f64).collect()
        };
        for at in positions {
            if horizontal {
                self.line(rect.left, at, rect.right, at, outline.as_ref());
            } else {
                self.line(at, rect.top, at, rect.bottom, outline.as_ref());
            }
        }
    }

    /// Bars: each category's cluster or stack.
    pub fn draw_bars(&mut self, rect: Rect, series: &[Series], categories: &[String], scale: Scale) {
        if series.is_empty() || categories.is_empty() {
            return;
        }
        let horizontal = self.horizontal();
        let grouping = self.grouping_or("clustered").to_owned();
        let stacked = matches!(grouping.as_str(), "stacked" | "percentStacked");
        #[allow(clippy::cast_precision_loss)]
        let band = (if horizontal { rect.height() } else { rect.width() }) / categories.len() as f64;
        let gap_width = self.plot.gap_width.unwrap_or(DEFAULT_GAP_WIDTH);
        let slots = if stacked { 1 } else { series.len() };
        #[allow(clippy::cast_precision_loss)]
        let slots_f = slots as f64;
        let overlap = self.plot.overlap.unwrap_or(0.0);
        let slack = slots_f + gap_width / 100.0 - (slots_f - 1.0) * overlap / 100.0;
        let bar_size = band / py_max(slack, MIN_BAR_SLOTS);
        let step = bar_size * (1.0 - overlap / 100.0);
        let cluster = bar_size + step * (slots_f - 1.0);
        let percent = grouping == "percentStacked";
        let totals = if percent { Some(percent_totals(series)) } else { None };
        let base = self.bar_base(scale);
        if self.draws_a_scene() {
            self.group.prisms = Some(Vec::new());
        }
        for point in 0..categories.len() {
            let place = self.category_slot(point, categories.len());
            #[allow(clippy::cast_precision_loss)]
            let band_start = if horizontal {
                rect.bottom - (place + 1) as f64 * band
            } else {
                rect.left + place as f64 * band
            };
            let centre = band_start + band / 2.0;
            let (mut positive_base, mut negative_base) = (0.0, 0.0);
            for (order, item) in series.iter().enumerate() {
                let mut value = match at(&item.values, point) {
                    Some(v) => v,
                    None if self.chart.display_blanks_as.as_deref() == Some("zero") => 0.0,
                    None => continue,
                };
                if let Some(totals) = &totals {
                    let total = totals.get(point).copied().unwrap_or(0.0);
                    value = if total == 0.0 { 0.0 } else { value / total };
                }
                let (start, end, mut slot) = if stacked {
                    let start = if value >= 0.0 { positive_base } else { negative_base };
                    let end = start + value;
                    if value >= 0.0 {
                        positive_base = end;
                    } else {
                        negative_base = end;
                    }
                    (start, end, 0)
                } else {
                    (base, value, order)
                };
                if !stacked && horizontal && self.rules.bars_upward {
                    slot = slots - 1 - order;
                }
                #[allow(clippy::cast_precision_loss)]
                let mut offset = centre - cluster / 2.0 + slot as f64 * step;
                if self.categories_reversed() {
                    offset = 2.0 * centre - offset - bar_size;
                }
                self.bar(
                    rect,
                    item,
                    &Bar {
                        point,
                        offset,
                        size: bar_size,
                        start,
                        end,
                    },
                    scale,
                    horizontal,
                );
            }
        }
        self.paint_prisms();
    }

    /// Areas: each series' band from its base (stacked on the ones before it).
    pub fn draw_areas(&mut self, rect: Rect, series: &[Series], categories: &[String], scale: Scale) {
        if categories.is_empty() || series.is_empty() {
            return;
        }
        let xs = self.category_positions(rect, categories.len());
        let blanks = blanks_as(self.chart.display_blanks_as.as_deref(), true);
        let stacked = self.stacked("standard");
        let percent = self.grouping_or("standard") == "percentStacked";
        let totals = if percent { Some(percent_totals(series)) } else { None };
        let zero = self.value_to_y(rect, 0.0, scale);
        let mut bases = vec![0.0; categories.len()];
        for item in series {
            let mut run: Vec<Option<(f64, f64, f64)>> = Vec::new();
            for index in 0..categories.len() {
                let mut value = match at(&item.values, index) {
                    Some(v) => v,
                    None if blanks == "zero" => 0.0,
                    None if blanks == "span" => continue,
                    None => {
                        run.push(None);
                        continue;
                    }
                };
                if let Some(totals) = &totals {
                    let total = totals.get(index).copied().unwrap_or(0.0);
                    value = if total == 0.0 { 0.0 } else { value / total };
                }
                let (top, bottom) = if stacked {
                    let start = bases.get(index).copied().unwrap_or(0.0);
                    if let Some(b) = bases.get_mut(index) {
                        *b = start + value;
                    }
                    (
                        self.value_to_y(rect, start + value, scale),
                        self.value_to_y(rect, start, scale),
                    )
                } else {
                    (self.value_to_y(rect, value, scale), zero)
                };
                run.push(Some((xs.get(index).copied().unwrap_or(0.0), top, bottom)));
            }
            for stretch in split_runs(&run) {
                if stretch.len() < 2 {
                    continue;
                }
                let mut points: Vec<Point> = stretch.iter().map(|&(x, top, _)| (x, top)).collect();
                points.extend(stretch.iter().rev().map(|&(x, _, bottom)| (x, bottom)));
                self.polygon(&points, item.fill.clone(), item.outline.clone());
            }
        }
    }

    /// Lines with their markers.
    pub fn draw_lines(&mut self, rect: Rect, series: &[Series], categories: &[String], scale: Scale) {
        if categories.is_empty() {
            return;
        }
        let xs = self.category_positions(rect, categories.len());
        let blanks = blanks_as(self.chart.display_blanks_as.as_deref(), false);
        for item in series {
            let mut points: Vec<Option<Point>> = Vec::new();
            for index in 0..categories.len() {
                let value = match at(&item.values, index) {
                    Some(v) => v,
                    None if blanks == "zero" => 0.0,
                    None if blanks == "span" => continue,
                    None => {
                        points.push(None);
                        continue;
                    }
                };
                points.push(Some((
                    xs.get(index).copied().unwrap_or(0.0),
                    self.value_to_y(rect, value, scale),
                )));
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
    }

    /// A stock chart's high-low lines.
    pub fn draw_hi_low_lines(&mut self, rect: Rect, series: &[Series], categories: &[String], scale: Scale) {
        let Some(lines) = &self.plot.hi_low_lines else { return };
        if series.len() < 2 || categories.is_empty() {
            return;
        }
        let Some(outline) = self.axis_outline(lines.outline.as_ref()) else {
            return;
        };
        let xs = self.category_positions(rect, categories.len());
        for index in 0..categories.len() {
            let values: Vec<f64> = series.iter().filter_map(|s| at(&s.values, index)).collect();
            if values.len() < 2 {
                continue;
            }
            let (low, high) = super::series::min_max(&values);
            let top = self.value_to_y(rect, high, scale);
            let bottom = self.value_to_y(rect, low, scale);
            if (top - bottom).abs() < 1e-6 {
                continue;
            }
            let x = xs.get(index).copied().unwrap_or(0.0);
            self.line(x, top, x, bottom, Some(&outline));
        }
    }

    /// A stock chart's up-down bars, open to close.
    pub fn draw_up_down_bars(&mut self, rect: Rect, series: &[Series], categories: &[String], scale: Scale) {
        let Some(bars) = &self.plot.up_down_bars else { return };
        if series.len() < 2 || categories.is_empty() {
            return;
        }
        let gap = bars.gap_width.unwrap_or(DEFAULT_UP_DOWN_GAP_WIDTH);
        #[allow(clippy::cast_precision_loss)]
        let band = rect.width() / categories.len().max(1) as f64;
        let width = band / py_max(1.0 + py_max(gap, 0.0) / 100.0, MIN_BAR_SLOTS);
        let xs = self.category_positions(rect, categories.len());
        let default_outline = Outline::solid(DEFAULT_AXIS_LINE_EMU, Colour::opaque(DEFAULT_AXIS_COLOR));
        let up_fill = self
            .resolve
            .fill(bars.up_fill.as_ref())
            .unwrap_or_else(|| Fill::solid(DEFAULT_UP_BAR_FILL));
        let down_fill = self
            .resolve
            .fill(bars.down_fill.as_ref())
            .unwrap_or_else(|| Fill::solid(DEFAULT_DOWN_BAR_FILL));
        let up_outline = self
            .resolve
            .outline(bars.up_outline.as_ref())
            .unwrap_or_else(|| default_outline.clone());
        let down_outline = self
            .resolve
            .outline(bars.down_outline.as_ref())
            .unwrap_or(default_outline);
        let (Some(first), Some(last)) = (series.first(), series.last()) else {
            return;
        };
        for index in 0..categories.len() {
            let (Some(opening), Some(closing)) = (at(&first.values, index), at(&last.values, index)) else {
                continue;
            };
            if opening == closing {
                continue;
            }
            let rising = closing > opening;
            let top = self.value_to_y(rect, py_max(opening, closing), scale);
            let bottom = self.value_to_y(rect, py_min(opening, closing), scale);
            let x = xs.get(index).copied().unwrap_or(0.0);
            let (fill, outline) = if rising {
                (up_fill.clone(), up_outline.clone())
            } else {
                (down_fill.clone(), down_outline.clone())
            };
            self.rect(
                Rect::new(x - width / 2.0, top, x + width / 2.0, bottom),
                Some(fill),
                Some(outline),
                "rect",
            );
        }
    }

    fn cross_between(&self) -> String {
        let stated = self
            .value_axis()
            .and_then(|a| a.cross_between.clone())
            .filter(|c| !c.is_empty());
        stated.unwrap_or_else(|| {
            if self.is_area() || self.is_surface() {
                DEFAULT_AREA_CROSS_BETWEEN
            } else {
                "between"
            }
            .to_owned()
        })
    }

    /// Whether points stand on the category ticks rather than in the bands' middles.
    pub fn points_on_ticks(&self) -> bool {
        (self.is_area() || self.is_line() || self.is_surface()) && self.cross_between() == "midCat"
    }

    /// Where each category stands across the plot.
    pub fn category_positions(&self, rect: Rect, count: usize) -> Vec<f64> {
        if count == 0 {
            return Vec::new();
        }
        #[allow(clippy::cast_precision_loss)]
        if self.points_on_ticks() {
            let step = rect.width() / (count - 1).max(1) as f64;
            return (0..count)
                .map(|i| rect.left + self.category_slot(i, count) as f64 * step)
                .collect();
        }
        #[allow(clippy::cast_precision_loss)]
        let band = rect.width() / count as f64;
        #[allow(clippy::cast_precision_loss)]
        (0..count)
            .map(|i| rect.left + (self.category_slot(i, count) as f64 + 0.5) * band)
            .collect()
    }

    /// Whether the category axis runs reversed.
    pub fn categories_reversed(&self) -> bool {
        if self.is_three_d() {
            return false;
        }
        let axis = self
            .axis_for(0)
            .or_else(|| self.axis_of_kind("catAx"))
            .or_else(|| self.axis_of_kind("dateAx"));
        axis.is_some_and(|a| a.orientation == "maxMin")
    }

    /// Category `index`'s place along the axis.
    pub fn category_slot(&self, index: usize, count: usize) -> usize {
        if self.categories_reversed() {
            count.saturating_sub(1).saturating_sub(index)
        } else {
            index
        }
    }

    /// A marker at `centre`.
    pub fn marker(&mut self, centre: Point, item: &Series) {
        let half = item.marker_size / 2.0;
        let rect = Rect::new(centre.0 - half, centre.1 - half, centre.0 + half, centre.1 + half);
        let preset = marker_preset(item.marker_symbol.as_deref().unwrap_or("")).unwrap_or("ellipse");
        self.rect(rect, item.marker_fill.clone(), item.marker_outline.clone(), preset);
    }

    fn bar(&mut self, rect: Rect, item: &Series, bar: &Bar, scale: Scale, horizontal: bool) {
        let key = i64::try_from(bar.point).unwrap_or(i64::MAX);
        let mut fill = item
            .point_fills
            .get(&key)
            .or_else(|| item.vary_fills.get(bar.point))
            .cloned()
            .or_else(|| item.fill.clone());
        let mut outline = item.outline_at(bar.point);
        if bar.end < bar.start && item.invert_if_negative && !item.has_point_fill(bar.point) {
            fill = Some(Fill::solid("#FFFFFF"));
            outline = outline.or_else(|| {
                Some(Outline::solid(
                    INVERTED_BAR_OUTLINE_EMU,
                    Colour::opaque(DEFAULT_AXIS_COLOR),
                ))
            });
        }
        let shape = if horizontal {
            let (x0, x1) = (
                self.value_to_x(rect, bar.start, scale),
                self.value_to_x(rect, bar.end, scale),
            );
            Rect::new(py_min(x0, x1), bar.offset, py_max(x0, x1), bar.offset + bar.size)
        } else {
            let (y0, y1) = (
                self.value_to_y(rect, bar.start, scale),
                self.value_to_y(rect, bar.end, scale),
            );
            Rect::new(bar.offset, py_min(y0, y1), bar.offset + bar.size, py_max(y0, y1))
        };
        if shape.width() <= 0.0 || shape.height() <= 0.0 {
            return;
        }
        if self.group.prisms.is_some() {
            let order = self.prism_order(shape);
            if let Some(prisms) = self.group.prisms.as_mut() {
                prisms.push(Prism {
                    order,
                    rect: shape,
                    fill,
                    outline,
                    base: item.colour.clone(),
                });
            }
            return;
        }
        self.rect(shape, fill, outline, "rect");
    }

    /// Where bars start: the category axis' crossing value.
    pub fn bar_base(&self, scale: Scale) -> f64 {
        if self.draws_a_scene() {
            return 0.0;
        }
        let axis = self
            .axis_for(0)
            .or_else(|| self.axis_of_kind("catAx"))
            .or_else(|| self.axis_of_kind("dateAx"));
        match axis.and_then(|a| a.crosses.as_deref()) {
            Some("max") => scale.maximum,
            Some("min") => scale.minimum,
            _ => 0.0,
        }
    }

    /// Where the category axis crosses the value axis.
    pub fn category_axis_position(&self, rect: Rect, scale: Scale, axis: Option<&Axis>, labels: bool) -> f64 {
        let horizontal = self.horizontal();
        let (mut low, mut high) = if horizontal {
            (rect.left, rect.right)
        } else {
            (rect.bottom, rect.top)
        };
        if scale.reversed {
            std::mem::swap(&mut low, &mut high);
        }
        if labels && axis.is_some_and(|a| a.tick_label_position.as_deref() == Some("low")) {
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
        let along = if horizontal {
            self.value_to_x(rect, value, scale)
        } else {
            self.value_to_y(rect, value, scale)
        };
        py_min(py_max(along, py_min(low, high)), py_max(low, high))
    }

    /// The value and category axis lines.
    pub fn draw_axis_lines(
        &mut self,
        rect: Rect,
        scale: Scale,
        value_axis: Option<&Axis>,
        category_axis: Option<&Axis>,
    ) {
        let horizontal = self.horizontal();
        let crossing = self.category_axis_position(rect, scale, category_axis, false);
        if let Some(axis) = value_axis.filter(|a| !a.delete) {
            let outline = self.axis_outline(axis.outline.as_ref());
            if horizontal {
                self.line(rect.left, rect.bottom, rect.right, rect.bottom, outline.as_ref());
            } else {
                self.line(rect.left, rect.top, rect.left, rect.bottom, outline.as_ref());
            }
        }
        if let Some(axis) = category_axis.filter(|a| !a.delete) {
            let outline = self.axis_outline(axis.outline.as_ref());
            if horizontal {
                self.line(crossing, rect.top, crossing, rect.bottom, outline.as_ref());
            } else {
                self.line(rect.left, crossing, rect.right, crossing, outline.as_ref());
            }
        }
    }

    /// The tick marks of both axes (none on a 3-D chart).
    pub fn draw_axis_ticks(
        &mut self,
        rect: Rect,
        scale: Scale,
        axes: (Option<&Axis>, Option<&Axis>),
        count: usize,
        fonts: (&ChartFont, &ChartFont),
    ) {
        let (value_axis, category_axis) = axes;
        let (value_font, category_font) = fonts;
        if self.is_three_d() {
            return;
        }
        let horizontal = self.horizontal();
        let crossing = self.category_axis_position(rect, scale, category_axis, false);
        if let Some(axis) = value_axis {
            let (majors, minors) = self.value_tick_positions(rect, scale, Some(axis), horizontal);
            let (at, outward) = if horizontal {
                (rect.bottom, 1.0)
            } else {
                (rect.left, -1.0)
            };
            let run = TickRun {
                vertical: !horizontal,
                at,
                outward,
                majors: &majors,
                minors: &minors,
            };
            self.draw_ticks(Some(axis), value_font, &run);
        }
        if let Some(axis) = category_axis {
            let far = axis.crosses.as_deref() == Some("max");
            let (majors, minors, outward) = if horizontal {
                let (m, n) = self.category_tick_positions(rect.bottom, rect.height(), -1.0, count, Some(axis));
                (m, n, if far { 1.0 } else { -1.0 })
            } else {
                let (m, n) = self.category_tick_positions(rect.left, rect.width(), 1.0, count, Some(axis));
                (m, n, if far { -1.0 } else { 1.0 })
            };
            let run = TickRun {
                vertical: horizontal,
                at: crossing,
                outward,
                majors: &majors,
                minors: &minors,
            };
            self.draw_ticks(Some(axis), category_font, &run);
        }
    }

    /// The major and minor tick positions of a value axis.
    pub fn value_tick_positions(
        &self,
        rect: Rect,
        scale: Scale,
        axis: Option<&Axis>,
        horizontal: bool,
    ) -> (Vec<f64>, Vec<f64>) {
        let place = |v: f64| {
            if horizontal {
                self.value_to_x(rect, v, scale)
            } else {
                self.value_to_y(rect, v, scale)
            }
        };
        (
            tick_values(scale).into_iter().map(place).collect(),
            minor_tick_values(scale, axis).into_iter().map(place).collect(),
        )
    }

    fn category_tick_positions(
        &self,
        origin: f64,
        length: f64,
        sign: f64,
        count: usize,
        axis: Option<&Axis>,
    ) -> (Vec<f64>, Vec<f64>) {
        if count == 0 || length <= 0.0 {
            return (Vec::new(), Vec::new());
        }
        #[allow(clippy::cast_precision_loss)]
        let (step, ticks) = if self.points_on_ticks() {
            (length / (count - 1).max(1) as f64, count)
        } else {
            (length / count as f64, count + 1)
        };
        let skip = axis
            .and_then(|a| a.tick_mark_skip)
            .filter(|s| *s != 0)
            .unwrap_or(1)
            .max(1);
        let skip = usize::try_from(skip).unwrap_or(1);
        #[allow(clippy::cast_precision_loss)]
        let skip_f = skip as f64;
        let kept: Vec<usize> = (0..ticks).step_by(skip).collect();
        #[allow(clippy::cast_precision_loss)]
        let majors = kept.iter().map(|&i| origin + sign * i as f64 * step).collect();
        #[allow(clippy::cast_precision_loss)]
        let minors = kept
            .iter()
            .map(|&i| origin + sign * py_min((i as f64 + skip_f / 2.0) * step, length))
            .collect();
        (majors, minors)
    }

    /// Tick marks along an axis line.
    pub fn draw_ticks(&mut self, axis: Option<&Axis>, font: &ChartFont, run: &TickRun<'_>) {
        let TickRun {
            vertical,
            at,
            outward,
            majors,
            minors,
        } = *run;
        let Some(axis) = axis.filter(|a| !a.delete) else { return };
        let Some(outline) = self.axis_outline(axis.outline.as_ref()) else {
            return;
        };
        let (major, minor) = tick_marks(Some(axis), self.style.office_2007);
        let ascent = font.box_.ascent;
        for (kind, length, positions) in [
            (minor, ascent * TICK_MINOR_ASCENT, minors),
            (major, ascent * TICK_MAJOR_ASCENT, majors),
        ] {
            let Some((low, high)) = tick_span(&kind, length, outward) else {
                continue;
            };
            for &position in positions {
                if vertical {
                    self.line(at + low, position, at + high, position, Some(&outline));
                } else {
                    self.line(position, at + low, position, at + high, Some(&outline));
                }
            }
        }
    }
}
