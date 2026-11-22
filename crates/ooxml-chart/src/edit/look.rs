//! Owns: the chart actions that change how the chart part shows its data, as ooxml-edit's
//! `Chart` makes them (`charts/chart.py`): the legend shown at a position or taken away
//! (`set_legend`), the bar plots' gap width (`set_gap_width`), and a series' value labels
//! shown in Office's data-label style or hidden (`set_data_labels`, `_value_labels`).
//!
//! Must not: check the caller's arguments (the handler does, in ooxml-edit's words), or know
//! which application's look a size comes from (the caller gives it).

use super::make::Make;
use super::owned::{Order, Owned};
use super::series::SER_ORDER;
use super::title::CHART_ORDER;
use super::ChartTree;
use ooxml_xml::view::{attr, child, children, local, py_int};
use ooxml_xml::Element;

/// `c:barChart`'s.
const BAR_ORDER: Order = &[
    &["barDir"],
    &["grouping"],
    &["varyColors"],
    &["ser"],
    &["dLbls"],
    &["gapWidth"],
    &["overlap"],
    &["serLines"],
    &["axId"],
    &["extLst"],
];

/// The gap a bar plot without `c:gapWidth` draws (`gap_width`).
const DEFAULT_GAP: i64 = 150;

/// Whether a `c:dLbls` shows its points' values (`_labels_show_values`).
#[must_use]
pub fn labels_show_values(holder: Option<&Element>) -> bool {
    let Some(holder) = holder else { return false };
    let on = |e: &Element| matches!(attr(e, "val").unwrap_or("1"), "1" | "true");
    if child(holder, "delete").is_some_and(on) {
        return false;
    }
    child(holder, "showVal").is_some_and(on)
}

impl ChartTree {
    fn chart(&self) -> Option<&Element> {
        child(&self.root, "chart")
    }

    /// Whether the chart has a legend (`has_legend`).
    #[must_use]
    pub fn has_legend(&self) -> bool {
        self.chart().and_then(|c| child(c, "legend")).is_some()
    }

    /// Show the legend at `position` (`r`, `l`, `t`, `b`) or, with `None`, take it away
    /// (`set_legend`); nothing where it is shown or hidden already. Whether the part
    /// changed.
    pub fn set_legend(&mut self, position: Option<&str>) -> bool {
        if position.is_some() == self.has_legend() {
            return false;
        }
        let make = Make::of(&self.root);
        self.change_root(|root| {
            let Some(chart) = root.child("chart") else { return };
            match position {
                Some(position) => {
                    let legend = make
                        .c("legend")
                        .with(make.c("legendPos").with_attr("val", position))
                        .with(make.c("overlay").with_attr("val", "0"));
                    chart.insert_ordered(Some(CHART_ORDER), legend);
                }
                None => {
                    if let Some(at) = chart.place("legend") {
                        chart.remove(at);
                    }
                }
            }
        });
        true
    }

    /// The bar plots (`c:chart/c:plotArea/c:barChart`).
    fn bar_plots(&self) -> Vec<&Element> {
        self.chart()
            .and_then(|c| child(c, "plotArea"))
            .map(|area| children(area, "barChart").collect())
            .unwrap_or_default()
    }

    /// The first bar plot's gap width (`gap_width`): `None` for a chart with none.
    #[must_use]
    pub fn gap_width(&self) -> Option<i64> {
        let plot = *self.bar_plots().first()?;
        Some(match child(plot, "gapWidth") {
            None => DEFAULT_GAP,
            Some(node) => attr(node, "val").and_then(py_int).unwrap_or(DEFAULT_GAP),
        })
    }

    /// Give every bar plot the gap width `percent` (`set_gap_width`, past its checks).
    /// Whether the part changed.
    pub fn set_gap_width(&mut self, percent: i64) -> bool {
        if self.gap_width() == Some(percent) {
            return false;
        }
        let text = percent.to_string();
        self.change_root(|root| {
            let Some(area) = root.child("chart").and_then(|c| c.child("plotArea")) else {
                return;
            };
            for at in area.places("barChart") {
                if let Some(plot) = area.at(at) {
                    let c = std::sync::Arc::clone(plot.name());
                    plot.sub(&c, "gapWidth", Some(BAR_ORDER)).set_attr("val", &text);
                }
            }
        });
        true
    }

