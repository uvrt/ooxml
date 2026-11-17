//! Owns: the legend -- its entries' names and order, its bands and grid (a row or rows above
//! or below, a column at the side, Word's top-right column), its keys (swatch, line, marker)
//! and how its names are wrapped.
//!
//! Must not: lay out the plot (it gives the plot its bands' sizes).

use super::{Builder, Series};
use crate::layout::consts::{
    EDGE_INSET_PT, FRAME_PADDING_PT, LEGEND_BAND_MAX_FRACTION, LEGEND_BAND_PAD_PT, LEGEND_ENTRY_KEY_EM,
    LEGEND_ENTRY_SLACK, LEGEND_HORIZONTAL_OFFSET_PT, LEGEND_ROW_PITCH_PT, LEGEND_ROW_PITCH_RATIO, LEGEND_SIDE_LEAD_EM,
    LEGEND_SIDE_MAX_FRACTION, LEGEND_SIDE_TITLE_GAP_PT, LEGEND_SIDE_TRAIL_EM, LEGEND_SWATCH_EM, LEGEND_SWATCH_GAP_EM,
    LEGEND_WRAP_BAND_TRIM_PT, LINE_LEGEND_ENTRY_KEY_PT, LINE_LEGEND_KEY_GAP_PT, LINE_LEGEND_KEY_PT,
    WORD_LEGEND_LEAD_PT, WORD_LEGEND_LEFT_PT, WORD_LEGEND_TRAIL_PT,
};
use crate::layout::scale::{py_max, py_min};
use crate::layout::text::{legend_wrap, ChartFont, FontBox};
use crate::layout::three_d::Rect;
use crate::pymath;
use crate::rules::{LegendOrderRule, SideLegendRule, TopRightLegendRule};

/// A legend row's pitch for entries of `lines` lines.
pub(crate) fn legend_row_pitch(box_: &FontBox, lines: usize) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let lines = lines.max(1) as f64;
    LEGEND_ROW_PITCH_RATIO * box_.line_height() * lines + LEGEND_ROW_PITCH_PT
}

/// Where one legend entry stands.
struct Entry {
    x: f64,
    baseline: f64,
    swatch: f64,
    gap: f64,
    column: Option<f64>,
}

impl<'a> Builder<'a> {
    /// The legend's entry names (empty for an unnamed or deleted one is left out by callers).
    pub fn legend_names(&mut self, per_point: bool) -> Vec<String> {
        let deleted: Vec<i64> = self
            .chart
            .legend
            .as_ref()
            .map(|l| l.deleted_entries.clone())
            .unwrap_or_default();
        let gone = |index: usize| i64::try_from(index).is_ok_and(|i| deleted.contains(&i));
        if self.is_surface() {
            let scale = self.band_scale();
            return self
                .band_series(scale, None)
                .into_iter()
                .map(|s| s.name.unwrap_or_default())
                .collect();
        }
        let plots = self.drawn_plots();
        let sources: Vec<&crate::model::Series> = plots.iter().flat_map(|p| p.series.iter()).collect();
        if !per_point {
            return sources
                .iter()
                .enumerate()
                .filter(|(index, source)| source.name.is_some() && !gone(*index))
                .map(|(_, source)| source.name.as_ref().and_then(|n| n.plain()).unwrap_or_default())
                .collect();
        }
        for source in &sources {
            if source.categories.iter().any(|c| !c.is_empty()) {
                return self
                    .category_texts(source)
                    .into_iter()
                    .enumerate()
                    .filter(|(index, name)| !name.is_empty() && !gone(*index))
                    .map(|(_, name)| name)
                    .collect();
            }
        }
        Vec::new()
    }

    fn legend_grid(&self, widths: &[f64]) -> (usize, usize) {
        if widths.is_empty() {
            return (1, 0);
        }
        let cap = LEGEND_BAND_MAX_FRACTION * self.frame.width();
        if pymath::sum(widths.iter().copied()) <= cap {
            return (1, widths.len());
        }
        let column = widths.iter().copied().fold(f64::NEG_INFINITY, py_max);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let fits = if column > 0.0 {
            ((cap / column).floor() as usize).max(1)
        } else {
            1
        };
        let rows = widths.len().div_ceil(fits);
        (rows, widths.len().div_ceil(rows))
    }

