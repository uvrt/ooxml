//! Owns: reading a `c:chartSpace` into the chart model ([`crate::model`]), equal to
//! ooxml-common's `chart/read.py`: every fact the layout needs and none invented. Elements are
//! found by local name through `ooxml_schema::c`'s views; DrawingML inside a chart (`c:spPr`,
//! `c:txPr`, `c:rich`) through `ooxml_drawingml::read`. The data caches are read in
//! `cache`.
//!
//! Three rules the Python states and this keeps: a chart boolean present without `val` is
//! **true** (`flag`); categories come from any of four cache spellings; the cache is read,
//! never the formula.
//!
//! Must not: resolve a colour or font, lay anything out, or read the embedded workbook.

mod cache;

use crate::model::{
    Axis, Chart, ChartText, DataLabels, DataPoint, Legend, Lines, Marker, Plot, Series, UpDownBars, View3D,
    AXIS_ELEMENTS, CHART_GROUP_ELEMENTS,
};
use ooxml_drawingml::read::fill::{fill, line};
use ooxml_drawingml::read::text::text_body;
use ooxml_drawingml::source::{Fill, Outline};
use ooxml_schema::a;
use ooxml_schema::c;
use ooxml_xml::view::{attr, child, children, local};
use ooxml_xml::Element;

/// The relationships namespace (`r:id`).
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A `CT_Boolean`: present with no `val` is true; else `val` is `1`, `true` or `True`.
fn flag(node: Option<&Element>, default: bool) -> bool {
    match node {
        None => default,
        Some(node) => attr(node, "val").is_none_or(|v| matches!(v, "1" | "true" | "True")),
    }
}

/// As [`flag`], but absence stays `None`.
fn optional_flag(node: Option<&Element>) -> Option<bool> {
    node.map(|n| flag(Some(n), true))
}

/// An attribute read as `1`, `true` or `True`; `None` where absent.
fn bool_attr(node: Option<&Element>, name: &str) -> Option<bool> {
    node.and_then(|n| attr(n, name))
        .map(|v| matches!(v, "1" | "true" | "True"))
}

/// `val` as a string.
fn val(node: Option<c::Val<'_>>) -> Option<String> {
    node.and_then(c::Val::val).map(str::to_owned)
}

/// `val` as a finite number.
fn num(node: Option<c::Val<'_>>) -> Option<f64> {
    node.and_then(c::Val::num)
}

/// Python's `int()` of a finite float: toward zero, saturating.
pub(crate) fn int_of(value: f64) -> i64 {
    #[allow(clippy::cast_possible_truncation)]
    let int = value.trunc() as i64;
    int
}

/// `val` as an integer (`int(float(val))`).
fn int(node: Option<c::Val<'_>>) -> Option<i64> {
    num(node).map(int_of)
}

/// The element of a view.
fn el<'a>(node: Option<c::Val<'a>>) -> Option<&'a Element> {
    node.map(c::Val::element)
}

/// The fill of a `c:spPr`.
fn sp_fill(sp_pr: Option<c::Any<'_>>) -> Option<Fill> {
    fill(sp_pr.map(c::Any::element))
}

/// The outline of a `c:spPr`.
fn sp_outline(sp_pr: Option<c::Any<'_>>) -> Option<Outline> {
    line(sp_pr.and_then(|s| a::ShapeProperties::new(s.element()).line()))
}

/// A text body under a `c:txPr` or `c:rich`.
fn body(node: Option<c::Any<'_>>) -> Option<ooxml_drawingml::text::TextBody> {
    text_body(node.map(c::Any::element))
}

