//! Owns: what the builder reads off the plot -- its series with their resolved fills,
//! outlines, lines, markers and labels; accent colours and their cycle; category labels;
//! which axes and plots apply; the value axis' reach, scale, ticks and their texts; and the
//! fonts text is drawn in.
//!
//! Must not: place or draw anything.

use super::{at, same_axis, Builder, Labels, Series};
use crate::layout::consts::{
    ACCENT_KEYS, AXIS_MAX_INTERVALS, COMBO_CHART_KINDS, DEFAULT_LINE_SERIES_WIDTH_EMU, DEFAULT_MARKER_CYCLE,
    DEFAULT_MARKER_OUTLINE_EMU, MAX_MAJOR_TICKS, MAX_MINOR_TICKS, MINOR_TICKS_PER_MAJOR, RADAR_MARKER_SIZE_PT,
};
use crate::layout::numbers::{format_number, shows_date};
use crate::layout::paths::cycle_shift;
use crate::layout::scale::{apply_axis_limits, nice_axis_scale, py_max, py_min, NiceOptions, Scale};
use crate::layout::sources::{default_run, text_size, text_typeface, text_typeface_ea};
use crate::layout::text::{font_box, ChartFont};
use crate::model::{flat_chart_kind, Axis, DataLabels, Plot};
use crate::pymath;
use ooxml_drawingml::drawable::{Colour, Fill, Outline};
use ooxml_drawingml::source;
use ooxml_drawingml::text::TextBody;
use ooxml_fonts::advances::covers_east_asian;

/// The face East Asian text is drawn in, from a cascade of names (`east_asian_family`).
fn east_asian_family(candidates: &[Option<&str>]) -> Option<String> {
    let named: Vec<&str> = candidates
        .iter()
        .flatten()
        .copied()
        .filter(|n| !crate::layout::text::py_strip(n).is_empty() && !n.starts_with('+'))
        .collect();
    named
        .iter()
        .find(|n| covers_east_asian(n))
        .or(named.first())
        .map(|n| (*n).to_owned())
}

/// Whether an outline is stated as `a:noFill`.
pub(crate) fn no_fill_outline(outline: Option<&source::Outline>) -> bool {
    outline.is_some_and(|o| matches!(o.fill, Some(source::Fill::None)))
}

impl<'a> Builder<'a> {
    /// Whether points of a single series take the accents in turn.
    pub fn vary_colors(&self) -> bool {
        if self.is_polar() {
            return self.plot.vary_colors != Some(false);
        }
        if self.plot.vary_colors != Some(true) || self.plot.series.len() != 1 {
            return false;
        }
        !self.stacked("clustered")
    }

    /// The plot's series as drawn.
    pub fn series(&self) -> Vec<Series> {
        let mut out = Vec::new();
        for source in &self.plot.series {
            let mut fill = self.resolve.fill(source.fill.as_ref());
            let colour = self.series_colour(fill.as_ref(), source.index);
            if fill.is_none() {
                fill = Some(Fill::Solid(colour.clone()));
            }
            let outline = self.resolve.outline(source.outline.as_ref());
            let mut item = Series::new(source.name.as_ref().and_then(|n| n.plain()), colour, fill, outline);
            item.values = source.values.clone();
            item.format_code = source.format_code.clone();
            item.invert_if_negative = source.invert_if_negative == Some(true);
            if self.vary_colors() && source.fill.is_none() && !self.style.accents.is_empty() {
                let count = self.point_colour_count(item.values.len());
                item.vary_fills = (0..item.values.len())
                    .map(|i| Fill::Solid(self.cycle_accent(i, count)))
                    .collect();
            }
            let keyed = self.is_line()
                || (self.is_scatter() && !self.is_bubble())
                || (self.is_radar() && self.radar_style() != "filled");
            if keyed {
                self.read_line_style(&mut item, source, source.index);
                item.line_keyed = self.plot.kind != "line3DChart";
                if self.is_radar() && source.marker.as_ref().is_none_or(|m| m.size.is_none_or(|s| s == 0.0)) {
                    item.marker_size = RADAR_MARKER_SIZE_PT;
                }
            }
            item.labels = Some(self.read_labels(&[source.data_labels.as_ref(), self.plot.data_labels.as_ref()]));
            item.label_range = source.label_range.clone().unwrap_or_default();
            if let Some(own) = &source.data_labels {
                for (index, labels) in &own.overrides {
                    let read = self.read_labels(&[Some(labels), Some(own), self.plot.data_labels.as_ref()]);
                    item.point_labels.insert(*index, read);
                }
            }
            for point in &source.data_points {
                if let Some(f) = self.resolve.fill(point.fill.as_ref()) {
                    item.point_fills.insert(point.index, f);
                }
                if let Some(o) = self.resolve.outline(point.outline.as_ref()) {
                    item.point_outlines.insert(point.index, o);
                }
            }
            out.push(item);
        }
        out
    }

