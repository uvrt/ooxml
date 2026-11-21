//! Owns: a series' or one point's colours, lines and markers changed in the chart part
//! (ooxml-edit's `ChartStyling.format_series`, `format_point`, `_format`,
//! `charts/styling.py`): what each kind of series takes checked (a fill for areas, markers
//! for lines), the series' `c:spPr` -- or the point's `c:dPt`, made from its series' look
//! (`_data_point`) -- given its fill, line and an empty effect list, and its marker.
//!
//! Must not: resolve the series or point by name (the tool's handler does).

use super::format::{effects, points_to_emu, set_fill, set_line, set_marker, Colour, MarkerLook};
use super::make::Make;
use super::owned::{Order, Owned};
use super::points::reference;
use super::series::{POINT_SOURCES, SER_ORDER};
use super::{ChartTree, DataError};
use ooxml_xml::view::{attr, child, local};

/// `c:dPt`'s sequence.
const DPT_ORDER: Order = &[
    &["idx"],
    &["invertIfNegative"],
    &["marker"],
    &["bubble3D"],
    &["explosion"],
    &["spPr"],
    &["pictureOptions"],
    &["extLst"],
];
/// Plots whose series are areas (`_FILLED`), and lines (`_LINED`).
const FILLED: [&str; 9] = [
    "barChart",
    "bar3DChart",
    "pieChart",
    "pie3DChart",
    "doughnutChart",
    "ofPieChart",
    "areaChart",
    "area3DChart",
    "bubbleChart",
];
const LINED: [&str; 5] = ["lineChart", "line3DChart", "scatterChart", "radarChart", "stockChart"];

/// A series' or a point's look as `format_series` takes it: each `None` stays as it is; a
/// colour (a specification, [`Colour::parse`]) `Some(None)` is none.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SeriesFormat {
    /// A bar's, slice's, area's or bubble's colour.
    pub fill: Option<Option<String>>,
    /// A line series' line, the others' outline.
    pub line: Option<Option<String>>,
    /// The line's width in points.
    pub line_width: Option<ooxml_base::pyfmt::PyNumber>,
    /// The line's dash (`ST_PresetLineDashVal`).
    pub dash: Option<String>,
    /// The marker's symbol (`ST_MarkerStyle`).
    pub marker: Option<String>,
    /// The marker's size, 2-72.
    pub marker_size: Option<i64>,
    /// The marker's fill (and outline: Office outlines a marker in its fill colour).
    pub marker_fill: Option<Option<String>>,
}

impl SeriesFormat {
    fn markers(&self) -> bool {
        self.marker.is_some() || self.marker_size.is_some() || self.marker_fill.is_some()
    }
}

