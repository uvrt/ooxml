//! Owns: a new chart's `c:chartSpace` as ooxml-edit's `chart_space` writes it -- its series
//! (`_series`: drawn as Office draws each kind's, measured), its plot and the line plot of a
//! combination chart, its axes (`_axes`), legend and chart area, and the ids Office picks at
//! random drawn from a hash of the part's name instead (`_ids`), so the same chart is the
//! same bytes.
//!
//! Must not: check the data (the caller has, [`NewChart::check`]).

use super::super::make::{Make, A, C};
use super::super::owned::Owned;
use super::super::series::Hash;
use super::parts::{
    dot_marker, end, fill, formula, grey_line, no_labels, no_line, no_line_shape, none_sp, num_ref, scheme,
    series_fill, sp, str_ref, text_properties, title_element, unique_id, val, LINE_WIDTH, SCATTER_LINE_WIDTH,
};
use super::{Labels, NewChart, NewKind, NewSeries, Plot};
use ooxml_xml::Element;
use std::fmt::Write as _;

/// The relationships namespace.
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// Markup compatibility.
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
/// Office 2010's chart namespace.
const C14: &str = "http://schemas.microsoft.com/office/drawing/2007/8/2/chart";
/// Office 2017's chart namespace.
const C16R3: &str = "http://schemas.microsoft.com/office/drawing/2017/03/chart";

/// The axis ids and the series' unique ids' shared tail (`_ids`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceIds {
    first: String,
    second: String,
    tail: String,
}

impl SpaceIds {
    /// The ids of a chart of `kind` with `count` series whose part is `seed`.
    fn of(seed: &str, kind: NewKind, count: usize, hash: Hash<'_>) -> SpaceIds {
        let digest = hash(format!("{seed}\n{}\n{count}", kind.name()).as_bytes());
        let hex = digest.iter().fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02X}");
            out
        });
        let piece = |from: usize, to: usize| hex.get(from..to).unwrap_or("").to_owned();
        let number = |from: usize, to: usize| u64::from_str_radix(&piece(from, to), 16).unwrap_or(0) + 1;
        SpaceIds {
            first: number(0, 7).to_string(),
            second: number(7, 14).to_string(),
            tail: format!(
                "{}-{}-{}-{}",
                piece(14, 18),
                piece(18, 22),
                piece(22, 26),
                piece(26, 38)
            ),
        }
    }
}

impl NewChart {
    /// The new chart's `c:chartSpace`; `seed` is its part's name, `workbook_rel` the id of
    /// its relationship to its workbook.
    #[must_use]
    pub fn chart_space(&self, seed: &str, workbook_rel: &str, hash: Hash<'_>) -> Element {
        let make = Make::fresh();
        let make = &make;
        let ids = SpaceIds::of(seed, self.kind, self.series.len(), hash);
        let lang = self.lang.as_str();
        let alternate = Owned::named(Some("mc"), "AlternateContent", Some(MC))
            .declaring("mc", MC)
            .with(
                Owned::named(Some("mc"), "Choice", Some(MC))
                    .declaring("c14", C14)
                    .with_attr("Requires", "c14")
                    .with(Owned::named(Some("c14"), "style", Some(C14)).with_attr("val", "102")),
            )
            .with(Owned::named(Some("mc"), "Fallback", Some(MC)).with(val(make, "style", "2")));
        let mut chart = make.c("chart");
        match &self.title {
            Some(title) => {
                chart.push(title_element(make, title, self.look.title, lang));
                chart.push(val(make, "autoTitleDeleted", "0"));
            }
            None => chart.push(val(make, "autoTitleDeleted", "1")),
        }
        chart.push(self.plot_area(make, &ids));
        if let Some(code) = self.legend_code() {
            chart.push(
                make.c("legend")
                    .with(val(make, "legendPos", code))
                    .with(val(make, "overlay", "0"))
                    .with(none_sp(make))
                    .with(text_properties(make, self.look.labels, "0", false, lang)),
            );
        }
        chart.push(val(make, "plotVisOnly", "1"));
        chart.push(val(make, "dispBlanksAs", "gap"));
        let options = Owned::named(Some("c16r3"), "dataDisplayOptions16", Some(C16R3))
            .with(Owned::named(Some("c16r3"), "dispNaAsBlank", Some(C16R3)).with_attr("val", "1"));
        let extension = make
            .c("ext")
            .declaring("c16r3", C16R3)
            .with_attr("uri", "{56B9EC1D-385E-4148-901F-78D8002777C0}")
            .with(options);
        chart.push(make.c("extLst").with(extension));
        let area = if self.look.framed {
            sp(
                make,
                vec![fill(make, scheme(make, "bg1", &[])), grey_line(make, 15_000)],
            )
        } else {
            none_sp(make)
        };
        let text = make.c("txPr").with(make.a("bodyPr")).with(make.a("lstStyle")).with(
            make.a("p")
                .with(make.a("pPr").with(make.a("defRPr")))
                .with(end(make, lang)),
        );
        make.c("chartSpace")
            .declaring("c", C)
            .declaring("a", A)
            .declaring("r", R)
            .with(val(make, "date1904", "0"))
            .with(val(make, "lang", if lang.is_empty() { "en-US" } else { lang }))
            .with(val(make, "roundedCorners", "0"))
            .with(alternate)
            .with(chart)
            .with(area)
            .with(text)
            .with(
                make.c("externalData")
                    .with_ns_attr(("r", R), "id", workbook_rel)
                    .with(val(make, "autoUpdate", "0")),
            )
            .close()
    }