/// `c:chartSpace` into the model; `None` when there is no `c:chart`.
pub fn chart_space(root: &Element) -> Option<Chart> {
    let space = c::ChartSpace::new(root);
    let chart = space.chart()?;
    let plot_area = chart.plot_area();
    let members: Vec<&Element> = plot_area
        .map(|p| p.members().map(c::Any::element).collect())
        .unwrap_or_default();
    let title = chart.title();
    let plot_sp_pr = plot_area.and_then(c::PlotArea::shape_properties);
    Some(Chart {
        plots: members
            .iter()
            .filter(|e| CHART_GROUP_ELEMENTS.contains(&local(e)))
            .map(|e| plot(e))
            .collect(),
        axes: members
            .iter()
            .filter(|e| AXIS_ELEMENTS.contains(&local(e)))
            .map(|e| axis(e))
            .collect(),
        title: title.and_then(|t| text(Some(t.element()))),
        auto_title_deleted: flag(el(chart.auto_title_deleted()), false),
        title_stated: title.is_some(),
        auto_title_deleted_stated: chart.auto_title_deleted().is_some(),
        legend: chart.legend().map(legend),
        view_3d: chart.view_3d().map(view_3d),
        display_blanks_as: val(chart.display_blanks_as()),
        plot_visible_only: optional_flag(el(chart.plot_visible_only())),
        fill: sp_fill(space.shape_properties()),
        outline: sp_outline(space.shape_properties()),
        plot_area_fill: sp_fill(plot_sp_pr),
        plot_area_outline: sp_outline(plot_sp_pr),
        text_properties: body(space.text_properties()),
        round_corners: optional_flag(el(space.rounded_corners())),
        color_map_override: space
            .colour_map_override()
            .and_then(|o| colour_map_override(o.element())),
        external_data_rel_id: space
            .external_data()
            .and_then(|e| e.element().attribute(Some(NS_R), "id"))
            .map(str::to_owned),
        date1904: flag(el(space.date1904()), false),
        title_text_properties: title.and_then(|t| body(t.text_properties())),
    })
}

fn plot(node: &Element) -> Plot {
    let view = c::Plot::new(node);
    let mut series: Vec<Series> = view.series().enumerate().map(|(i, s)| self::series(s, i)).collect();
    // A stable sort, as Python's `sorted` is.
    series.sort_by_key(|s| s.order);
    let bands = view.band_formats();
    Plot {
        kind: local(node).to_owned(),
        series,
        axis_ids: view
            .axis_ids()
            .filter_map(c::Val::val)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .collect(),
        grouping: val(view.grouping()),
        vary_colors: optional_flag(el(view.vary_colours())),
        bar_direction: val(view.bar_direction()),
        gap_width: num(view.gap_width()),
        gap_depth: num(view.gap_depth()),
        overlap: num(view.overlap()),
        shape: val(view.shape()),
        hole_size: num(view.hole_size()),
        first_slice_angle: num(view.first_slice_angle()),
        scatter_style: val(view.scatter_style()),
        radar_style: val(view.radar_style()),
        marker: optional_flag(el(view.marker())),
        data_labels: view.data_labels().map(|d| data_labels(d.element())),
        bubble_scale: num(view.bubble_scale()),
        size_represents: val(view.size_represents()),
        show_negative_bubbles: optional_flag(el(view.show_negative_bubbles())),
        bubble_3d: optional_flag(el(view.bubble_3d())),
        of_pie_type: val(view.of_pie_type()),
        split_type: val(view.split_type()),
        split_position: num(view.split_position()),
        custom_split: view
            .custom_split()
            .map(|s| {
                s.points()
                    .filter_map(c::Val::num)
                    .map(int_of)
                    .filter(|i| *i >= 0)
                    .collect()
            })
            .unwrap_or_default(),
        second_pie_size: num(view.second_pie_size()),
        series_lines: view.series_lines().map(chart_lines),
        hi_low_lines: view.hi_low_lines().map(chart_lines),
        up_down_bars: view.up_down_bars().map(up_down_bars),
        wireframe: optional_flag(el(view.wireframe())),
        band_fills: band_formats(bands, |s| sp_fill(Some(s))),
        band_outlines: band_formats(bands, |s| sp_outline(Some(s))),
    }
}