impl ChartTree {
    /// Give the series at `series` (or its point `point`, a position from 0) `look`
    /// (`_format`).
    ///
    /// # Errors
    /// [`DataError`] (`FormatError`, `IndexError`) for a fill on a line, markers on a series
    /// without, a line width past 0-1584 points or a point the series does not have.
    pub fn format_series(
        &mut self,
        address: &str,
        series: usize,
        point: Option<usize>,
        look: &SeriesFormat,
    ) -> Result<(), DataError> {
        let Some((path, _)) = self.series(series) else {
            return Err(DataError(format!("{address}: no series {series}")));
        };
        let plot = path
            .split_last()
            .and_then(|(_, p)| super::path::at(&self.root, p))
            .map_or_else(String::new, |p| local(p).to_owned());
        let filled = if plot == "radarChart" {
            let at = path.split_last().and_then(|(_, p)| super::path::at(&self.root, p));
            at.and_then(|p| child(p, "radarStyle")).and_then(|s| attr(s, "val")) == Some("filled")
        } else {
            FILLED.contains(&plot.as_str())
        };
        if look.fill.is_some() && !filled {
            let kind: String = plot.chars().take(plot.chars().count().saturating_sub(5)).collect();
            return Err(DataError(format!(
                "{address}: a {kind} series is a line; colour it with line, or its markers with marker_fill"
            )));
        }
        if look.markers() && (filled || !LINED.contains(&plot.as_str())) {
            return Err(DataError(format!(
                "{address}: only line, scatter and radar series have markers"
            )));
        }
        let colour = |given: &Option<Option<String>>| -> Result<Option<Option<Colour>>, DataError> {
            given
                .as_ref()
                .map(|spec| spec.as_deref().map(Colour::parse).transpose())
                .transpose()
        };
        let (fill, line, marker_fill) = (colour(&look.fill)?, colour(&look.line)?, colour(&look.marker_fill)?);
        let width = look.line_width.map(points_to_emu).transpose()?;
        if let Some(point) = point {
            let count = self.series_point_count(series);
            if point >= count {
                return Err(DataError(format!("{address}: no point {point} (have {count})")));
            }
        }
        let make = Make::of(&self.root);
        self.change_series(series, |ser| {
            let (owner, order) = match point {
                None => (ser, SER_ORDER),
                Some(point) => {
                    let at = data_point(ser, &make, &plot, point);
                    // found or made just now
                    (ser.at(at).unwrap_or_else(|| unreachable!("a c:dPt")), DPT_ORDER)
                }
            };
            let shape_at = match owner.place("spPr") {
                Some(at) => at,
                None => owner.insert_ordered(Some(order), make.c("spPr")),
            };
            if let Some(shape) = owner.at(shape_at) {
                if let Some(fill) = &fill {
                    set_fill(shape, &make, fill.as_ref());
                }
                if line.is_some() || width.is_some() || look.dash.is_some() {
                    let colour = line.as_ref().and_then(Option::as_ref);
                    set_line(shape, &make, colour, line.is_none(), width, look.dash.as_deref());
                }
                effects(shape, &make);
            }
            if look.markers() {
                let fill = marker_fill.as_ref().map(Option::as_ref);
                let marker = MarkerLook {
                    symbol: look.marker.as_deref(),
                    size: look.marker_size,
                    fill,
                    line: fill,
                };
                set_marker(owner, order, &make, &marker);
            }
        });
        Ok(())
    }

    /// The most points any source of the series at `series` holds (`_Series.count`).
    pub(crate) fn series_point_count(&self, series: usize) -> usize {
        let Some((_, ser)) = self.series(series) else { return 0 };
        let mut owned = Owned::open(ser);
        POINT_SOURCES
            .into_iter()
            .filter_map(|which| reference(&mut owned, which).map(|mut s| s.count()))
            .max()
            .unwrap_or(0)
    }
}

/// The series' `c:dPt` for point `index`, made from the series' look where there is none
/// (`_data_point`): its place.
fn data_point(ser: &mut Owned, make: &Make, plot: &str, index: usize) -> usize {
    let wanted = index.to_string();
    let points = ser.places("dPt");
    let idx_of = |ser: &Owned, at: usize| {
        ser.read(at)
            .and_then(|d| child(&d, "idx").and_then(|n| attr(n, "val")).map(str::to_owned))
    };
    if let Some(&at) = points
        .iter()
        .find(|&&at| idx_of(ser, at).as_deref() == Some(wanted.as_str()))
    {
        return at;
    }
    let mut point = make.c("dPt").with(make.c("idx").with_attr("val", &wanted));
    if matches!(plot, "barChart" | "bar3DChart" | "bubbleChart") {
        point.push(make.c("invertIfNegative").with_attr("val", "0"));
    }
    if LINED.contains(&plot) {
        if let Some(marker) = ser.place("marker").and_then(|at| ser.read(at)) {
            point.push(Owned::open(&marker));
        }
    }
    point.push(make.c("bubble3D").with_attr("val", "0"));
    if let Some(shape) = ser.place("spPr").and_then(|at| ser.read(at)) {
        point.push(Owned::open(&shape));
    }
    let later = points.iter().copied().find(|&at| {
        idx_of(ser, at)
            .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|v| v > index as u64)
    });
    match later {
        Some(at) => {
            ser.insert(at, point);
            at
        }
        None => ser.insert_ordered(Some(SER_ORDER), point),
    }
}