    /// `c:plotArea`: the plot, a combination chart's line plot, the axes.
    fn plot_area(&self, make: &Make, ids: &SpaceIds) -> Owned {
        let (plot_kind, direction, grouping) = self.kind.plot();
        let grouping = grouping.unwrap_or("");
        let combo = self.combo();
        let mut plot = match plot_kind {
            Plot::Bar => make
                .c("barChart")
                .with(val(make, "barDir", direction.unwrap_or("col")))
                .with(val(make, "grouping", grouping)),
            Plot::Line => make.c("lineChart").with(val(make, "grouping", grouping)),
            Plot::Pie => make.c("pieChart"),
            Plot::Radar => make.c("radarChart").with(val(make, "radarStyle", grouping)),
            Plot::Bubble => make.c("bubbleChart"),
            Plot::Scatter => make.c("scatterChart").with(val(make, "scatterStyle", "lineMarker")),
        };
        plot.push(val(make, "varyColors", if plot_kind == Plot::Pie { "1" } else { "0" }));
        let mut lines = make
            .c("lineChart")
            .with(val(make, "grouping", "standard"))
            .with(val(make, "varyColors", "0"));
        for (index, (series, columns)) in self.series.iter().zip(self.data_columns()).enumerate() {
            let made = self.series_element(make, index, series, columns, &ids.tail);
            if series.line == Some(true) {
                lines.push(made);
            } else {
                plot.push(made);
            }
        }
        plot.push(no_labels(make, plot_kind == Plot::Pie));
        match plot_kind {
            Plot::Bar => {
                let stacked = grouping == "stacked";
                let column = direction == Some("col");
                let gap = if stacked || combo {
                    "150"
                } else if column {
                    "219"
                } else {
                    "182"
                };
                plot.push(val(make, "gapWidth", gap));
                if stacked {
                    plot.push(val(make, "overlap", "100"));
                } else if column && !combo {
                    plot.push(val(make, "overlap", "-27"));
                }
            }
            Plot::Bubble => {
                plot.push(val(make, "bubbleScale", "100"));
                plot.push(val(make, "showNegBubbles", "0"));
            }
            Plot::Line => {
                if self.kind == NewKind::Dot {
                    plot.push(val(make, "marker", "1"));
                }
                plot.push(val(make, "smooth", "0"));
            }
            Plot::Pie => plot.push(val(make, "firstSliceAng", "0")),
            Plot::Scatter | Plot::Radar => {}
        }
        if plot_kind != Plot::Pie {
            plot.push(val(make, "axId", &ids.first));
            plot.push(val(make, "axId", &ids.second));
        }
        let mut area = make.c("plotArea").with(make.c("layout")).with(plot);
        if combo {
            lines.push(no_labels(make, false));
            lines.push(val(make, "marker", "1"));
            lines.push(val(make, "smooth", "0"));
            lines.push(val(make, "axId", &ids.first));
            lines.push(val(make, "axId", &ids.second));
            area.push(lines);
        }
        if plot_kind != Plot::Pie {
            for axis in self.axes(make, ids) {
                area.push(axis);
            }
        }
        area.with(none_sp(make))
    }