/// `c:bandFmts` read through `read`, keyed by each `c:bandFmt`'s `c:idx`: a later band of
/// the same index replaces the earlier in place (a Python dict).
fn band_formats<T>(node: Option<c::BandFormats<'_>>, read: impl Fn(c::Any<'_>) -> Option<T>) -> Vec<(i64, T)> {
    let mut out: Vec<(i64, T)> = Vec::new();
    for band in node.into_iter().flat_map(c::BandFormats::formats) {
        let index = int(band.idx());
        let value = band.shape_properties().and_then(&read);
        if let (Some(index), Some(value)) = (index, value) {
            if index >= 0 {
                insert(&mut out, index, value);
            }
        }
    }
    out
}

/// A dict assignment: replace in place, else append.
fn insert<T>(map: &mut Vec<(i64, T)>, key: i64, value: T) {
    if let Some(slot) = map.iter_mut().find(|(k, _)| *k == key) {
        slot.1 = value;
    } else {
        map.push((key, value));
    }
}

fn chart_lines(node: c::Formatted<'_>) -> Lines {
    Lines {
        outline: sp_outline(node.shape_properties()),
    }
}

fn up_down_bars(node: c::UpDownBars<'_>) -> UpDownBars {
    let up = node.up();
    let down = node.down();
    UpDownBars {
        gap_width: num(node.gap_width()),
        up_fill: up.and_then(|u| sp_fill(u.shape_properties())),
        up_outline: up.and_then(|u| sp_outline(u.shape_properties())),
        down_fill: down.and_then(|d| sp_fill(d.shape_properties())),
        down_outline: down.and_then(|d| sp_outline(d.shape_properties())),
    }
}

/// A data source's element.
fn source<'a>(d: Option<c::DataSource<'a>>) -> Option<&'a Element> {
    d.map(c::DataSource::element)
}

fn series(ser: c::Series<'_>, fallback: usize) -> Series {
    let sp_pr = ser.shape_properties();
    let fallback = i64::try_from(fallback).unwrap_or(i64::MAX);
    let (mut values, mut format_code) = cache::numeric_points(source(ser.values()));
    let (y_values, y_format) = cache::numeric_points(source(ser.y_values()));
    if !y_values.is_empty() {
        values = y_values;
        format_code = y_format;
    }
    let (x_values, _) = cache::numeric_points(source(ser.x_values()));
    let (mut categories, mut category_format) = cache::string_points(source(ser.categories()));
    let (category_values, category_point_codes) = cache::numeric_categories(source(ser.categories()));
    if categories.is_empty() && !x_values.is_empty() {
        (categories, category_format) = cache::string_points(source(ser.x_values()));
    }
    let (bubble_sizes, _) = cache::numeric_points(source(ser.bubble_sizes()));
    let length = values.len();
    if categories.len() < length {
        categories.resize(length, String::new());
    }
    Series {
        index: int(ser.idx()).unwrap_or(fallback),
        order: int(ser.order()).unwrap_or(fallback),
        name: ser.name().and_then(|t| text(Some(t.element()))),
        categories,
        values,
        x_values: (!x_values.is_empty()).then_some(x_values),
        bubble_sizes: (!bubble_sizes.is_empty()).then_some(bubble_sizes),
        format_code,
        category_format_code: category_format,
        category_values,
        category_point_format_codes: category_point_codes,
        fill: sp_fill(sp_pr),
        outline: sp_outline(sp_pr),
        invert_if_negative: optional_flag(el(ser.invert_if_negative())),
        smooth: optional_flag(el(ser.smooth())),
        marker: ser.marker().map(marker),
        explosion: num(ser.explosion()),
        data_points: ser.data_points().filter_map(data_point).collect(),
        data_labels: ser.data_labels().map(|d| data_labels(d.element())),
        label_range: cache::label_range(extension(Some(ser.element()), "datalabelsRange")),
    }
}