    /// Data labels read through their cascade, the first stating each flag winning.
    pub fn read_labels(&self, sources: &[Option<&DataLabels>]) -> Labels {
        let present: Vec<&DataLabels> = sources.iter().flatten().copied().collect();
        if present.iter().any(|s| s.delete == Some(true)) {
            return Labels::default();
        }
        let flag = |get: fn(&DataLabels) -> Option<bool>| present.iter().find_map(|s| get(s)).unwrap_or(false);
        let number_format = present.iter().find(|s| s.number_format.is_some()).and_then(|s| {
            if s.number_format_source_linked == Some(true) {
                None
            } else {
                s.number_format.clone()
            }
        });
        let mut labels = Labels {
            show_value: flag(|s| s.show_value),
            show_category: flag(|s| s.show_category_name),
            show_series: flag(|s| s.show_series_name),
            show_percent: flag(|s| s.show_percent),
            show_bubble_size: flag(|s| s.show_bubble_size),
            position: present
                .iter()
                .find_map(|s| s.position.clone().filter(|p| !p.is_empty())),
            number_format,
            font: None,
            colour: None,
            text: present.first().and_then(|s| s.text.clone()),
            show_range: flag(|s| s.show_range),
        };
        if labels.anything() {
            let bodies: Vec<Option<&TextBody>> = present.iter().map(|s| s.text_properties.as_ref()).collect();
            let font = self.font(&bodies);
            labels.colour = Some(font.colour.clone().unwrap_or_else(|| self.style.colour.clone()));
            labels.font = Some(font);
        }
        labels
    }

    /// The colour the first body (then the chart's) states for its default run.
    pub fn text_colour(&self, sources: &[Option<&TextBody>]) -> Option<Colour> {
        let chart = self.chart.text_properties.as_ref();
        for source in sources.iter().copied().chain(std::iter::once(chart)) {
            if let Some(colour) = default_run(source).and_then(|r| r.colour.clone()) {
                if let Some(Fill::Solid(c)) = self.resolve.fill(Some(&source::Fill::Solid(colour))) {
                    return Some(c);
                }
            }
        }
        None
    }

    fn read_line_style(&self, item: &mut Series, source: &crate::model::Series, index: i64) {
        let mut outline = self.resolve.outline(source.outline.as_ref());
        if let Some(Fill::Solid(c)) = outline.as_ref().and_then(|o| o.fill.as_ref()) {
            item.colour = c.clone();
        } else if let Some(accent) = self.accent(index) {
            item.colour = accent;
        }
        if no_fill_outline(source.outline.as_ref()) {
            item.line = None;
            self.read_marker_style(item, source, index);
            return;
        }
        let mut line = match outline.take() {
            Some(o) if o.fill.is_some() => o,
            other => {
                let mut width = DEFAULT_LINE_SERIES_WIDTH_EMU;
                if self.rules.stated_line_width {
                    if let Some(o) = &other {
                        width = o.width;
                    }
                }
                Outline::solid(width, item.colour.clone())
            }
        };
        if line.line_cap.is_none() {
            line.line_cap = Some("round".into());
        }
        item.line = Some(line);
        self.read_marker_style(item, source, index);
        item.smooth = source.smooth != Some(false);
    }