    /// `_series`: series `index`, its values in column `columns.0` (a bubble chart's sizes
    /// in `columns.1`), drawn as Office draws it.
    fn series_element(
        &self,
        make: &Make,
        index: usize,
        series: &NewSeries,
        (column, sizes): (u32, Option<u32>),
        tail: &str,
    ) -> Owned {
        let (mut plot, _, style) = self.kind.plot();
        if series.line == Some(true) {
            plot = Plot::Line;
        }
        let count = self.labels.len();
        let mut ser =
            make.c("ser")
                .with(val(make, "idx", &index.to_string()))
                .with(val(make, "order", &index.to_string()));
        if let Some(name) = &series.name {
            ser.push(
                make.c("tx")
                    .with(str_ref(make, &formula(column, 1, None), std::slice::from_ref(name))),
            );
        }
        for look in self.series_look(make, index, plot, style) {
            ser.push(look);
        }
        let last = Some(count + 1);
        let values = num_ref(make, &formula(column, 2, last), &series.values, self.format_code());
        match &self.labels {
            Labels::Numbers(xs) if matches!(plot, Plot::Scatter | Plot::Bubble) => {
                let xs: Vec<Option<f64>> = xs.iter().copied().map(Some).collect();
                ser.push(make.c("xVal").with(num_ref(make, &formula(1, 2, last), &xs, "General")));
                ser.push(make.c("yVal").with(values));
            }
            Labels::Numbers(xs) => {
                let texts: Vec<String> = xs.iter().map(|x| super::super::number_text(*x)).collect();
                ser.push(make.c("cat").with(str_ref(make, &formula(1, 2, last), &texts)));
                ser.push(make.c("val").with(values));
            }
            Labels::Text(texts) => {
                ser.push(make.c("cat").with(str_ref(make, &formula(1, 2, last), texts)));
                ser.push(make.c("val").with(values));
            }
        }
        if plot == Plot::Bubble {
            let sizes = sizes.unwrap_or(column + 1);
            let given = series.size.clone().unwrap_or_default();
            ser.push(
                make.c("bubbleSize")
                    .with(num_ref(make, &formula(sizes, 2, last), &given, "General")),
            );
            ser.push(val(make, "bubble3D", "0"));
        }
        if matches!(plot, Plot::Line | Plot::Scatter) {
            ser.push(val(make, "smooth", "0"));
        }
        ser.with(unique_id(make, index, tail))
    }

    /// How series `index` is drawn as `plot` (of `style`): Office's shape, marker or
    /// points for each kind (measured).
    fn series_look(&self, make: &Make, index: usize, plot: Plot, style: Option<&str>) -> Vec<Owned> {
        let mut out = Vec::new();
        let count = self.labels.len();
        match plot {
            Plot::Bar => {
                out.push(sp(make, vec![series_fill(make, index), no_line(make)]));
                out.push(val(make, "invertIfNegative", "0"));
            }
            Plot::Radar if style == Some("filled") => out.push(sp(make, vec![series_fill(make, index), no_line(make)])),
            _ if self.kind == NewKind::Dot => {
                out.push(no_line_shape(make, LINE_WIDTH));
                out.push(dot_marker(make, index));
            }
            Plot::Line | Plot::Radar => {
                let line = make
                    .a("ln")
                    .with_attr("w", LINE_WIDTH)
                    .with_attr("cap", "rnd")
                    .with(series_fill(make, index))
                    .with(make.a("round"));
                out.push(sp(make, vec![line]));
                out.push(make.c("marker").with(val(make, "symbol", "none")));
            }
            Plot::Scatter => {
                out.push(no_line_shape(make, SCATTER_LINE_WIDTH));
                out.push(dot_marker(make, index));
            }
            Plot::Bubble => {
                let colour = super::super::series::series_colour(make, index).with(val_a(make, "alpha", "75000"));
                out.push(sp(make, vec![fill(make, colour), no_line(make)]));
                out.push(val(make, "invertIfNegative", "0"));
            }
            Plot::Pie => {
                for point in 0..count {
                    let edge = make
                        .a("ln")
                        .with_attr("w", "19050")
                        .with(fill(make, scheme(make, "lt1", &[])));
                    out.push(
                        make.c("dPt")
                            .with(val(make, "idx", &point.to_string()))
                            .with(val(make, "bubble3D", "0"))
                            .with(sp(make, vec![series_fill(make, point), edge])),
                    );
                }
            }
        }
        out
    }