/// The first `c:extLst/c:ext` child of `node` named `name` (any namespace).
fn extension<'a>(node: Option<&'a Element>, name: &str) -> Option<&'a Element> {
    let list = node.and_then(|n| child(n, "extLst"))?;
    children(list, "ext").find_map(|ext| child(ext, name))
}

fn data_point(point: c::DataPoint<'_>) -> Option<DataPoint> {
    let index = int(point.index())?;
    if index < 0 {
        return None;
    }
    let sp_pr = point.shape_properties();
    Some(DataPoint {
        index,
        fill: sp_fill(sp_pr),
        outline: sp_outline(sp_pr),
        invert_if_negative: optional_flag(el(point.invert_if_negative())),
        explosion: num(point.explosion()),
    })
}

fn marker(node: c::Marker<'_>) -> Marker {
    let sp_pr = node.shape_properties();
    Marker {
        symbol: val(node.symbol()),
        size: num(node.size()),
        fill: sp_fill(sp_pr),
        outline: sp_outline(sp_pr),
    }
}

/// `c:dLbls` or `c:dLbl` (read alike, as the Python reads them).
fn data_labels(node: &Element) -> DataLabels {
    let view = c::DataLabels::new(node);
    let num_fmt = view.number_format();
    let sp_pr = view.shape_properties();
    let mut labels = DataLabels {
        delete: optional_flag(el(view.delete())),
        show_value: optional_flag(el(view.show_value())),
        show_category_name: optional_flag(el(view.show_category_name())),
        show_series_name: optional_flag(el(view.show_series_name())),
        show_percent: optional_flag(el(view.show_percent())),
        show_bubble_size: optional_flag(el(view.show_bubble_size())),
        show_legend_key: optional_flag(el(view.show_legend_key())),
        show_leader_lines: optional_flag(el(view.show_leader_lines())),
        position: val(view.position()),
        number_format: num_fmt.and_then(|n| num_fmt_code(n.element())),
        number_format_source_linked: bool_attr(num_fmt.map(c::NumberFormat::element), "sourceLinked"),
        text_properties: body(view.text_properties()),
        fill: sp_fill(sp_pr),
        outline: sp_outline(sp_pr),
        overrides: Vec::new(),
        text: text(child(node, "tx")),
        show_range: optional_flag(extension(Some(node), "showDataLabelsRange")),
    };
    for label in children(node, "dLbl") {
        if let Some(index) = int(c::DataLabel::new(label).idx()) {
            if index >= 0 {
                let read = data_labels(label);
                insert(&mut labels.overrides, index, read);
            }
        }
    }
    labels
}

/// A `c:numFmt`'s code: a namespaced `formatcode2` attribute where Office wrote one, else
/// `formatCode`.
fn num_fmt_code(node: &Element) -> Option<String> {
    let written = node
        .attributes()
        .iter()
        .find(|a| a.name.namespace().is_some() && a.name.local() == "formatcode2" && !a.value.is_empty());
    if let Some(found) = written {
        return Some(found.value.to_string());
    }
    attr(node, "formatCode").map(str::to_owned)
}

/// A `CT_TickMark`: its `val`, `cross` when it has none (or an empty one), `None` absent.
fn tick_mark(node: Option<c::Val<'_>>) -> Option<String> {
    node.map(|n| n.val().filter(|v| !v.is_empty()).unwrap_or("cross").to_owned())
}