    /// Show the values of the series `chosen` as labels (`set_data_labels`) in Office's
    /// style at `size` (hundredths of a point; none: the theme's), in `number_format`
    /// (none: the values' own), their text in `lang`; or hide them, a series whose plot
    /// shows labels given its own deleted ones.
    pub fn set_data_labels(&mut self, show: bool, chosen: &[usize], labels: &LabelLook<'_>) {
        let make = Make::of(&self.root);
        for &series in chosen {
            let Some((path, _)) = self.series(series) else { continue };
            let plot = path.split_last().and_then(|(_, p)| super::path::at(&self.root, p));
            let pie = plot.is_some_and(|p| local(p) == "pieChart");
            let plot_shows = labels_show_values(plot.and_then(|p| child(p, "dLbls")));
            self.change_series(series, |ser| {
                if let Some(at) = ser.place("dLbls") {
                    ser.remove(at);
                }
                if show {
                    ser.insert_ordered(Some(SER_ORDER), value_labels(&make, labels, pie));
                } else if plot_shows {
                    let hidden = make.c("dLbls").with(make.c("delete").with_attr("val", "1"));
                    ser.insert_ordered(Some(SER_ORDER), hidden);
                }
            });
        }
    }
}

/// How shown labels look: their size, number format and language.
#[derive(Debug, Clone, Copy)]
pub struct LabelLook<'a> {
    /// Hundredths of a point (`look.data_labels`); `None`: the theme's.
    pub size: Option<u32>,
    /// An Excel format code; `None`: the values' own.
    pub number_format: Option<&'a str>,
    /// The text's language (`lang` on the paragraph's end).
    pub lang: Option<&'a str>,
}

/// `_value_labels`: a series' `c:dLbls` showing its values, in Office's data-label style.
fn value_labels(make: &Make, look: &LabelLook<'_>, pie: bool) -> Owned {
    let mut labels = make.c("dLbls");
    if let Some(format) = look.number_format {
        labels.push(
            make.c("numFmt")
                .with_attr("formatCode", format)
                .with_attr("sourceLinked", "0"),
        );
    }
    let shape = make
        .c("spPr")
        .with(make.a("noFill"))
        .with(make.a("ln").with(make.a("noFill")))
        .with(make.a("effectLst"));
    labels.push(shape);
    labels.push(text_properties(make, look.size, 75000, look.lang));
    for (flag, value) in [
        ("showLegendKey", "0"),
        ("showVal", "1"),
        ("showCatName", "0"),
        ("showSerName", "0"),
        ("showPercent", "0"),
        ("showBubbleSize", "0"),
    ] {
        labels.push(make.c(flag).with_attr("val", value));
    }
    if pie {
        labels.push(make.c("showLeaderLines").with_attr("val", "1"));
    }
    make.top(labels)
}

/// `text_properties` (rotation 0, not a title): `c:txPr` in Office's chart text style, the
/// text colour at `shade`; `sz` left out where `size` is none.
fn text_properties(make: &Make, size: Option<u32>, shade: u32, lang: Option<&str>) -> Owned {
    let body = make
        .a("bodyPr")
        .with_attr("rot", "0")
        .with_attr("spcFirstLastPara", "1")
        .with_attr("vertOverflow", "ellipsis")
        .with_attr("vert", "horz")
        .with_attr("wrap", "square")
        .with_attr("anchor", "ctr")
        .with_attr("anchorCtr", "1");
    let mut defaults = make.a("defRPr");
    if let Some(size) = size {
        defaults.set_attr("sz", &size.to_string());
    }
    for (name, value) in [
        ("b", "0"),
        ("i", "0"),
        ("u", "none"),
        ("strike", "noStrike"),
        ("kern", "1200"),
        ("baseline", "0"),
    ] {
        defaults.set_attr(name, value);
    }
    let colour = make
        .a("schemeClr")
        .with_attr("val", "tx1")
        .with(make.a("lumMod").with_attr("val", &shade.to_string()))
        .with(make.a("lumOff").with_attr("val", &(100_000 - shade).to_string()));
    let defaults = defaults
        .with(make.a("solidFill").with(colour))
        .with(make.a("latin").with_attr("typeface", "+mn-lt"))
        .with(make.a("ea").with_attr("typeface", "+mn-ea"))
        .with(make.a("cs").with_attr("typeface", "+mn-cs"));
    let mut end = make.a("endParaRPr");
    if let Some(lang) = lang.filter(|l| !l.is_empty()) {
        end.set_attr("lang", lang);
    }
    let paragraph = make.a("p").with(make.a("pPr").with(defaults)).with(end);
    make.c("txPr").with(body).with(make.a("lstStyle")).with(paragraph)
}
