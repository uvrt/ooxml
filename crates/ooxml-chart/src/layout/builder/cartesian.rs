//! Owns: cartesian charts -- bars, lines, areas, stock and surface charts and their
//! combinations, with a secondary value axis -- and scatter and bubble charts (two value
//! axes, their crossing, bubbles and point labels).
//!
//! Must not: lay out a polar chart.

use super::frame::{texts, PlotRectInput};
use super::marks::TickRun;
use super::{same_axis, Builder, Group, Series};
use crate::layout::consts::EDGE_INSET_PT;
use crate::layout::data::ChartData;
use crate::layout::scale::Scale;
use crate::layout::sources::labels_shown;
use crate::layout::text::ChartFont;
use crate::layout::three_d::Rect;
use crate::model::Axis;

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
}