    /// `_axis_common`: an axis as Office writes a new chart's.
    fn axis(&self, make: &Make, spec: &AxisSpec<'_>) -> Owned {
        let orientation = if spec.reversed { "maxMin" } else { "minMax" };
        let mut axis = make
            .c(spec.tag)
            .with(val(make, "axId", spec.id))
            .with(make.c("scaling").with(val(make, "orientation", orientation)))
            .with(val(make, "delete", "0"))
            .with(val(make, "axPos", spec.position));
        if spec.gridlines && self.gridlines {
            axis.push(make.c("majorGridlines").with(sp(make, vec![grey_line(make, 15_000)])));
        }
        axis.push(
            make.c("numFmt")
                .with_attr("formatCode", spec.code)
                .with_attr("sourceLinked", "1"),
        );
        axis.push(val(make, "majorTickMark", "none"));
        axis.push(val(make, "minorTickMark", "none"));
        axis.push(val(make, "tickLblPos", "nextTo"));
        axis.push(match spec.line {
            Some(level) => sp(make, vec![make.a("noFill"), grey_line(make, level)]),
            None => none_sp(make),
        });
        axis.push(text_properties(make, self.look.labels, "-60000000", false, &self.lang));
        axis.push(val(make, "crossAx", spec.cross));
        if !self.combo() {
            axis.push(val(make, "crosses", "autoZero"));
        }
        axis
    }

    /// `_axes`: the category (or x) axis, then the value (or y) axis.
    fn axes(&self, make: &Make, ids: &SpaceIds) -> [Owned; 2] {
        let (plot, direction, _) = self.kind.plot();
        let (first, second) = (ids.first.as_str(), ids.second.as_str());
        let code = self.format_code();
        let spec = |tag, (id, cross), position, (gridlines, line), code, reversed| AxisSpec {
            tag,
            id,
            cross,
            position,
            gridlines,
            line,
            code,
            reversed,
        };
        let (category, mut value) = if matches!(plot, Plot::Scatter | Plot::Bubble) {
            let grey = (true, Some(25_000));
            let x = spec("valAx", (first, second), "b", grey, "General", self.reversed);
            let y = spec("valAx", (second, first), "l", grey, code, false);
            let between = || val(make, "crossBetween", "midCat");
            (self.axis(make, &x).with(between()), self.axis(make, &y).with(between()))
        } else {
            let (across, up) = if direction == Some("bar") {
                ("l", "b")
            } else {
                ("b", "l")
            };
            let labels = spec(
                "catAx",
                (first, second),
                across,
                (false, Some(15_000)),
                "General",
                self.reversed,
            );
            let mut category = self.axis(make, &labels);
            for (flag, setting) in [
                ("auto", "1"),
                ("lblAlgn", "ctr"),
                ("lblOffset", "100"),
                ("noMultiLvlLbl", "0"),
            ] {
                category.push(val(make, flag, setting));
            }
            let values = spec("valAx", (second, first), up, (true, None), code, false);
            (
                category,
                self.axis(make, &values).with(val(make, "crossBetween", "between")),
            )
        };
        if self.reversed {
            // as "Categories in reverse order", the value axis crossing at the maximum
            match value.place("crosses") {
                Some(place) => {
                    if let Some(crosses) = value.at(place) {
                        crosses.set_attr("val", "max");
                    }
                }
                None => {
                    let after = value.place("crossAx").map_or(0, |p| value.after(p));
                    value.insert(after, val(make, "crosses", "max"));
                }
            }
        }
        [category, value]
    }
}

/// One axis' settings (`_axis_common`'s arguments).
struct AxisSpec<'s> {
    tag: &'s str,
    id: &'s str,
    cross: &'s str,
    position: &'s str,
    gridlines: bool,
    line: Option<u32>,
    code: &'s str,
    reversed: bool,
}

/// A DrawingML element with its `val`.
fn val_a(make: &Make, local: &str, value: &str) -> Owned {
    make.a(local).with_attr("val", value)
}
