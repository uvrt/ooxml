//! Owns: how the frame is divided -- the plot rectangle and the bands round it (value and
//! category labels, the title, the legend, axis titles), a 3-D chart's scene inside it, the
//! value axis' room, a polar chart's region -- and the title and background drawn there.
//!
//! Must not: draw marks, axes or the legend.

use super::{Builder, Title};
use crate::layout::consts::{
    sin_45, CATEGORY_LABEL_GAP_ASCENT, CATEGORY_LABEL_GAP_EM, EDGE_INSET_PT, EMU_PER_POINT, FRAME_PADDING_PT,
    LABEL_ELLIPSIS, LEGEND_BAND_PAD_PT, RICH_TITLE_FALLBACK_FACE, ROTATED_LABEL_HEADROOM_PT, ROTATED_LABEL_RATIO,
    TITLE_BAND_LINES, TITLE_BASELINE_ASCENTS, TOP_INSET_BASE_PT, VALUE_LABEL_GAP_EM, WORD_AXIS_TITLE_INSET_PT,
    WORD_SHORT_PLOT_EM, WORD_SHORT_PLOT_FLOOR_PT, WORD_TITLE_BASELINE_EM, WORD_TITLE_BASELINE_PT, WORD_TITLE_PAD_PT,
};
use crate::layout::scale::{bottom_axis_intervals, py_max, radial_axis_intervals, side_axis_intervals, Scale};
use crate::layout::sources::{
    labels_shown, powerpoint_auto_title, states_colour, title_size, title_states, title_states_defaults,
    title_states_no_font, TitleProperty,
};
use crate::layout::text::{font_box, py_split, rotated_label_anchor, truncate_label, wrap_label, ChartFont, FontBox};
use crate::layout::three_d::{camera, scene, Rect, Scene, SceneSpec};
use crate::model::{Axis, View3D};
use crate::rules::{SideLegendRule, TitleRule, TopRightLegendRule};
use ooxml_drawingml::drawable::{Element, Fill, Geometry, Outline, Shape, Transform};

/// The texts of `(value, text)` pairs.
pub(crate) fn texts(pairs: &[(f64, String)]) -> Vec<&str> {
    pairs.iter().map(|(_, t)| t.as_str()).collect()
}

/// The widest of `texts` in `font`, 0 for none.
pub(crate) fn widest<'t>(font: &ChartFont, texts: impl IntoIterator<Item = &'t str>) -> f64 {
    texts
        .into_iter()
        .map(|t| font.width(t))
        .fold(None, |m: Option<f64>, w| Some(m.map_or(w, |m| py_max(m, w))))
        .unwrap_or(0.0)
}

/// A plot rectangle's edges as they are worked out: left, top and right, the legend's band
/// at the bottom, and what the right edge was cut by.
struct Edges {
    left: f64,
    top: f64,
    right: f64,
    legend_bottom: f64,
    overhang_right: f64,
    second_band: f64,
}

/// What a plot rectangle is laid out with.
pub(crate) struct PlotRectInput<'t> {
    pub tick_texts: &'t [(f64, String)],
    pub categories: &'t [String],
    pub value_font: &'t ChartFont,
    pub category_font: &'t ChartFont,
    pub scale: Scale,
    pub second: Option<(&'t [(f64, String)], &'t ChartFont)>,
    pub flat: bool,
}