    fn read_marker_style(&self, item: &mut Series, source: &crate::model::Series, index: i64) {
        let cycle = || {
            let i = usize::try_from(index.rem_euclid(6)).unwrap_or(0);
            DEFAULT_MARKER_CYCLE.get(i).map(|s| (*s).to_owned())
        };
        let marker = source.marker.as_ref();
        match marker.map(|m| m.symbol.as_deref()) {
            None | Some(None | Some("auto")) => item.marker_symbol = cycle(),
            Some(Some("none")) => {}
            Some(Some(symbol)) => item.marker_symbol = Some(symbol.to_owned()),
        }
        if let Some(size) = marker.and_then(|m| m.size).filter(|s| *s != 0.0) {
            item.marker_size = size;
        }
        item.marker_fill = marker
            .and_then(|m| self.resolve.fill(m.fill.as_ref()))
            .or_else(|| Some(Fill::Solid(item.colour.clone())));
        item.marker_outline = marker
            .and_then(|m| self.resolve.outline(m.outline.as_ref()))
            .or_else(|| Some(Outline::solid(DEFAULT_MARKER_OUTLINE_EMU, item.colour.clone())));
    }

    fn point_colour_count(&self, points: usize) -> usize {
        if self.is_of_pie() {
            points + 1
        } else {
            points
        }
    }

    /// The accent for `index` (Python's `accents[index % len(accents)]`).
    fn accent(&self, index: i64) -> Option<Colour> {
        let count = i64::try_from(self.style.accents.len()).ok().filter(|c| *c > 0)?;
        self.style
            .accents
            .get(usize::try_from(index.rem_euclid(count)).ok()?)
            .cloned()
    }

    /// The accent for point `index` of `count`, shifted on each repetition past six.
    pub fn cycle_accent(&self, index: usize, count: usize) -> Colour {
        let accents = &self.style.accents;
        let base = accents
            .get(index % accents.len().max(1))
            .cloned()
            .unwrap_or_else(|| Colour::opaque("#4472C4"));
        if count <= ACCENT_KEYS.len() {
            return base;
        }
        Colour {
            hex: cycle_shift(&base.hex, index / ACCENT_KEYS.len()),
            alpha: base.alpha,
        }
    }

    fn series_colour(&self, fill: Option<&Fill>, index: i64) -> Colour {
        if let Some(Fill::Solid(c)) = fill {
            return c.clone();
        }
        self.accent(index).unwrap_or_else(|| Colour::opaque("#4472C4"))
    }

