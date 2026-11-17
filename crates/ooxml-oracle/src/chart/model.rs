//! Owns: the chart model as JSON in the form `python/chart_diff.py` writes ooxml-common's
//! `SourceChart` in (`drawingml_diff.py`'s dump: each dataclass led by its Python class name,
//! every field by its Python name, a dict as `{"{}": [[key, value]...]}`, a set as a sorted
//! list).
//!
//! Must not: be used by any library; it is a test instrument.

use crate::dml::model::{class, fill, num, onum, opt, ostr, outline};
use crate::dml::tree::text_body;
use ooxml_chart::model::{
    Axis, Chart, ChartText, DataLabels, DataPoint, Legend, Lines, Marker, Plot, Series, UpDownBars, View3D,
};
use serde_json::{json, Value as J};

fn onums(values: &[Option<f64>]) -> J {
    J::Array(values.iter().map(|v| onum(*v)).collect())
}

fn text(t: &ChartText) -> J {
    class(
        "SourceChartText",
        vec![
            ("rich", opt(t.rich.as_ref(), text_body)),
            ("cached", ostr(t.cached.as_ref())),
        ],
    )
}

fn point(p: &DataPoint) -> J {
    class(
        "SourceChartDataPoint",
        vec![
            ("index", json!(p.index)),
            ("fill", opt(p.fill.as_ref(), fill)),
            ("outline", opt(p.outline.as_ref(), outline)),
            ("invert_if_negative", json!(p.invert_if_negative)),
            ("explosion", onum(p.explosion)),
        ],
    )
}

fn labels(l: &DataLabels) -> J {
    let overrides: Vec<J> = l.overrides.iter().map(|(k, v)| json!([k, labels(v)])).collect();
    class(
        "SourceChartDataLabels",
        vec![
            ("delete", json!(l.delete)),
            ("show_value", json!(l.show_value)),
            ("show_category_name", json!(l.show_category_name)),
            ("show_series_name", json!(l.show_series_name)),
            ("show_percent", json!(l.show_percent)),
            ("show_bubble_size", json!(l.show_bubble_size)),
            ("show_legend_key", json!(l.show_legend_key)),
            ("show_leader_lines", json!(l.show_leader_lines)),
            ("position", ostr(l.position.as_ref())),
            ("number_format", ostr(l.number_format.as_ref())),
            ("number_format_source_linked", json!(l.number_format_source_linked)),
            ("text_properties", opt(l.text_properties.as_ref(), text_body)),
            ("fill", opt(l.fill.as_ref(), fill)),
            ("outline", opt(l.outline.as_ref(), outline)),
            ("overrides", json!({"{}": overrides})),
            ("text", opt(l.text.as_ref(), text)),
            ("show_range", json!(l.show_range)),
        ],
    )
}

fn marker(m: &Marker) -> J {
    class(
        "SourceChartMarker",
        vec![
            ("symbol", ostr(m.symbol.as_ref())),
            ("size", onum(m.size)),
            ("fill", opt(m.fill.as_ref(), fill)),
            ("outline", opt(m.outline.as_ref(), outline)),
        ],
    )
}

fn lines(l: &Lines) -> J {
    class("SourceChartLines", vec![("outline", opt(l.outline.as_ref(), outline))])
}

fn up_down(u: &UpDownBars) -> J {
    class(
        "SourceChartUpDownBars",
        vec![
            ("gap_width", onum(u.gap_width)),
            ("up_fill", opt(u.up_fill.as_ref(), fill)),
            ("up_outline", opt(u.up_outline.as_ref(), outline)),
            ("down_fill", opt(u.down_fill.as_ref(), fill)),
            ("down_outline", opt(u.down_outline.as_ref(), outline)),
        ],
    )
}