impl<'a> Builder<'a> {
    /// The camera this plot is drawn through, where one applies.
    pub fn three_d_view(&self) -> Option<&'a View3D> {
        camera(
            self.chart.view_3d.as_ref(),
            &self.plot.kind,
            self.plot.series.len().max(1),
            self.plot.grouping.as_deref(),
        )
    }

    /// The flat plot region a 3-D scene stands in.
    pub fn three_d_region(&mut self, scale: Scale) -> Rect {
        let value_axis = self.value_axis();
        let category_axis = self.category_axis();
        let ticks = self.tick_texts(scale, value_axis);
        let series = self.series();
        let categories = self.categories(&series, true);
        let value_font = self.label_font(value_axis);
        let category_font = self.label_font(category_axis);
        self.plot_rect(&PlotRectInput {
            tick_texts: &ticks,
            categories: &categories,
            value_font: &value_font,
            category_font: &category_font,
            scale,
            second: None,
            flat: true,
        })
    }

    fn three_d_reservation(&mut self, scale: Option<Scale>) -> f64 {
        let (Some(view), Some(scale)) = (self.three_d_view(), scale) else {
            return 0.0;
        };
        let region = self.three_d_region(scale);
        let face = self.three_d_scene(region, view).map_or(region, |s| s.face);
        py_max(0.0, region.height() - face.height())
    }

    /// The scene a region holds for this plot.
    pub fn three_d_scene(&self, region: Rect, view: &View3D) -> Option<Scene> {
        let count = self.categories(&self.series(), true).len().max(1);
        let across = if self.points_on_ticks() {
            (count - 1).max(1)
        } else {
            count
        };
        let spec = SceneSpec {
            kind: &self.plot.kind,
            series: self.plot.series.len().max(1),
            gap_depth: self.plot.gap_depth,
            grouping: self.plot.grouping.as_deref(),
            categories: count,
            gap_width: self.plot.gap_width,
            bar_direction: self.plot.bar_direction.as_deref(),
            across: Some(across),
        };
        scene(region, view, &spec)
    }

    fn axis_band_height(&mut self, scale: Option<Scale>) -> f64 {
        let mut height = self.frame.height();
        if let Some(title) = self.title_box() {
            height -= self.title_band(&title);
        }
        if matches!(self.legend_position(), Some("b" | "t" | "tr")) && !self.legend_overlays() {
            let font = self.legend_font();
            height -= self.legend_band_height(&font, false);
        }
        height -= self.axis_title_band("b");
        height - self.three_d_reservation(scale)
    }

    fn axis_band_width(&mut self) -> f64 {
        let mut width = self.frame.width();
        if matches!(self.legend_position(), Some("l" | "r")) && !self.legend_overlays() {
            let font = self.legend_font();
            width -= self.legend_side_width(&font, false);
        }
        width - self.axis_title_band("l")
    }

    /// How many intervals a value axis has room for.
    pub fn value_axis_intervals(
        &mut self,
        axis: Option<&'a Axis>,
        horizontal: bool,
        provisional: Option<Scale>,
        radial_pt: Option<f64>,
    ) -> i64 {
        let font = self.label_font(axis);
        if let Some(radial) = radial_pt {
            return radial_axis_intervals(radial, font.box_.pitch());
        }
        if !horizontal {
            return side_axis_intervals(self.axis_band_height(provisional), font.box_.pitch());
        }
        let wide = provisional.map_or(0.0, |p| widest(&font, texts(&self.tick_texts(p, axis))));
        bottom_axis_intervals(self.axis_band_width(), font.size(), wide)
    }

    /// The band value labels take beside the plot.
    pub fn value_label_band(&self, texts: &[&str], font: &ChartFont) -> f64 {
        FRAME_PADDING_PT + widest(font, texts.iter().copied()) + font.box_.descent + VALUE_LABEL_GAP_EM * font.size()
    }

    /// The plot rectangle: the frame less every band round it (and a 3-D scene's face).
    pub fn plot_rect(&mut self, input: &PlotRectInput<'_>) -> Rect {
        let mut edges = self.plot_sides(input);
        edges.top = self.frame.top
            + if self.horizontal() {
                EDGE_INSET_PT
            } else {
                self.top_inset(&input.value_font.box_)
            };
        if let Some(title) = self.title_box() {
            edges.top += self.title_band(&title);
        }
        self.plot_legend_bands(input, &mut edges);
        edges.left += self.axis_title_band("l");
        if edges.right - edges.left < 1.0 {
            edges.right = edges.left + 1.0;
        }
        let bottom = self.plot_bottom(input, &edges);
        let region = Rect::new(edges.left, edges.top, edges.right, bottom);
        let view = if input.flat { None } else { self.three_d_view() };
        let Some(view) = view else {
            return region;
        };
        match self.three_d_scene(region, view) {
            Some(found) => {
                self.group.scene = Some(found);
                found.face
            }
            None => region,
        }
    }

    /// The plot's left and right edges before the legend: the value (or category) labels'
    /// band, the overhang of labels standing on the end ticks, a secondary axis' band.
    fn plot_sides(&self, input: &PlotRectInput<'_>) -> Edges {
        let frame = self.frame;
        let (show_values, show_categories) = (labels_shown(self.value_axis()), labels_shown(self.category_axis()));
        let horizontal = self.horizontal();
        let tick_labels = texts(input.tick_texts);
        let categories: Vec<&str> = input.categories.iter().map(String::as_str).collect();
        let mut left = frame.left + EDGE_INSET_PT;
        if (horizontal && show_categories) || (!horizontal && show_values) {
            let (labels, font) = if horizontal {
                (&categories, input.category_font)
            } else {
                (&tick_labels, input.value_font)
            };
            left = frame.left + self.value_label_band(labels, font);
        }
        let mut overhang_right = 0.0;
        if show_categories && !categories.is_empty() && !horizontal && self.points_on_ticks() {
            let overhang_left = input.category_font.width(categories.first().copied().unwrap_or("")) / 2.0;
            overhang_right = input.category_font.width(categories.last().copied().unwrap_or("")) / 2.0;
            left = py_max(left, frame.left + EDGE_INSET_PT + overhang_left);
        }
        let second_band = match input.second {
            Some((second, font)) if !second.is_empty() => self.value_label_band(&texts(second), font),
            _ => EDGE_INSET_PT,
        };
        let mut right = frame.right - second_band - overhang_right;
        if horizontal && show_values {
            right -= widest(input.value_font, tick_labels.iter().copied()) / 2.0;
        }
        Edges {
            left,
            top: frame.top,
            right,
            legend_bottom: 0.0,
            overhang_right,
            second_band,
        }
    }

    /// The legend's band taken from the plot's edges.
    fn plot_legend_bands(&mut self, input: &PlotRectInput<'_>, edges: &mut Edges) {
        let legend = self.legend_position();
        if legend.is_none() || self.legend_overlays() {
            return;
        }
        let frame = self.frame;
        let legend_font = self.legend_font();
        let band = self.legend_band_height(&legend_font, false);
        let top_column = legend == Some("tr") && self.rules.top_right_legend == TopRightLegendRule::Column;
        if legend == Some("b") {
            edges.legend_bottom = band;
        } else if top_column {
            let named = self.legend_names(false).iter().filter(|n| !n.is_empty()).count();
            #[allow(clippy::cast_precision_loss)]
            let rows = named as f64;
            edges.top += rows * super::legend::legend_row_pitch(&legend_font.box_, 1);
            edges.legend_bottom = LEGEND_BAND_PAD_PT;
        } else if matches!(legend, Some("t" | "tr")) {
            edges.top += band;
        }
        if legend == Some("r") || top_column {
            edges.right = frame.right
                - self.legend_side_width(&legend_font, false)
                - (edges.second_band - EDGE_INSET_PT)
                - edges.overhang_right;
            let show_values = labels_shown(self.value_axis());
            if self.rules.side_legend == SideLegendRule::Word && self.horizontal() && show_values {
                edges.right -= widest(input.value_font, texts(input.tick_texts)) / 2.0;
            }
        } else if legend == Some("l") {
            edges.left += self.legend_side_width(&legend_font, false) - EDGE_INSET_PT;
        }
    }

    /// The plot's bottom: over the category (or value) labels' band, or the top inset, the
    /// legend's band and an axis title's; a short plot's shortfall shared (Word's).
    fn plot_bottom(&mut self, input: &PlotRectInput<'_>, edges: &Edges) -> f64 {
        let frame = self.frame;
        let horizontal = self.horizontal();
        let category_axis = self.category_axis();
        let show_bottom = if horizontal {
            labels_shown(self.value_axis())
        } else {
            labels_shown(category_axis)
        };
        let labels_under_plot = show_bottom
            && (horizontal
                || input.scale.minimum >= 0.0
                || category_axis.is_some_and(|a| a.tick_label_position.as_deref() == Some("low")));
        let mut bottom = if labels_under_plot {
            let banded: Vec<String> = if horizontal || self.points_on_ticks() {
                Vec::new()
            } else {
                input.categories.to_vec()
            };
            let font = if horizontal {
                input.value_font
            } else {
                input.category_font
            };
            frame.bottom - edges.legend_bottom - self.bottom_label_band(font, &banded, edges.right - edges.left)
        } else {
            frame.bottom - edges.legend_bottom - self.top_inset(&input.value_font.box_)
        };
        bottom -= self.axis_title_band("b");
        let top = edges.top;
        if self.rules.short_plot && !horizontal && !self.is_three_d() {
            let shortfall = WORD_SHORT_PLOT_EM * input.value_font.size() - (bottom - top);
            if shortfall > 0.0 {
                bottom = py_max(top + WORD_SHORT_PLOT_FLOOR_PT, bottom + shortfall / 2.0);
            }
        } else if bottom - top < 1.0 {
            bottom = top + 1.0;
        }
        bottom
    }

    /// Whether category labels turn to 45 degrees: a word wider than its band.
    pub fn labels_rotate(&self, font: &ChartFont, categories: &[String], plot_width: f64) -> bool {
        if categories.is_empty() {
            return false;
        }
        #[allow(clippy::cast_precision_loss)]
        let band = plot_width / categories.len() as f64;
        if band <= 0.0 {
            return false;
        }
        widest(font, categories.iter().flat_map(|t| py_split(t))) > band * ROTATED_LABEL_RATIO
    }

    fn label_line_count(&self, font: &ChartFont, categories: &[String], plot_width: f64) -> usize {
        if categories.is_empty() {
            return 1;
        }
        #[allow(clippy::cast_precision_loss)]
        let band = plot_width / categories.len() as f64;
        let most = categories
            .iter()
            .map(|t| wrap_label(t, font, band).len())
            .max()
            .unwrap_or(1);
        if most == 0 {
            1
        } else {
            most
        }
    }

    fn furniture_height(&mut self) -> f64 {
        let mut height = self.frame.height();
        if let Some(title) = self.title_box() {
            height -= self.title_band(&title);
        }
        if matches!(self.legend_position(), Some("b" | "t" | "tr")) && !self.legend_overlays() {
            let font = self.legend_font();
            height -= self.legend_band_height(&font, false);
        }
        height
    }

    /// How long a turned label may run.
    pub fn rotated_allowance(&mut self, box_: &FontBox) -> f64 {
        let headroom = self.furniture_height() / 2.0 - ROTATED_LABEL_HEADROOM_PT - rotated_label_anchor(box_);
        py_max(headroom / sin_45(), 0.0)
    }

    /// The band category labels take under the plot: turned, or wrapped to their bands.
    pub fn bottom_label_band(&mut self, font: &ChartFont, categories: &[String], plot_width: f64) -> f64 {
        let box_ = font.box_for(categories.iter().map(String::as_str));
        if self.labels_rotate(font, categories, plot_width) {
            let allowance = self.rotated_allowance(&box_);
            let cut: Vec<String> = categories
                .iter()
                .map(|t| {
                    let cut = truncate_label(t, font, allowance);
                    cut.strip_suffix(LABEL_ELLIPSIS).map_or(cut.clone(), str::to_owned)
                })
                .collect();
            let wide = widest(font, cut.iter().map(String::as_str));
            return FRAME_PADDING_PT + (box_.line_height() + wide) * sin_45() + CATEGORY_LABEL_GAP_EM * box_.size;
        }
        let lines = self.label_line_count(font, categories, plot_width);
        #[allow(clippy::cast_precision_loss)]
        let extra = (lines - 1) as f64;
        FRAME_PADDING_PT + box_.line_height() + box_.pitch() * extra + CATEGORY_LABEL_GAP_ASCENT * box_.ascent
    }

    /// A pie's, doughnut's or radar's region.
    pub fn polar_region(&mut self) -> Rect {
        let per_point = !self.is_radar();
        let frame = self.frame;
        let (mut left, mut right) = (frame.left + EDGE_INSET_PT, frame.right - EDGE_INSET_PT);
        let (mut top, mut bottom) = (frame.top + EDGE_INSET_PT, frame.bottom - EDGE_INSET_PT);
        if let Some(title) = self.title_box() {
            top = frame.top + self.title_band(&title) + EDGE_INSET_PT;
        }
        let legend = self.legend_position();
        if legend.is_some() && !self.legend_overlays() {
            let font = self.legend_font();
            let band = self.legend_band_height(&font, per_point);
            match legend {
                Some("b") => bottom -= band,
                Some("t" | "tr") => top += band,
                Some("r") => right = frame.right - self.legend_side_width(&font, per_point),
                Some("l") => left += self.legend_side_width(&font, per_point) - EDGE_INSET_PT,
                _ => {}
            }
        }
        if right - left < 1.0 {
            right = left + 1.0;
        }
        if bottom - top < 1.0 {
            bottom = top + 1.0;
        }
        Rect::new(left, top, right, bottom)
    }

    /// The plot's inset from the frame over its highest value label.
    pub fn top_inset(&self, label: &FontBox) -> f64 {
        py_max(EDGE_INSET_PT, TOP_INSET_BASE_PT + label.line_height() / 2.0)
    }

    /// The legend's position (`r` for an unknown one), `None` without a legend.
    pub fn legend_position(&self) -> Option<&'static str> {
        let legend = self.chart.legend.as_ref()?;
        Some(
            match legend.position.as_deref().filter(|p| !p.is_empty()).unwrap_or("r") {
                "b" => "b",
                "t" => "t",
                "l" => "l",
                "tr" => "tr",
                _ => "r",
            },
        )
    }

    /// Whether the legend is drawn over the plot.
    pub fn legend_overlays(&self) -> bool {
        self.chart.legend.as_ref().is_some_and(|l| l.overlay)
    }

    /// The title's body and box, resolved once.
    pub fn title(&mut self) -> Option<Title> {
        if let Some(cached) = &self.group.title {
            return cached.clone();
        }
        let built = self.build_title();
        self.group.title = Some(built.clone());
        built
    }

    fn build_title(&mut self) -> Option<Title> {
        let mut text = self.title_text();
        if text.is_none() && self.group.default_title_wanted {
            self.group.title_hidden = true;
            text = Some(String::new());
        }
        let text = text?;
        if self.chart.title.is_none() && !self.rules.auto_title {
            let auto = powerpoint_auto_title(self.chart, &text);
            let size = crate::layout::sources::powerpoint_auto_title_size(self.chart);
            let size = auto
                .paragraphs
                .first()
                .and_then(|p| p.runs.first())
                .and_then(|r| r.properties.as_ref())
                .and_then(|p| p.size)
                .unwrap_or(size);
            return Some(self.resolved_text(Some(&auto), &text, size, None, None));
        }
        let size = title_size(self.chart.title.as_ref());
        let rich = self.chart.title.as_ref().and_then(|t| t.rich.as_ref());
        let (mut face, mut bold) = (None, None);
        if self.rules.rich_title_fonts {
            if title_states_no_font(self.chart) {
                (face, bold) = (Some(RICH_TITLE_FALLBACK_FACE.to_owned()), Some(false));
            } else if title_states_defaults(self.chart) {
                bold = Some(true);
                if !title_states(self.chart, TitleProperty::Typeface) {
                    face = self.resolve.typeface("+mn-lt");
                }
            }
            if title_states(self.chart, TitleProperty::Bold) {
                bold = None;
            }
        }
        Some(self.resolved_text(rich, &text, size, face, bold))
    }

    /// A title's body through the consumer, its face, weight, size and colour settled.
    pub fn resolved_text(
        &self,
        rich: Option<&ooxml_drawingml::text::TextBody>,
        text: &str,
        size: f64,
        face: Option<String>,
        bold: Option<bool>,
    ) -> Title {
        let mut body = self.resolve.text(rich, text, size, "ctr");
        for run in body.paragraphs.iter_mut().flat_map(|p| p.runs.iter_mut()) {
            if let Some(face) = &face {
                run.properties.font_family = Some(face.clone());
            }
            if let Some(bold) = bold {
                run.properties.bold = bold;
            }
        }
        let first = body.paragraphs.iter().flat_map(|p| p.runs.first()).next();
        let family = first.and_then(|r| r.properties.font_family.clone());
        let size = first
            .and_then(|r| r.properties.font_size)
            .filter(|s| *s != 0.0)
            .unwrap_or(size);
        let colour = if states_colour(rich) {
            None
        } else {
            self.text_colour(&[])
        };
        for run in body.paragraphs.iter_mut().flat_map(|p| p.runs.iter_mut()) {
            if run.properties.font_size.is_none() {
                run.properties.font_size = Some(size);
            }
            if let Some(colour) = &colour {
                run.properties.colour = Some(colour.clone());
            }
        }
        (body, font_box(family.as_deref(), size))
    }

    /// The title's box, where there is a title.
    pub fn title_box(&mut self) -> Option<FontBox> {
        self.title().map(|t| t.1)
    }

    /// The title text shown.
    pub fn title_text(&mut self) -> Option<String> {
        if self.rules.auto_title {
            return self.shown_title_text();
        }
        if self.chart.auto_title_deleted {
            return None;
        }
        match &self.chart.title {
            None => {
                if self.chart.title_stated {
                    self.sole_series_name()
                } else {
                    None
                }
            }
            Some(title) => title.plain().filter(|t| !t.is_empty()),
        }
    }

    fn shown_title_text(&mut self) -> Option<String> {
        let chart = self.chart;
        if let Some(plain) = chart.title.as_ref().and_then(|t| t.plain()).filter(|t| !t.is_empty()) {
            return Some(plain);
        }
        let name = self.sole_series_name();
        if chart.title_stated || chart.title.is_some() {
            if !chart.auto_title_deleted && name.is_some() {
                return name;
            }
            self.group.default_title_wanted = self.default_title.is_none();
            return self.default_title.clone();
        }
        if chart.auto_title_deleted_stated && !chart.auto_title_deleted {
            return name;
        }
        None
    }

    fn sole_series_name(&self) -> Option<String> {
        let series: Vec<&crate::model::Series> = self.plots.iter().flat_map(|p| p.series.iter()).collect();
        match series.as_slice() {
            [only] => only.name.as_ref().and_then(|n| n.plain()),
            _ => None,
        }
    }

    /// The chart space's fill and line, and the plot area's.
    pub fn draw_background(&mut self, rect: Rect, plot: Option<Rect>) {
        let rules = self.rules;
        let mut fill = self.resolve.fill(self.chart.fill.as_ref());
        let mut outline = self.resolve.outline(self.chart.outline.as_ref());
        if self.chart.fill.is_none() {
            if let Some(colour) = rules.chart_fill {
                fill = Some(Fill::solid(&format!("#{colour}")));
            }
        }
        if self.chart.outline.is_none() {
            if let Some((colour, width)) = rules.chart_line {
                outline = Some(Outline::solid(
                    width,
                    ooxml_drawingml::drawable::Colour::opaque(&format!("#{colour}")),
                ));
            }
        }
        if !matches!(fill, None | Some(Fill::None)) || outline.is_some() {
            self.rect(self.frame, fill, outline, "rect");
        }
        let mut plot_fill = self.resolve.fill(self.chart.plot_area_fill.as_ref());
        let mut rect = rect;
        if let Some(colour) = rules.plot_fill {
            if let Some(plot) = plot {
                rect = plot;
            }
            if self.chart.plot_area_fill.is_none() && !(self.is_polar() && !self.is_radar()) {
                plot_fill = Some(Fill::solid(&format!("#{colour}")));
            }
        }
        let plot_line = if rules.plot_line {
            self.resolve.outline(self.chart.plot_area_outline.as_ref())
        } else {
            None
        };
        if !matches!(plot_fill, None | Some(Fill::None)) {
            self.rect(rect, plot_fill, plot_line, "rect");
        } else if plot_line.is_some() {
            self.rect(rect, None, plot_line, "rect");
        }
    }

    /// The band a title takes.
    pub fn title_band(&self, title: &FontBox) -> f64 {
        match self.rules.title {
            TitleRule::Pitch => title.pitch() + WORD_TITLE_PAD_PT,
            TitleRule::Lines => TITLE_BAND_LINES * title.line_height(),
        }
    }

    /// The title, drawn.
    pub fn draw_title(&mut self) {
        let Some((body, box_)) = self.title() else {
            return;
        };
        if self.group.title_hidden {
            return;
        }
        let baseline = match self.rules.title {
            TitleRule::Pitch => self.frame.top + WORD_TITLE_BASELINE_PT + WORD_TITLE_BASELINE_EM * box_.size,
            TitleRule::Lines => self.frame.top + TITLE_BASELINE_ASCENTS * box_.ascent,
        };
        self.text(body, self.frame.left, self.frame.width(), baseline, &box_);
    }

    /// The axis titles by side, resolved once.
    pub fn axis_titles(&mut self) -> Vec<(String, super::AxisTitle<'a>)> {
        if let Some(found) = &self.group.axis_titles {
            return found.clone();
        }
        let found = self.find_axis_titles();
        self.group.axis_titles = Some(found.clone());
        found
    }

    fn find_axis_titles(&mut self) -> Vec<(String, super::AxisTitle<'a>)> {
        let mut found: Vec<(String, super::AxisTitle<'a>)> = Vec::new();
        if !self.rules.axis_titles {
            return found;
        }
        let cartesian =
            !(self.is_polar() || self.is_of_pie() || self.is_three_d() || self.is_stock() || self.is_surface());
        let mut horizontal = self.horizontal();
        let (category_axis, value_axis) = if self.is_scatter() || self.is_bubble() {
            horizontal = false;
            self.scatter_axes()
        } else {
            (self.category_axis(), self.value_axis())
        };
        for axis in &self.chart.axes {
            if !axis.title_stated || axis.delete {
                continue;
            }
            let Some(title) = axis.title.as_ref().filter(|t| t.plain().is_some_and(|p| !p.is_empty())) else {
                self.not_drawn("default");
                continue;
            };
            let is_category = category_axis.is_some_and(|c| std::ptr::eq(c, axis));
            let is_value = value_axis.is_some_and(|v| std::ptr::eq(v, axis));
            if !cartesian || !(is_value || is_category) {
                self.not_drawn("chart");
                continue;
            }
            let side = if horizontal == is_category { "l" } else { "b" };
            let rotation = title
                .rich
                .as_ref()
                .and_then(|r| r.properties.as_ref())
                .and_then(|p| p.rotation);
            let wanted = if side == "l" { -5_400_000.0 } else { 0.0 };
            let misplaced = axis.position.as_deref().is_some_and(|p| p != side)
                || found.iter().any(|(s, _)| s == side)
                || rotation.is_some_and(|r| r != wanted);
            if misplaced {
                self.not_drawn("placement");
                continue;
            }
            let plain = title.plain().unwrap_or_default();
            let (body, box_) = self.resolved_text(title.rich.as_ref(), &plain, title_size(Some(title)), None, None);
            found.push((side.to_owned(), (axis, body, box_)));
        }
        found
    }

    fn not_drawn(&mut self, reason: &str) {
        if !self.axis_titles_not_drawn.iter().any(|r| r == reason) {
            self.axis_titles_not_drawn.push(reason.to_owned());
        }
    }

    /// The band an axis title on `side` takes.
    pub fn axis_title_band(&mut self, side: &str) -> f64 {
        self.axis_titles()
            .iter()
            .find(|(s, _)| s == side)
            .map_or(0.0, |(_, t)| t.2.pitch() + WORD_TITLE_PAD_PT)
    }

    /// The axis titles, drawn beside `rect`.
    pub fn draw_axis_titles(&mut self, rect: Rect) {
        let titles = self.axis_titles();
        if titles.is_empty() {
            return;
        }
        let legend = if self.legend_overlays() {
            None
        } else {
            self.legend_position()
        };
        if let Some((_, (_, body, box_))) = titles.iter().find(|(s, _)| s == "b") {
            let mut edge = self.frame.bottom;
            if legend == Some("b") {
                let font = self.legend_font();
                edge -= self.legend_band_height(&font, false);
            }
            let baseline = edge - WORD_AXIS_TITLE_INSET_PT - box_.descent;
            let width = self.frame.width();
            self.text(
                body.clone(),
                rect.left + rect.width() / 2.0 - width / 2.0,
                width,
                baseline,
                box_,
            );
        }
        if let Some((_, (_, body, box_))) = titles.iter().find(|(s, _)| s == "l") {
            let mut edge = self.frame.left;
            if legend == Some("l") {
                let font = self.legend_font();
                edge += self.legend_side_width(&font, false) - EDGE_INSET_PT;
            }
            let baseline_x = edge + WORD_AXIS_TITLE_INSET_PT + box_.gap + box_.ascent;
            let width = self.frame.height();
            let height = box_.line_height() * 1.5;
            let centre_x = baseline_x - (self.first_baseline(box_) - height / 2.0);
            let centre_y = rect.top + rect.height() / 2.0;
            self.elements.push(Element::Shape(Box::new(Shape {
                transform: Transform {
                    rotation: -90.0,
                    ..Transform::of(
                        (centre_x - width / 2.0) * EMU_PER_POINT,
                        (centre_y - height / 2.0) * EMU_PER_POINT,
                        width * EMU_PER_POINT,
                        height * EMU_PER_POINT,
                    )
                },
                geometry: Geometry::preset("rect"),
                fill: None,
                outline: None,
                text: Some(body.clone()),
                ..Shape::default()
            })));
        }
    }
}