    fn legend_entry_widths(&mut self, font: &ChartFont, per_point: bool) -> Vec<f64> {
        let cell = self.legend_key_cell(font);
        self.legend_names(per_point)
            .iter()
            .filter(|n| !n.is_empty())
            .map(|n| cell + font.width(n))
            .collect()
    }

    /// The band a legend above or below the plot takes.
    pub fn legend_band_height(&mut self, font: &ChartFont, per_point: bool) -> f64 {
        let widths = self.legend_entry_widths(font, per_point);
        let (rows, _) = self.legend_grid(&widths);
        let pad = LEGEND_BAND_PAD_PT - if rows > 1 { LEGEND_WRAP_BAND_TRIM_PT } else { 0.0 };
        #[allow(clippy::cast_precision_loss)]
        let rows_f = rows as f64;
        rows_f * legend_row_pitch(&font.box_, 1) + pad
    }

    fn legend_side_metrics(&mut self, font: &ChartFont, per_point: bool) -> (f64, f64) {
        let names: Vec<String> = self
            .legend_names(per_point)
            .into_iter()
            .filter(|n| !n.is_empty())
            .collect();
        let (key, key_gap) = self.legend_key_size(font);
        let pads = if self.rules.side_legend == SideLegendRule::Word {
            key + key_gap + self.legend_side_lead(font) + self.legend_side_trail(font)
        } else {
            key + key_gap + (LEGEND_SIDE_LEAD_EM + LEGEND_SIDE_TRAIL_EM) * font.size()
        };
        let widest = super::frame::widest(font, names.iter().map(String::as_str));
        let cap = self.frame.width() * LEGEND_SIDE_MAX_FRACTION;
        if widest + pads <= cap {
            return (widest + pads, widest);
        }
        let column = py_max(cap - pads, font.size());
        let lines: Vec<String> = names.iter().flat_map(|n| legend_wrap(n, font, column)).collect();
        let drawn = if lines.is_empty() {
            column
        } else {
            super::frame::widest(font, lines.iter().map(String::as_str))
        };
        (py_min(drawn + pads, cap), column)
    }

    fn legend_side_lead(&self, font: &ChartFont) -> f64 {
        if self.rules.side_legend == SideLegendRule::Word {
            WORD_LEGEND_LEAD_PT + LEGEND_SWATCH_EM * font.size() / 2.0
        } else {
            LEGEND_SIDE_LEAD_EM * font.size()
        }
    }

    fn legend_side_trail(&self, font: &ChartFont) -> f64 {
        if self.rules.side_legend == SideLegendRule::Word {
            WORD_LEGEND_TRAIL_PT
        } else {
            LEGEND_SIDE_TRAIL_EM * font.size()
        }
    }

    /// The width a legend at the side takes.
    pub fn legend_side_width(&mut self, font: &ChartFont, per_point: bool) -> f64 {
        self.legend_side_metrics(font, per_point).0
    }

