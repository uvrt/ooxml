//! Owns: text on the plot -- axis labels (down the side, along the bottom wrapped or turned
//! and cut, beside a secondary axis) and data labels (their composed or own rich text, where
//! each sits against its bar, line point or area, and how its lines stand).
//!
//! Must not: draw the title or the legend.

use super::{at, percent_totals, Builder, LabelText, Labels, RichLine, Series};
use crate::layout::consts::{
    CATEGORY_LABEL_GAP_ASCENT, DATA_LABEL_GAP_PT, DATA_LABEL_INNER_GAP_PT, DATA_LABEL_LINE_GAP_EM, DEFAULT_GAP_WIDTH,
    EMU_PER_POINT, LABEL_ELLIPSIS, MIN_BAR_SLOTS, ROTATED_LABEL_DEGREES, ROTATED_LABEL_OFFSET_X_PT, VALUE_LABEL_GAP_EM,
};
use crate::layout::numbers::format_number;
use crate::layout::scale::{py_max, py_min, Scale};
use crate::layout::sources::{labels_shown, merged_run};
use crate::layout::text::{font_box, rotated_label_anchor, text_width, truncate_label, wrap_label, ChartFont, FontBox};
use crate::layout::three_d::Rect;
use crate::model::Axis;
use ooxml_drawingml::drawable::{
    Element, Fill, Geometry, Paragraph, ParagraphProperties, RunProperties, Shape, TextBody, TextRun, Transform,
};
use ooxml_drawingml::source;
use ooxml_drawingml::text::RunProperties as SourceRun;

/// Where a data label stands: the point, and how it sits against it.
pub(crate) type Anchor = (f64, f64, &'static str);

/// The point a label is for: its series, the categories, its index and value.
pub(crate) struct LabelPoint<'s> {
    pub item: &'s Series,
    pub categories: &'s [String],
    pub point: usize,
    pub value: Option<f64>,
}

/// A bar's place among its plot's: its series' order, its point and value, and the count of
/// categories.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BarSpot {
    pub order: usize,
    pub point: usize,
    pub value: f64,
    pub count: usize,
}