fn series(s: &Series) -> J {
    class(
        "SourceChartSeries",
        vec![
            ("index", json!(s.index)),
            ("order", json!(s.order)),
            ("name", opt(s.name.as_ref(), text)),
            ("categories", json!(s.categories)),
            ("values", onums(&s.values)),
            ("x_values", opt(s.x_values.as_ref(), |v| onums(v))),
            ("bubble_sizes", opt(s.bubble_sizes.as_ref(), |v| onums(v))),
            ("format_code", ostr(s.format_code.as_ref())),
            ("category_format_code", ostr(s.category_format_code.as_ref())),
            ("category_values", opt(s.category_values.as_ref(), |v| onums(v))),
            ("category_point_format_codes", json!(s.category_point_format_codes)),
            ("fill", opt(s.fill.as_ref(), fill)),
            ("outline", opt(s.outline.as_ref(), outline)),
            ("invert_if_negative", json!(s.invert_if_negative)),
            ("smooth", json!(s.smooth)),
            ("marker", opt(s.marker.as_ref(), marker)),
            ("explosion", onum(s.explosion)),
            ("data_points", J::Array(s.data_points.iter().map(point).collect())),
            ("data_labels", opt(s.data_labels.as_ref(), labels)),
            ("label_range", json!(s.label_range)),
        ],
    )
}

fn axis(a: &Axis) -> J {
    class(
        "SourceChartAxis",
        vec![
            ("kind", json!(a.kind)),
            ("axis_id", ostr(a.axis_id.as_ref())),
            ("position", ostr(a.position.as_ref())),
            ("delete", json!(a.delete)),
            ("orientation", json!(a.orientation)),
            ("minimum", onum(a.minimum)),
            ("maximum", onum(a.maximum)),
            ("major_unit", onum(a.major_unit)),
            ("minor_unit", onum(a.minor_unit)),
            ("log_base", onum(a.log_base)),
            ("number_format", ostr(a.number_format.as_ref())),
            ("number_format_source_linked", json!(a.number_format_source_linked)),
            ("major_tick_mark", ostr(a.major_tick_mark.as_ref())),
            ("minor_tick_mark", ostr(a.minor_tick_mark.as_ref())),
            ("tick_label_position", ostr(a.tick_label_position.as_ref())),
            ("major_gridlines", json!(a.major_gridlines)),
            (
                "major_gridline_outline",
                opt(a.major_gridline_outline.as_ref(), outline),
            ),
            ("minor_gridlines", json!(a.minor_gridlines)),
            (
                "minor_gridline_outline",
                opt(a.minor_gridline_outline.as_ref(), outline),
            ),
            ("cross_axis_id", ostr(a.cross_axis_id.as_ref())),
            ("crosses", ostr(a.crosses.as_ref())),
            ("crosses_at", onum(a.crosses_at)),
            ("cross_between", ostr(a.cross_between.as_ref())),
            ("label_offset", onum(a.label_offset)),
            ("label_align", ostr(a.label_align.as_ref())),
            ("tick_label_skip", json!(a.tick_label_skip)),
            ("tick_mark_skip", json!(a.tick_mark_skip)),
            ("title", opt(a.title.as_ref(), text)),
            ("title_stated", json!(a.title_stated)),
            ("text_properties", opt(a.text_properties.as_ref(), text_body)),
            ("outline", opt(a.outline.as_ref(), outline)),
        ],
    )
}