    /// The category labels: the first plot's series that states any, padded with numbers.
    pub fn categories(&self, series: &[Series], labels: bool) -> Vec<String> {
        let longest = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
        let mut names: Vec<String> = Vec::new();
        'plots: for plot in &self.plots {
            for source in &plot.series {
                if source.categories.iter().any(|c| !c.is_empty()) {
                    names = if labels {
                        self.category_texts(source)
                    } else {
                        source.categories.clone()
                    };
                    break;
                }
            }
            if !names.is_empty() {
                break 'plots;
            }
        }
        let have = names.len();
        names.extend((have..longest).map(|i| (i + 1).to_string()));
        names
    }

    /// A series' category labels with numbers and dates written through their codes.
    pub fn category_texts(&self, source: &crate::model::Series) -> Vec<String> {
        let mut names = source.categories.clone();
        let Some(values) = source.category_values.as_ref().filter(|v| !v.is_empty()) else {
            return names;
        };
        let own = source.category_point_format_codes.clone().unwrap_or_default();
        for (index, value) in values.iter().enumerate() {
            let Some(value) = *value else { continue };
            if index >= names.len() {
                continue;
            }
            let code = own
                .get(index)
                .cloned()
                .flatten()
                .filter(|c| !c.is_empty())
                .or_else(|| source.category_format_code.clone());
            let text = if shows_date(code.as_deref(), value) {
                match ooxml_numfmt::format_serial(value, code.as_deref(), self.locale, self.chart.date1904) {
                    Some(text) => text,
                    None => continue,
                }
            } else {
                format_number(value, code.as_deref(), self.locale)
            };
            if let Some(name) = names.get_mut(index) {
                *name = text;
            }
        }
        names
    }

    /// The axis the plot's `position`th `c:axId` names.
    pub fn axis_for(&self, position: usize) -> Option<&'a Axis> {
        axis_for_plot(self.chart, self.plot, position)
    }

    /// The first axis of `kind`.
    pub fn axis_of_kind(&self, kind: &str) -> Option<&'a Axis> {
        self.chart.axes.iter().find(|a| a.kind == kind)
    }

    /// The value axis (the plot's second, else the first `valAx`).
    pub fn value_axis(&self) -> Option<&'a Axis> {
        self.axis_for(1).or_else(|| self.axis_of_kind("valAx"))
    }

    /// The category axis (the plot's first, else the first `catAx`).
    pub fn category_axis(&self) -> Option<&'a Axis> {
        self.axis_for(0).or_else(|| self.axis_of_kind("catAx"))
    }

    /// `plot`'s value axis.
    pub fn plot_value_axis(&self, plot: &Plot) -> Option<&'a Axis> {
        axis_for_plot(self.chart, plot, 1).or_else(|| self.axis_of_kind("valAx"))
    }

    /// A combination chart's value axes (left, right), or `None` where they do not pair.
    pub fn combo_axes(&self) -> Option<(Option<&'a Axis>, Option<&'a Axis>)> {
        let mut axes: Vec<Option<&'a Axis>> = Vec::new();
        for plot in &self.plots {
            let axis = self.plot_value_axis(plot);
            if !axes.iter().any(|seen| same_axis(*seen, axis)) {
                axes.push(axis);
            }
        }
        if let [only] = axes.as_slice() {
            return Some((*only, None));
        }
        let right: Vec<Option<&'a Axis>> = axes
            .iter()
            .copied()
            .filter(|a| a.is_some_and(|a| a.crosses.as_deref() == Some("max")))
            .collect();
        let left: Vec<Option<&'a Axis>> = axes
            .iter()
            .copied()
            .filter(|a| !right.iter().any(|r| same_axis(*a, *r)))
            .collect();
        match (left.as_slice(), right.as_slice()) {
            ([l], [r]) => Some((*l, *r)),
            _ => None,
        }
    }

    /// The plots drawn: a combination's in area, bar, line order, else this one.
    pub fn drawn_plots(&self) -> Vec<&'a Plot> {
        if self.plots.len() < 2 {
            return vec![self.plot];
        }
        let kinds: Vec<&str> = self.plots.iter().map(|p| flat_chart_kind(&p.kind)).collect();
        if kinds.iter().any(|k| !COMBO_CHART_KINDS.contains(k)) {
            return vec![self.plot];
        }
        if self
            .plots
            .iter()
            .zip(&kinds)
            .any(|(p, k)| *k == "barChart" && p.bar_direction.as_deref().unwrap_or("col") != "col")
        {
            return vec![self.plot];
        }
        if self.combo_axes().is_none() {
            return vec![self.plot];
        }
        let rank = |p: &&Plot| {
            COMBO_CHART_KINDS
                .iter()
                .position(|k| *k == flat_chart_kind(&p.kind))
                .unwrap_or(0)
        };
        let mut sorted = self.plots.clone();
        sorted.sort_by_key(rank);
        sorted
    }

    /// Whether any drawn plot keys its legend with a line.
    pub fn any_line_keyed(&mut self) -> bool {
        self.drawn_plots()
            .into_iter()
            .any(|plot| self.for_plot(plot, |b| b.is_line_keyed()))
    }

    /// Whether this plot keys its legend with a line.
    pub fn is_line_keyed(&self) -> bool {
        ((self.is_line() && self.plot.kind != "line3DChart")
            || (self.is_scatter() && !self.is_bubble())
            || (self.is_radar() && self.radar_style() != "filled"))
            && self.draws_a_rule()
    }

    fn draws_a_rule(&self) -> bool {
        self.plot.series.iter().any(|s| !no_fill_outline(s.outline.as_ref()))
    }

    /// The numbers a value axis must reach: each value, or stacked columns' sums.
    pub fn axis_reach(&self, series: &[Series]) -> Vec<f64> {
        if self.stacked("clustered") {
            let length = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
            let mut numbers = Vec::new();
            for index in 0..length {
                let column: Vec<f64> = series.iter().filter_map(|s| at(&s.values, index)).collect();
                numbers.push(pymath::sum(column.iter().copied().filter(|v| *v != 0.0 && *v > 0.0)));
                numbers.push(pymath::sum(column.iter().copied().filter(|v| *v != 0.0 && *v < 0.0)));
            }
            return numbers;
        }
        series.iter().flat_map(|s| s.values.iter().flatten().copied()).collect()
    }

    /// The value axis' scale for `series` (or `numbers`).
    pub fn scale(
        &mut self,
        series: &[Series],
        axis: Option<&'a Axis>,
        radial_pt: Option<f64>,
        numbers: Option<Vec<f64>>,
    ) -> Scale {
        if self.grouping_or("") == "percentStacked" {
            return Scale::new(0.0, 1.0, 0.1);
        }
        let mut numbers = numbers.unwrap_or_else(|| self.axis_reach(series));
        if numbers.is_empty() {
            numbers = vec![0.0];
        }
        let horizontal = self.horizontal();
        let (low, high) = min_max(&numbers);
        let strict = !(self.is_radar() || self.is_three_d());
        let scaled = |intervals: i64| {
            nice_axis_scale(
                low,
                high,
                NiceOptions {
                    intervals,
                    strict,
                    ..NiceOptions::default()
                },
            )
        };
        let provisional = scaled(AXIS_MAX_INTERVALS);
        let intervals = self.value_axis_intervals(axis, horizontal, Some(provisional), radial_pt);
        let scale = if intervals >= AXIS_MAX_INTERVALS {
            provisional
        } else {
            scaled(intervals)
        };
        apply_axis_limits(scale, axis)
    }

    /// Each major tick's value and text.
    pub fn tick_texts(&self, scale: Scale, axis: Option<&Axis>) -> Vec<(f64, String)> {
        let mut code = self.axis_format(axis);
        if self.grouping_or("") == "percentStacked" {
            code = Some("0%".into());
        }
        let values = tick_values(scale);
        if values.is_empty() {
            return vec![(
                scale.minimum,
                format_number(scale.minimum, code.as_deref(), self.locale),
            )];
        }
        values
            .into_iter()
            .map(|v| (v, format_number(v, code.as_deref(), self.locale)))
            .collect()
    }

    fn axis_format(&self, axis: Option<&Axis>) -> Option<String> {
        let axis = axis?;
        let own = axis.number_format.clone().filter(|f| f != "General");
        if own.as_deref().is_some_and(|o| !o.is_empty()) && axis.number_format_source_linked != Some(true) {
            return own;
        }
        if axis.number_format_source_linked == Some(false) {
            return None;
        }
        for source in &self.plot.series {
            if let Some(code) = source.format_code.as_ref().filter(|c| !c.is_empty() && *c != "General") {
                return Some(code.clone());
            }
        }
        own
    }

    /// The face, size and colour text is drawn in, from its bodies and then the chart's.
    pub fn font(&self, sources: &[Option<&TextBody>]) -> ChartFont {
        let (mut size, mut typeface, mut typeface_ea) = (None, None, None);
        let chart = self.chart.text_properties.as_ref();
        for source in sources.iter().copied().chain(std::iter::once(chart)) {
            size = size.or_else(|| text_size(source));
            typeface = typeface.or_else(|| text_typeface(source));
            typeface_ea = typeface_ea.or_else(|| text_typeface_ea(source));
        }
        let family = typeface
            .filter(|t| !t.is_empty())
            .and_then(|t| self.resolve.typeface(t))
            .filter(|f| !f.is_empty())
            .or_else(|| self.style.font_family.clone());
        let resolved_ea = typeface_ea
            .filter(|t| !t.is_empty())
            .and_then(|t| self.resolve.typeface(t));
        let family_ea = east_asian_family(&[resolved_ea.as_deref(), self.style.font_family_ea.as_deref()]);
        let size = size.unwrap_or(self.style.font_size);
        ChartFont {
            box_: font_box(family.as_deref(), size),
            box_ea: family_ea.as_deref().map(|f| font_box(Some(f), size)),
            family,
            family_ea,
            colour: self.text_colour(sources),
        }
    }

    /// An axis' label font.
    pub fn label_font(&self, axis: Option<&Axis>) -> ChartFont {
        self.font(&[axis.and_then(|a| a.text_properties.as_ref())])
    }

    /// The legend's font.
    pub fn legend_font(&self) -> ChartFont {
        self.font(&[self.chart.legend.as_ref().and_then(|l| l.text_properties.as_ref())])
    }
}