impl<'a> Builder<'a> {
    /// The value and category axis labels.
    pub fn draw_labels(
        &mut self,
        rect: Rect,
        scale: Scale,
        tick_texts: &[(f64, String)],
        categories: &[String],
        axes: (Option<&Axis>, Option<&Axis>),
        fonts: (&ChartFont, &ChartFont),
    ) {
        let (value_axis, category_axis) = axes;
        let (value_font, category_font) = fonts;
        let horizontal = self.horizontal();
        if labels_shown(value_axis) {
            if horizontal {
                let placed: Vec<(f64, String)> = tick_texts
                    .iter()
                    .map(|(v, t)| (self.value_to_x(rect, *v, scale), t.clone()))
                    .collect();
                self.labels_along_bottom(&placed, value_font, rect.bottom, None, true);
            } else {
                let placed: Vec<(f64, String)> = tick_texts
                    .iter()
                    .map(|(v, t)| (self.value_to_y(rect, *v, scale), t.clone()))
                    .collect();
                self.labels_down_left(rect, &placed, value_font, None);
            }
        }
        if !labels_shown(category_axis) || categories.is_empty() {
            return;
        }
        let count = categories.len();
        #[allow(clippy::cast_precision_loss)]
        let count_f = count as f64;
        if horizontal {
            let band = rect.height() / count_f;
            #[allow(clippy::cast_precision_loss)]
            let placed: Vec<(f64, String)> = categories
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    (
                        rect.bottom - (self.category_slot(i, count) as f64 + 0.5) * band,
                        t.clone(),
                    )
                })
                .collect();
            self.labels_down_left(rect, &placed, category_font, None);
            return;
        }
        let band = rect.width() / count_f;
        let axis_y = self.category_axis_position(rect, scale, category_axis, true);
        let in_band = (axis_y - rect.bottom).abs() < 0.01;
        if self.points_on_ticks() {
            let placed: Vec<(f64, String)> = self
                .category_positions(rect, count)
                .into_iter()
                .zip(categories.iter().cloned())
                .collect();
            self.labels_along_bottom(&placed, category_font, axis_y, None, true);
        } else if in_band && self.labels_rotate(category_font, categories, rect.width()) {
            let allowance = self.rotated_allowance(&category_font.box_for(categories.iter().map(String::as_str)));
            #[allow(clippy::cast_precision_loss)]
            let placed: Vec<(f64, String)> = categories
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    (
                        rect.left + (self.category_slot(i, count) as f64 + 0.5) * band,
                        truncate_label(t, category_font, allowance),
                    )
                })
                .collect();
            self.rotated_labels_along_bottom(&placed, category_font, axis_y);
        } else {
            #[allow(clippy::cast_precision_loss)]
            let placed: Vec<(f64, String)> = categories
                .iter()
                .enumerate()
                .map(|(i, t)| (rect.left + self.category_slot(i, count) as f64 * band, t.clone()))
                .collect();
            self.labels_along_bottom(&placed, category_font, axis_y, Some(band), false);
        }
    }

    fn rotated_labels_along_bottom(&mut self, labels: &[(f64, String)], font: &ChartFont, axis_y: f64) {
        let box_ = font.box_for(labels.iter().map(|(_, t)| t.as_str()));
        let radians = ROTATED_LABEL_DEGREES.to_radians();
        let (cos, sin) = (radians.cos(), radians.sin());
        for (position, text) in labels {
            if text.is_empty() {
                continue;
            }
            let width = font.width(text) + box_.size;
            let height = box_.line_height() * 1.5;
            let overhang = if text.ends_with(LABEL_ELLIPSIS) {
                font.width(LABEL_ELLIPSIS)
            } else {
                0.0
            };
            let offset_x = width / 2.0 - box_.size / 2.0 - overhang;
            let offset_y = self.first_baseline(&box_) - height / 2.0;
            let turned_x = offset_x * cos - offset_y * sin;
            let turned_y = offset_x * sin + offset_y * cos;
            let centre_x = position + ROTATED_LABEL_OFFSET_X_PT - turned_x;
            let centre_y = axis_y + rotated_label_anchor(&box_) - turned_y;
            let body = self.label_body(text, font, "r", None);
            self.elements.push(Element::Shape(Box::new(Shape {
                transform: Transform {
                    rotation: ROTATED_LABEL_DEGREES,
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
                text: Some(body),
            })));
        }
    }

    /// Labels down the left of the plot (or of `axis_x`), right-aligned, each centred on its
    /// position.
    pub fn labels_down_left(&mut self, rect: Rect, labels: &[(f64, String)], font: &ChartFont, axis_x: Option<f64>) {
        let box_ = font.box_;
        let edge = axis_x.unwrap_or(rect.left);
        let width = edge - self.frame.left - box_.descent - VALUE_LABEL_GAP_EM * box_.size;
        for (y, text) in labels {
            if text.is_empty() {
                continue;
            }
            let body = self.label_body(text, font, "r", None);
            self.text(body, self.frame.left, width, y + box_.ink_centre(), &box_);
        }
    }

    /// Labels down the right of the plot (a secondary axis'), left-aligned.
    pub fn labels_down_right(&mut self, rect: Rect, labels: &[(f64, String)], font: &ChartFont) {
        let box_ = font.box_;
        let left = rect.right + box_.descent + VALUE_LABEL_GAP_EM * box_.size;
        let width = py_max(self.frame.right - left, box_.size);
        for (y, text) in labels {
            if text.is_empty() {
                continue;
            }
            let body = self.label_body(text, font, "l", None);
            self.text(body, left, width, y + box_.ink_centre(), &box_);
        }
    }

    /// Labels along the bottom: centred on their positions, or wrapped in their bands.
    pub fn labels_along_bottom(
        &mut self,
        labels: &[(f64, String)],
        font: &ChartFont,
        axis_y: f64,
        width: Option<f64>,
        centred: bool,
    ) {
        let box_ = font.box_;
        let baseline = axis_y + box_.ascent + CATEGORY_LABEL_GAP_ASCENT * box_.ascent;
        for (position, text) in labels {
            if text.is_empty() {
                continue;
            }
            let (left, box_width, lines) = if centred {
                let span = font.width(text) + box_.size;
                (position - span / 2.0, span, vec![text.clone()])
            } else {
                let box_width = width.filter(|w| *w != 0.0).unwrap_or(box_.size);
                (*position, box_width, wrap_label(text, font, box_width))
            };
            for (index, line) in lines.iter().enumerate() {
                let body = self.label_body(line, font, "ctr", None);
                #[allow(clippy::cast_precision_loss)]
                self.text(body, left, box_width, baseline + index as f64 * box_.pitch(), &box_);
            }
        }
    }

    /// Each point's data label.
    pub fn draw_data_labels(&mut self, rect: Rect, series: &[Series], categories: &[String], scale: Scale) {
        if categories.is_empty() {
            return;
        }
        let horizontal = self.horizontal();
        let totals = percent_totals(series);
        for (order, item) in series.iter().enumerate() {
            for point in 0..categories.len() {
                let Some(labels) = item
                    .labels_at(point)
                    .filter(|l| l.anything() && l.font.is_some())
                    .cloned()
                else {
                    continue;
                };
                let Some(value) = at(&item.values, point) else { continue };
                let at_point = LabelPoint {
                    item,
                    categories,
                    point,
                    value: Some(value),
                };
                let text = self.label_text(&labels, &at_point, &totals, None);
                if text.text.is_empty() {
                    continue;
                }
                let spot = BarSpot {
                    order,
                    point,
                    value,
                    count: categories.len(),
                };
                let Some(geometry) = self.label_anchor(rect, series, spot, scale, horizontal) else {
                    continue;
                };
                self.place_label(&text, &labels, geometry, None, None);
            }
        }
    }

    /// A point's label text: its own, else the parts it shows joined by lines.
    pub fn label_text(
        &self,
        labels: &Labels,
        at_point: &LabelPoint<'_>,
        totals: &[f64],
        bubble: Option<f64>,
    ) -> LabelText {
        let LabelPoint {
            item,
            categories,
            point,
            value,
        } = *at_point;
        let value = value.unwrap_or(0.0);
        let percent = (!totals.is_empty()).then(|| self.percent_text(value, point, totals));
        if let Some(own) = self.own_label(labels, at_point, percent, bubble) {
            return own;
        }
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
            parts.push(self.percent_text(value, point, totals));
        }
        if labels.show_value {
            let code = labels.number_format.as_deref().or(item.format_code.as_deref());
            parts.push(format_number(value, code, self.locale));
        }
        if labels.show_bubble_size {
            if let Some(size) = bubble {
                parts.push(format_number(size, labels.number_format.as_deref(), self.locale));
            }
        }
        LabelText::plain(Self::with_range(labels, item, point, parts).join("\n"))
    }

    fn percent_text(&self, value: f64, point: usize, totals: &[f64]) -> String {
        let total = totals.get(point).copied().unwrap_or(0.0);
        format_number(if total != 0.0 { value / total } else { 0.0 }, Some("0%"), self.locale)
    }

    /// The parts with the label range's text joined before them.
    pub fn with_range(labels: &Labels, item: &Series, point: usize, parts: Vec<String>) -> Vec<String> {
        if !labels.show_range {
            return parts;
        }
        let text = item.label_range.get(point).cloned().flatten().filter(|t| !t.is_empty());
        let joined: Vec<String> = text.into_iter().chain(parts).collect();
        if joined.is_empty() {
            Vec::new()
        } else {
            vec![joined.join("; ")]
        }
    }

    /// A label's own text, its fields filled in.
    pub fn own_label(
        &self,
        labels: &Labels,
        at_point: &LabelPoint<'_>,
        percent: Option<String>,
        bubble: Option<f64>,
    ) -> Option<LabelText> {
        let LabelPoint {
            item,
            categories,
            point,
            value,
        } = *at_point;
        let own = labels.text.as_ref()?;
        let Some(rich) = &own.rich else {
            return Some(LabelText::rich(vec![vec![(
                own.cached.clone().unwrap_or_default(),
                None,
            )]]));
        };
        let code = labels.number_format.as_deref().or(item.format_code.as_deref());
        let value_text = value.map(|v| format_number(v, code, self.locale));
        let category = categories.get(point).cloned();
        let field = |name: &str| -> Option<String> {
            match name {
                "CELLRANGE" => item.label_range.get(point).cloned().flatten(),
                "VALUE" | "YVALUE" => value_text.clone(),
                "SERIESNAME" => item.name.clone(),
                "CATEGORYNAME" | "XVALUE" => category.clone(),
                "PERCENTAGE" => percent.clone(),
                "BUBBLESIZE" => bubble.map(|b| format_number(b, labels.number_format.as_deref(), self.locale)),
                _ => None,
            }
        };
        let mut lines: Vec<RichLine> = Vec::new();
        for paragraph in &rich.paragraphs {
            let default = paragraph.properties.as_ref().and_then(|p| p.default_run.as_ref());
            let mut line: RichLine = Vec::new();
            for run in &paragraph.runs {
                let mut text = run.text.clone();
                if let Some(kind) = run.field_type.as_ref().filter(|k| !k.is_empty()) {
                    if let Some(shown) = field(&kind.to_uppercase()) {
                        text = shown;
                    }
                }
                let properties = merged_run(run.properties.as_ref(), default);
                if text == "\n" {
                    lines.push(std::mem::take(&mut line));
                    continue;
                }
                line.push((text, properties));
            }
            lines.push(line);
        }
        Some(LabelText::rich(lines))
    }

    fn label_anchor(
        &self,
        rect: Rect,
        series: &[Series],
        spot: BarSpot,
        scale: Scale,
        horizontal: bool,
    ) -> Option<Anchor> {
        let BarSpot {
            order,
            point,
            value,
            count,
        } = spot;
        let item = series.get(order)?;
        if self.is_area() {
            let xs = self.category_positions(rect, count);
            let x = *xs.get(point)?;
            let mut start = 0.0;
            if self.stacked("standard") {
                for earlier in series.iter().take(order) {
                    start += at(&earlier.values, point).filter(|v| *v != 0.0).unwrap_or(0.0);
                }
            }
            let mut value = value;
            if self.grouping_or("") == "percentStacked" {
                let totals = percent_totals(series);
                let total = totals.get(point).copied().unwrap_or(0.0);
                if total == 0.0 {
                    return None;
                }
                start /= total;
                value /= total;
            }
            return Some((x, self.value_to_y(rect, start + value / 2.0, scale), "centre"));
        }
        if self.is_line() {
            let xs = self.category_positions(rect, count.max(1));
            let x = xs.get(point).copied().unwrap_or(rect.left);
            return Some((x + item.marker_size / 2.0, self.value_to_y(rect, value, scale), "right"));
        }
        let shape = self.bar_box(rect, series, spot, scale, horizontal)?;
        let position = item
            .labels
            .as_ref()
            .and_then(|l| l.position.clone())
            .filter(|p| !p.is_empty());
        let position = position.unwrap_or_else(|| self.label_default().to_owned());
        let centre_x = (shape.left + shape.right) / 2.0;
        let centre_y = (shape.top + shape.bottom) / 2.0;
        let position = position.as_str();
        if scale.reversed && value >= 0.0 {
            return Some(match (position, horizontal) {
                ("ctr", _) => (centre_x, centre_y, "centre"),
                ("inEnd", true) => (shape.left, centre_y, "outside-x-flip"),
                ("inBase", true) => (shape.right, centre_y, "inside-x"),
                (_, true) => (shape.left, centre_y, "outside-x-left"),
                ("inEnd", false) => (centre_x, shape.bottom, "inside-base-y"),
                ("inBase", false) => (centre_x, shape.top, "inside-y"),
                (_, false) => (centre_x, shape.bottom, "below"),
            });
        }
        if horizontal {
            let (end, base) = if value >= 0.0 {
                (shape.right, shape.left)
            } else {
                (shape.left, shape.right)
            };
            return Some(match position {
                "ctr" => (centre_x, centre_y, "centre"),
                "inEnd" => (end, centre_y, "inside-x"),
                "inBase" => (base, centre_y, "outside-x-flip"),
                _ => (end, centre_y, "outside-x"),
            });
        }
        let (end, base) = if value >= 0.0 {
            (shape.top, shape.bottom)
        } else {
            (shape.bottom, shape.top)
        };
        Some(match position {
            "ctr" => (centre_x, centre_y, "centre"),
            "inEnd" => (centre_x, end, "inside-y"),
            "inBase" => (centre_x, base, "inside-base-y"),
            _ => (centre_x, end, "outside-y"),
        })
    }

    /// Where a data label defaults to.
    pub fn label_default(&self) -> &'static str {
        if self.is_area() {
            return "ctr";
        }
        if self.is_scatter() {
            return "r";
        }
        if self.is_line() || self.is_polar() {
            return "outEnd";
        }
        if self.stacked("clustered") {
            "ctr"
        } else {
            "outEnd"
        }
    }

    /// A data label's lines placed against its anchor.
    pub fn place_label(
        &mut self,
        text: &LabelText,
        labels: &Labels,
        geometry: Anchor,
        side: Option<f64>,
        edge: Option<f64>,
    ) {
        let (x, y, placement) = geometry;
        let Some(font) = labels.font.clone() else { return };
        let rich = text.lines.as_ref().filter(|l| !l.is_empty());
        let box_ = rich.map_or(font.box_, |r| self.rich_box(r, &font));
        let lines: Vec<&str> = text.text.split('\n').collect();
        let widths: Vec<f64> = match rich {
            Some(r) => r.iter().map(|line| self.rich_width(line, &font)).collect(),
            None => lines.iter().map(|l| font.width(l)).collect(),
        };
        let width = widths
            .iter()
            .copied()
            .fold(None, |m: Option<f64>, w| Some(m.map_or(w, |m| py_max(m, w))))
            .unwrap_or(0.0)
            + box_.size;
        #[allow(clippy::cast_precision_loss)]
        let block = box_.line_height() * lines.len() as f64;
        let side_gap = side.unwrap_or(DATA_LABEL_LINE_GAP_EM * box_.size);
        let (baseline, left, align) = match placement {
            "centre" => (
                y + box_.ink_centre() - block + box_.line_height(),
                x - width / 2.0,
                "ctr",
            ),
            "outside-y" => (
                y - edge.unwrap_or(DATA_LABEL_GAP_PT) - box_.descent,
                x - width / 2.0,
                "ctr",
            ),
            "inside-y" => (y + DATA_LABEL_INNER_GAP_PT + box_.ascent, x - width / 2.0, "ctr"),
            "inside-base-y" => (y - DATA_LABEL_GAP_PT - box_.descent, x - width / 2.0, "ctr"),
            "below" => (
                y + edge.unwrap_or(DATA_LABEL_GAP_PT) + box_.ascent,
                x - width / 2.0,
                "ctr",
            ),
            "right" => (y + box_.ink_centre(), x + side_gap, "l"),
            "left" => (y + box_.ink_centre(), x - side_gap - width, "r"),
            "inside-x" => (y + box_.ink_centre(), x - DATA_LABEL_INNER_GAP_PT - width, "r"),
            "outside-x-flip" => (y + box_.ink_centre(), x + DATA_LABEL_GAP_PT, "l"),
            "outside-x-left" => (y + box_.ink_centre(), x - DATA_LABEL_GAP_PT - width, "r"),
            _ => (y + box_.ink_centre(), x + DATA_LABEL_GAP_PT, "l"),
        };
        let mut first = baseline;
        if placement != "below" {
            #[allow(clippy::cast_precision_loss)]
            let more = (lines.len() - 1) as f64;
            first -= box_.line_height() * more;
        }
        for (index, line) in lines.iter().enumerate() {
            let body = match rich.and_then(|r| r.get(index)) {
                Some(runs) => self.rich_body(runs, &font, align, labels.colour.clone()),
                None => self.label_body(line, &font, align, labels.colour.clone()),
            };
            #[allow(clippy::cast_precision_loss)]
            self.text(body, left, width, first + index as f64 * box_.line_height(), &box_);
        }
    }

    /// A label's lines centred on `(x, y)`.
    pub fn centred_label(&mut self, lines: &[String], font: &ChartFont, x: f64, y: f64, rich: Option<&LabelText>) {
        let own: Vec<String>;
        let lines: Vec<&str> = match rich {
            Some(r) => {
                own = r.text.split('\n').map(str::to_owned).collect();
                own.iter().map(String::as_str).collect()
            }
            None => lines.iter().map(String::as_str).collect(),
        };
        let runs = rich.and_then(|r| r.lines.as_ref()).filter(|l| !l.is_empty());
        let box_ = runs.map_or(font.box_, |r| self.rich_box(r, font));
        let widths: Vec<f64> = match runs {
            Some(r) => r.iter().map(|line| self.rich_width(line, font)).collect(),
            None => lines.iter().map(|l| font.width(l)).collect(),
        };
        let width = widths
            .iter()
            .copied()
            .fold(None, |m: Option<f64>, w| Some(m.map_or(w, |m| py_max(m, w))))
            .unwrap_or(0.0)
            + box_.size;
        #[allow(clippy::cast_precision_loss)]
        let block = box_.line_height() * lines.len() as f64;
        let first = y - block / 2.0 + box_.ascent;
        for (index, line) in lines.iter().enumerate() {
            let body = match runs.and_then(|r| r.get(index)) {
                Some(line_runs) => self.rich_body(line_runs, font, "ctr", None),
                None => self.label_body(line, font, "ctr", None),
            };
            #[allow(clippy::cast_precision_loss)]
            self.text(
                body,
                x - width / 2.0,
                width,
                first + index as f64 * box_.line_height(),
                &box_,
            );
        }
    }

    fn rich_run(
        &self,
        properties: Option<&SourceRun>,
        font: &ChartFont,
        colour: Option<ooxml_drawingml::drawable::Colour>,
    ) -> (Option<String>, f64, RunProperties) {
        let mut size = font.size();
        let mut family = font.family.clone();
        let mut drawn = colour
            .or_else(|| font.colour.clone())
            .unwrap_or_else(|| self.style.colour.clone());
        let (mut bold, mut italic, mut underline) = (false, false, false);
        if let Some(p) = properties {
            size = p.size.filter(|s| *s != 0.0).unwrap_or(size);
            if let Some(face) = p.latin.as_deref().filter(|f| !f.is_empty()) {
                family = self.resolve.typeface(face).filter(|f| !f.is_empty()).or(family);
            }
            if let Some(c) = &p.colour {
                if let Some(Fill::Solid(solid)) = self.resolve.fill(Some(&source::Fill::Solid(c.clone()))) {
                    drawn = solid;
                }
            }
            bold = p.bold == Some(true);
            italic = p.italic == Some(true);
            underline = p.underline == Some(true);
        }
        let run = RunProperties {
            font_size: Some(size),
            font_family: family.clone(),
            font_family_ea: font.family_ea.clone(),
            colour: Some(drawn),
            bold,
            italic,
            underline,
            ..RunProperties::default()
        };
        (family, size, run)
    }

    fn rich_width(&self, line: &RichLine, font: &ChartFont) -> f64 {
        let mut total = 0.0;
        for (text, properties) in line {
            let (family, size, _) = self.rich_run(properties.as_ref(), font, None);
            total += text_width(text, family.as_deref(), size, font.family_ea.as_deref());
        }
        total
    }

    fn rich_box(&self, lines: &[RichLine], font: &ChartFont) -> FontBox {
        let mut best = font.box_;
        for (_, properties) in lines.iter().flatten() {
            let (family, size, _) = self.rich_run(properties.as_ref(), font, None);
            if size > best.size {
                best = font_box(family.as_deref(), size);
            }
        }
        best
    }

    fn rich_body(
        &self,
        line: &RichLine,
        font: &ChartFont,
        align: &str,
        colour: Option<ooxml_drawingml::drawable::Colour>,
    ) -> TextBody {
        let mut runs: Vec<TextRun> = line
            .iter()
            .map(|(text, p)| TextRun {
                text: text.clone(),
                properties: self.rich_run(p.as_ref(), font, colour.clone()).2,
            })
            .collect();
        if runs.is_empty() {
            runs.push(TextRun {
                text: String::new(),
                properties: self.rich_run(None, font, colour).2,
            });
        }
        TextBody {
            paragraphs: vec![Paragraph {
                runs,
                properties: ParagraphProperties::aligned(align),
                end_properties: None,
            }],
            properties: super::primitives::chart_text_body(),
        }
    }

    /// The box a bar's label is placed against (as `draw_bars` stands it).
    fn bar_box(&self, rect: Rect, series: &[Series], spot: BarSpot, scale: Scale, horizontal: bool) -> Option<Rect> {
        let BarSpot {
            order,
            point,
            value,
            count: categories,
        } = spot;
        if categories == 0 {
            return None;
        }
        let stacked = self.stacked("clustered");
        #[allow(clippy::cast_precision_loss)]
        let band = (if horizontal { rect.height() } else { rect.width() }) / categories as f64;
        let gap_width = self.plot.gap_width.unwrap_or(DEFAULT_GAP_WIDTH);
        let slots = if stacked { 1 } else { series.len() };
        #[allow(clippy::cast_precision_loss)]
        let slots_f = slots as f64;
        let overlap = self.plot.overlap.unwrap_or(0.0);
        let size = band
            / py_max(
                slots_f + gap_width / 100.0 - (slots_f - 1.0) * overlap / 100.0,
                MIN_BAR_SLOTS,
            );
        let step = size * (1.0 - overlap / 100.0);
        let cluster = size + step * (slots_f - 1.0);
        let place = self.category_slot(point, categories);
        let index = if horizontal { categories - 1 - place } else { place };
        #[allow(clippy::cast_precision_loss)]
        let band_start = (if horizontal { rect.top } else { rect.left }) + index as f64 * band;
        let centre = band_start + band / 2.0;
        let mut slot = if stacked { 0 } else { order };
        if !stacked && horizontal && self.rules.bars_upward {
            slot = slots - 1 - order;
        }
        #[allow(clippy::cast_precision_loss)]
        let mut offset = centre - cluster / 2.0 + slot as f64 * step;
        if self.categories_reversed() {
            offset = 2.0 * centre - offset - size;
        }
        let (start, end) = if stacked {
            let mut start = 0.0;
            for earlier in series.iter().take(order) {
                let earlier_value = at(&earlier.values, point).filter(|v| *v != 0.0).unwrap_or(0.0);
                if (earlier_value >= 0.0) == (value >= 0.0) {
                    start += earlier_value;
                }
            }
            (start, start + value)
        } else {
            (self.bar_base(scale), value)
        };
        Some(if horizontal {
            let (x0, x1) = (self.value_to_x(rect, start, scale), self.value_to_x(rect, end, scale));
            Rect::new(py_min(x0, x1), offset, py_max(x0, x1), offset + size)
        } else {
            let (y0, y1) = (self.value_to_y(rect, start, scale), self.value_to_y(rect, end, scale));
            Rect::new(offset, py_min(y0, y1), offset + size, py_max(y0, y1))
        })
    }
}