fn plot(p: &Plot) -> J {
    let bands = |items: Vec<J>| json!({"{}": items});
    class(
        "SourceChartPlot",
        vec![
            ("kind", json!(p.kind)),
            ("series", J::Array(p.series.iter().map(series).collect())),
            ("axis_ids", json!(p.axis_ids)),
            ("grouping", ostr(p.grouping.as_ref())),
            ("vary_colors", json!(p.vary_colors)),
            ("bar_direction", ostr(p.bar_direction.as_ref())),
            ("gap_width", onum(p.gap_width)),
            ("gap_depth", onum(p.gap_depth)),
            ("overlap", onum(p.overlap)),
            ("shape", ostr(p.shape.as_ref())),
            ("hole_size", onum(p.hole_size)),
            ("first_slice_angle", onum(p.first_slice_angle)),
            ("scatter_style", ostr(p.scatter_style.as_ref())),
            ("radar_style", ostr(p.radar_style.as_ref())),
            ("marker", json!(p.marker)),
            ("data_labels", opt(p.data_labels.as_ref(), labels)),
            ("bubble_scale", onum(p.bubble_scale)),
            ("size_represents", ostr(p.size_represents.as_ref())),
            ("show_negative_bubbles", json!(p.show_negative_bubbles)),
            ("bubble_3d", json!(p.bubble_3d)),
            ("of_pie_type", ostr(p.of_pie_type.as_ref())),
            ("split_type", ostr(p.split_type.as_ref())),
            ("split_position", onum(p.split_position)),
            ("custom_split", json!(p.custom_split)),
            ("second_pie_size", onum(p.second_pie_size)),
            ("series_lines", opt(p.series_lines.as_ref(), lines)),
            ("hi_low_lines", opt(p.hi_low_lines.as_ref(), lines)),
            ("up_down_bars", opt(p.up_down_bars.as_ref(), up_down)),
            ("wireframe", json!(p.wireframe)),
            (
                "band_fills",
                bands(p.band_fills.iter().map(|(k, v)| json!([k, fill(v)])).collect()),
            ),
            (
                "band_outlines",
                bands(p.band_outlines.iter().map(|(k, v)| json!([k, outline(v)])).collect()),
            ),
        ],
    )
}

fn view(v: &View3D) -> J {
    class(
        "SourceChartView3D",
        vec![
            ("rot_x", onum(v.rot_x)),
            ("rot_y", onum(v.rot_y)),
            ("depth_percent", onum(v.depth_percent)),
            ("h_percent", onum(v.h_percent)),
            ("right_angle_axes", json!(v.right_angle_axes)),
            ("perspective", onum(v.perspective)),
        ],
    )
}

fn legend(l: &Legend) -> J {
    class(
        "SourceChartLegend",
        vec![
            ("position", ostr(l.position.as_ref())),
            ("overlay", json!(l.overlay)),
            ("text_properties", opt(l.text_properties.as_ref(), text_body)),
            ("fill", opt(l.fill.as_ref(), fill)),
            ("outline", opt(l.outline.as_ref(), outline)),
            ("deleted_entries", json!(l.deleted_entries)),
        ],
    )
}

/// A chart as `chart_diff.py` dumps ooxml-common's `SourceChart`.
pub fn chart(c: &Chart) -> J {
    let map = c.color_map_override.as_ref().map(|m| {
        let pairs: Vec<J> = m.iter().map(|(k, v)| json!([k, v])).collect();
        class("SourceColorMap", vec![("mapping", json!({"{}": pairs}))])
    });
    class(
        "SourceChart",
        vec![
            ("plots", J::Array(c.plots.iter().map(plot).collect())),
            ("axes", J::Array(c.axes.iter().map(axis).collect())),
            ("title", opt(c.title.as_ref(), text)),
            ("auto_title_deleted", json!(c.auto_title_deleted)),
            ("title_stated", json!(c.title_stated)),
            ("auto_title_deleted_stated", json!(c.auto_title_deleted_stated)),
            ("legend", opt(c.legend.as_ref(), legend)),
            ("view_3d", opt(c.view_3d.as_ref(), view)),
            ("display_blanks_as", ostr(c.display_blanks_as.as_ref())),
            ("plot_visible_only", json!(c.plot_visible_only)),
            ("fill", opt(c.fill.as_ref(), fill)),
            ("outline", opt(c.outline.as_ref(), outline)),
            ("plot_area_fill", opt(c.plot_area_fill.as_ref(), fill)),
            ("plot_area_outline", opt(c.plot_area_outline.as_ref(), outline)),
            ("text_properties", opt(c.text_properties.as_ref(), text_body)),
            ("round_corners", json!(c.round_corners)),
            ("color_map_override", map.unwrap_or(J::Null)),
            ("external_data_rel_id", ostr(c.external_data_rel_id.as_ref())),
            ("date1904", json!(c.date1904)),
            (
                "title_text_properties",
                opt(c.title_text_properties.as_ref(), text_body),
            ),
        ],
    )
}

/// A number, as the dump writes one.
pub fn number(v: f64) -> J {
    num(v)
}