fn axis(node: &Element) -> Axis {
    let view = c::Axis::new(node);
    let scaling = view.scaling();
    let num_fmt = view.number_format();
    let major = view.major_gridlines();
    let minor = view.minor_gridlines();
    let title = view.title();
    Axis {
        kind: local(node).to_owned(),
        axis_id: val(view.id()),
        position: val(view.position()),
        delete: flag(el(view.delete()), false),
        orientation: scaling
            .and_then(|s| val(s.orientation()))
            .filter(|o| !o.is_empty())
            .unwrap_or_else(|| "minMax".into()),
        minimum: scaling.and_then(|s| num(s.min())),
        maximum: scaling.and_then(|s| num(s.max())),
        major_unit: num(view.major_unit()),
        minor_unit: num(view.minor_unit()),
        log_base: scaling.and_then(|s| num(s.log_base())),
        number_format: num_fmt.and_then(|n| num_fmt_code(n.element())),
        number_format_source_linked: bool_attr(num_fmt.map(c::NumberFormat::element), "sourceLinked"),
        major_tick_mark: tick_mark(view.major_tick_mark()),
        minor_tick_mark: tick_mark(view.minor_tick_mark()),
        tick_label_position: val(view.tick_label_position()),
        major_gridlines: major.is_some(),
        major_gridline_outline: major.and_then(|g| sp_outline(g.shape_properties())),
        minor_gridlines: minor.is_some(),
        minor_gridline_outline: minor.and_then(|g| sp_outline(g.shape_properties())),
        cross_axis_id: val(view.cross_axis()),
        crosses: val(view.crosses()),
        crosses_at: num(view.crosses_at()),
        cross_between: val(view.cross_between()),
        label_offset: num(view.label_offset()),
        label_align: val(view.label_align()),
        tick_label_skip: int(view.tick_label_skip()),
        tick_mark_skip: int(view.tick_mark_skip()),
        title: title.and_then(|t| text(Some(t.element()))),
        title_stated: title.is_some(),
        text_properties: body(view.text_properties()),
        outline: sp_outline(view.shape_properties()),
    }
}

fn legend(node: c::Legend<'_>) -> Legend {
    let sp_pr = node.shape_properties();
    let mut deleted: Vec<i64> = node
        .entries()
        .filter(|e| flag(el(e.delete()), false))
        .filter_map(|e| int(e.idx()))
        .collect();
    deleted.sort_unstable();
    deleted.dedup();
    Legend {
        position: val(node.position()),
        overlay: flag(el(node.overlay()), false),
        text_properties: body(node.text_properties()),
        fill: sp_fill(sp_pr),
        outline: sp_outline(sp_pr),
        deleted_entries: deleted,
    }
}

fn view_3d(node: c::View3D<'_>) -> View3D {
    View3D {
        rot_x: num(node.rot_x()),
        rot_y: num(node.rot_y()),
        depth_percent: num(node.depth_percent()),
        h_percent: num(node.h_percent()),
        right_angle_axes: optional_flag(el(node.right_angle())),
        perspective: num(node.perspective()),
    }
}

/// `c:clrMapOvr`: `a:overrideClrMapping`'s attributes by local name, in order (a later one of
/// the same name replaces the earlier in place); `None` for `a:masterClrMapping` or none.
fn colour_map_override(node: &Element) -> Option<Vec<(String, String)>> {
    let mapping = child(node, "overrideClrMapping")?;
    let mut out: Vec<(String, String)> = Vec::new();
    for attribute in mapping.attributes() {
        let key = attribute.name.local().to_owned();
        let value = attribute.value.to_string();
        if let Some(slot) = out.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            out.push((key, value));
        }
    }
    (!out.is_empty()).then_some(out)
}

/// `c:tx` or `c:title`: rich DrawingML, a cached string, or both; `None` for neither.
fn text(node: Option<&Element>) -> Option<ChartText> {
    let mut node = node?;
    let mut rich = text_body(child(node, "rich"));
    if rich.is_none() {
        if let Some(tx) = child(node, "tx") {
            rich = text_body(child(tx, "rich"));
            node = tx;
        }
    }
    let cached = cache::cached_string(node);
    if rich.is_none() && cached.is_none() {
        return None;
    }
    Some(ChartText { rich, cached })
}