/// The axis `plot`'s `position`th `c:axId` names.
pub(crate) fn axis_for_plot<'a>(chart: &'a crate::model::Chart, plot: &Plot, position: usize) -> Option<&'a Axis> {
    let wanted = plot.axis_ids.get(position)?;
    chart
        .axes
        .iter()
        .find(|a| a.axis_id.as_deref() == Some(wanted.as_str()))
}

/// Python's `min(values), max(values)` of a non-empty list.
pub(crate) fn min_max(values: &[f64]) -> (f64, f64) {
    let first = values.first().copied().unwrap_or(0.0);
    values
        .iter()
        .skip(1)
        .fold((first, first), |(lo, hi), v| (py_min(lo, *v), py_max(hi, *v)))
}

/// The major ticks' values.
pub(crate) fn tick_values(scale: Scale) -> Vec<f64> {
    let span = scale.maximum - scale.minimum;
    if scale.unit <= 0.0 || !span.is_finite() || !scale.unit.is_finite() {
        return Vec::new();
    }
    let steps = span / scale.unit;
    #[allow(clippy::cast_precision_loss)]
    if !steps.is_finite() || steps <= 0.0 || steps > MAX_MAJOR_TICKS as f64 {
        return Vec::new();
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = crate::pyfmt::round_half_even(steps) as usize;
    #[allow(clippy::cast_precision_loss)]
    (0..=count).map(|i| scale.minimum + i as f64 * scale.unit).collect()
}

/// The minor ticks' values.
pub(crate) fn minor_tick_values(scale: Scale, axis: Option<&Axis>) -> Vec<f64> {
    let stated = axis.and_then(|a| a.minor_unit).filter(|u| *u > 0.0);
    let minor = stated.unwrap_or(scale.unit / MINOR_TICKS_PER_MAJOR);
    let span = scale.maximum - scale.minimum;
    if minor <= 0.0 || !span.is_finite() || !minor.is_finite() {
        return Vec::new();
    }
    let steps = span / minor;
    #[allow(clippy::cast_precision_loss)]
    if !steps.is_finite() || steps <= 0.0 || steps > MAX_MINOR_TICKS as f64 {
        return Vec::new();
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = (steps + 1e-9).floor() as usize;
    #[allow(clippy::cast_precision_loss)]
    (0..=count).map(|i| scale.minimum + i as f64 * minor).collect()
}