    /// The legend, drawn.
    pub fn draw_legend(
        &mut self,
        rect: Rect,
        series: &[Series],
        per_point: bool,
        categories: Option<&[String]>,
        second_band: f64,
    ) {
        let Some(position) = self.legend_position() else { return };
        let Some(first_item) = series.first() else { return };
        let mut entries = self.legend_entries(series, first_item, per_point, categories);
        if entries.is_empty() {
            return;
        }
        let font = self.legend_font();
        let box_ = font.box_;
        let (swatch, gap) = self.legend_key_size(&font);
        let top_column = position == "tr" && self.rules.top_right_legend == TopRightLegendRule::Column;
        if matches!(position, "b" | "t" | "tr") && !top_column {
            self.draw_legend_band(position, &mut entries, &font, (swatch, gap));
            return;
        }
        let x = if self.rules.side_legend == SideLegendRule::Word || top_column {
            if position == "l" {
                self.frame.left + WORD_LEGEND_LEFT_PT + LEGEND_SWATCH_EM * box_.size / 2.0
            } else {
                self.frame.right - self.legend_side_width(&font, per_point) + self.legend_side_lead(&font)
            }
        } else if position == "l" {
            self.frame.left + EDGE_INSET_PT
        } else {
            rect.right + second_band + LEGEND_SIDE_LEAD_EM * box_.size
        };
        let column = self.legend_side_metrics(&font, per_point).1;
        let lines = entries
            .iter()
            .map(|e| legend_wrap(e.name.as_deref().unwrap_or(""), &font, column).len())
            .max()
            .unwrap_or(1);
        let pitch = legend_row_pitch(&box_, lines);
        let inside = legend_row_pitch(&box_, 1) / 2.0 + box_.ink_centre();
        let band = self.legend_title_band();
        #[allow(clippy::cast_precision_loss)]
        let count = entries.len() as f64;
        let mut y = self.frame.top + band + (self.frame.height() - band - pitch * count) / 2.0;
        if top_column {
            let title = self.title_box().map_or(0.0, |t| self.title_band(&t));
            y = self.frame.top + LEGEND_BAND_PAD_PT + title;
        }
        for item in &entries {
            self.legend_entry(
                item,
                &Entry {
                    x,
                    baseline: y + inside,
                    swatch,
                    gap,
                    column: Some(column),
                },
                &font,
            );
            y += pitch;
        }
    }

    /// The series a legend lists: each named point of the first series (per point), or each
    /// named series, less the deleted entries, in the legend's order.
    fn legend_entries(
        &self,
        series: &[Series],
        first: &Series,
        per_point: bool,
        categories: Option<&[String]>,
    ) -> Vec<Series> {
        let deleted: Vec<i64> = self
            .chart
            .legend
            .as_ref()
            .map(|l| l.deleted_entries.clone())
            .unwrap_or_default();
        let gone = |index: usize| i64::try_from(index).is_ok_and(|i| deleted.contains(&i));
        if per_point {
            return categories
                .unwrap_or(&[])
                .iter()
                .enumerate()
                .filter(|(index, name)| !name.is_empty() && !gone(*index))
                .map(|(index, name)| {
                    let mut item = first.clone();
                    item.name = Some(name.clone());
                    item.fill = self.point_fill(first, index);
                    item
                })
                .collect();
        }
        let mut kept: Vec<Series> = series
            .iter()
            .enumerate()
            .filter(|(index, item)| !gone(*index) && item.name.as_ref().is_some_and(|n| !n.is_empty()))
            .map(|(_, item)| item.clone())
            .collect();
        if self.legend_reversed() {
            kept.reverse();
        }
        kept
    }

    fn draw_legend_band(&mut self, position: &str, entries: &mut [Series], font: &ChartFont, key: (f64, f64)) {
        let (swatch, gap) = key;
        let box_ = font.box_;
        let cell = self.legend_key_cell(font);
        let widths: Vec<f64> = entries
            .iter()
            .map(|e| cell + font.width(e.name.as_deref().unwrap_or("")))
            .collect();
        let total = pymath::sum(widths.iter().copied());
        let (rows, columns) = self.legend_grid(&widths);
        let pitch = legend_row_pitch(&box_, 1);
        let inside = pitch / 2.0 + box_.ink_centre();
        #[allow(clippy::cast_precision_loss)]
        let (rows_f, columns_f) = (rows as f64, columns as f64);
        let first = if position == "b" {
            let pad = LEGEND_BAND_PAD_PT - if rows > 1 { LEGEND_WRAP_BAND_TRIM_PT } else { 0.0 };
            self.frame.bottom - pad - pitch + inside - (rows_f - 1.0) * pitch
        } else {
            let mut first = self.frame.top + LEGEND_BAND_PAD_PT + inside;
            if self.rules.legend_under_title {
                if let Some(title) = self.title_box() {
                    first += self.title_band(&title);
                }
            }
            first
        };
        if rows > 1 {
            let column = widths.iter().copied().fold(f64::NEG_INFINITY, py_max);
            let start = self.frame.left + (self.frame.width() - columns_f * column) / 2.0 + LEGEND_HORIZONTAL_OFFSET_PT;
            for (index, item) in entries.iter().enumerate() {
                let (row, slot) = (index / columns.max(1), index % columns.max(1));
                #[allow(clippy::cast_precision_loss)]
                let (row_f, slot_f) = (row as f64, slot as f64);
                let entry = Entry {
                    x: start + slot_f * column + (cell - swatch) / 2.0,
                    baseline: first + row_f * pitch,
                    swatch,
                    gap,
                    column: None,
                };
                self.legend_entry(item, &entry, font);
            }
            return;
        }
        let slack = py_min(
            LEGEND_ENTRY_SLACK * total,
            LEGEND_BAND_MAX_FRACTION * self.frame.width() - total,
        );
        #[allow(clippy::cast_precision_loss)]
        let count = entries.len() as f64;
        let entry_gap = py_max(slack, 0.0) / (count + 1.0);
        let run = total + entry_gap * (count - 1.0);
        let mut x = self.frame.left + (self.frame.width() - run) / 2.0 + LEGEND_HORIZONTAL_OFFSET_PT;
        for (item, width) in entries.iter().zip(widths.iter()) {
            let entry = Entry {
                x: x + (cell - swatch) / 2.0,
                baseline: first,
                swatch,
                gap,
                column: None,
            };
            self.legend_entry(item, &entry, font);
            x += width + entry_gap;
        }
    }

    fn legend_reversed(&self) -> bool {
        if self.rules.legend_order != LegendOrderRule::Stack
            || !matches!(self.plot.kind.as_str(), "barChart" | "bar3DChart")
        {
            return false;
        }
        let stacked = self.stacked("clustered");
        if self.horizontal() {
            return !stacked && !self.categories_reversed();
        }
        stacked && matches!(self.legend_position(), Some("l" | "r"))
    }

    fn legend_title_band(&mut self) -> f64 {
        self.title_box()
            .map_or(0.0, |t| t.line_height() + LEGEND_SIDE_TITLE_GAP_PT)
    }

    fn legend_key_size(&self, font: &ChartFont) -> (f64, f64) {
        if self.line_legend_key() {
            return (LINE_LEGEND_KEY_PT, LINE_LEGEND_KEY_GAP_PT);
        }
        (LEGEND_SWATCH_EM * font.size(), LEGEND_SWATCH_GAP_EM * font.size())
    }

    fn line_legend_key(&self) -> bool {
        self.group.line_legend_keys || self.is_line_keyed()
    }

    fn legend_key_cell(&self, font: &ChartFont) -> f64 {
        if self.line_legend_key() {
            return LINE_LEGEND_ENTRY_KEY_PT;
        }
        LEGEND_ENTRY_KEY_EM * font.size()
    }

    fn legend_entry(&mut self, item: &Series, entry: &Entry, font: &ChartFont) {
        let box_ = font.box_;
        let (x, swatch) = (entry.x, entry.swatch);
        let centre = entry.baseline - box_.ink_centre();
        if item.line_keyed {
            if let Some(line) = &item.line {
                self.line(x, centre, x + swatch, centre, Some(line));
            }
            if item.marker_symbol.is_some() {
                self.marker((x + swatch / 2.0, centre), item);
            }
        } else {
            let half = LEGEND_SWATCH_EM * font.size() / 2.0;
            let outline = if item.fill.is_none() || self.rules.legend_key_outlines {
                item.outline.clone()
            } else {
                None
            };
            self.rect(
                Rect::new(x, centre - half, x + swatch, centre + half),
                item.fill.clone(),
                outline,
                "rect",
            );
        }
        let left = x + swatch + entry.gap;
        let name = item.name.clone().unwrap_or_default();
        let natural = font.width(&name) + box_.size;
        let edge = self.frame.right - FRAME_PADDING_PT - left;
        let mut body = self.label_body(&name, font, "l", None);
        let width = match entry.column {
            None => {
                if natural > edge && edge > box_.size {
                    body.properties = super::primitives::chart_wrapped_text_body();
                }
                py_min(natural, py_max(edge, box_.size))
            }
            Some(column) if font.width(&name) > column => {
                body.properties = super::primitives::chart_wrapped_text_body();
                column
            }
            Some(_) => py_min(natural, py_max(edge, box_.size)),
        };
        self.text(body, left, width, entry.baseline, &box_);
    }
}
