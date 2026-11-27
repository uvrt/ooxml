//! Owns: `edit_chart`'s `read` reply (ooxml-edit's `chart_read_json`, `charts/actions.py`):
//! what every `edit_chart` result holds (`chart_json`: id, type, title, categories, series,
//! legend, number formats, and where they apply the categories' date format, the gap width
//! and the blanks), then the axes' titles, the look (`chart_style_json`: types, scales, each
//! series' own format and point labels) and what the embedded workbook holds -- in that key
//! order, from `ooxml-chart`'s readings of the chart part.
//!
//! Must not: change the chart, or word what ooxml-edit words as its own exceptions (those
//! reads are [`Unreadable`], left to the host).

use crate::workbook::{workbook_values, Unreadable};
use ooxml_chart::data::{
    axis_names, axis_scale, blanks, chart_data, gap_width, is_dot, number_formats, point_labels, series_elements,
    series_look, ChartData, Look, Point, Said, Scale,
};
use ooxml_chart::data::{point_json, points_json};
use ooxml_opc::Package;
use ooxml_xml::Element;
use serde_json::{Map, Value};

/// What docx-agent calls the package in a reason (`edit/charts.py`'s `DOCUMENT`).
const DOCUMENT: &str = "document";

/// `read`'s data for the chart `id`, whose part `chart_part` has the tree `root`.
///
/// # Errors
/// [`Unreadable`] where ooxml-edit's read of the chart or its workbook fails.
pub(crate) fn chart_read_json(
    id: &str,
    package: &Package,
    chart_part: &str,
    root: &Element,
) -> Result<Value, Unreadable> {
    let unreadable = Unreadable("edit_chart read: a chart whose data the model cannot read");
    let data = chart_data(root).map_err(|_| unreadable)?;
    let mut out = chart_json(id, root, &data);
    if !out.contains_key("blanks") {
        out.insert("blanks".into(), Value::from(blanks(root)));
    }
    let axes: Map<String, Value> = data
        .axis_titles
        .iter()
        .map(|(axis, title)| ((*axis).to_owned(), point_json(title.as_ref())))
        .collect();
    out.insert("axes".into(), Value::Object(axes));
    style_json(root, &data, &mut out).map_err(|_| unreadable)?;
    out.insert("workbook".into(), workbook_values(package, chart_part, root, DOCUMENT)?);
    Ok(Value::Object(out))
}

/// ooxml-edit's `chart_json`, the chart by its `id`.
pub fn chart_json(id: &str, root: &Element, data: &ChartData) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("id".into(), Value::from(id));
    let kind = if is_dot(root) {
        Value::from("dot")
    } else {
        data.types.first().map_or(Value::Null, |t| Value::from(t.as_str()))
    };
    out.insert("type".into(), kind);
    out.insert("title".into(), point_json(data.title.as_ref()));
    out.insert("categories".into(), categories(data));
    let series: Vec<Value> = data
        .series
        .iter()
        .map(|s| {
            let mut one = Map::new();
            one.insert("name".into(), point_json(s.name.as_ref()));
            one.insert("values".into(), points_json(&s.values));
            Value::Object(one)
        })
        .collect();
    out.insert("series".into(), Value::Array(series));
    out.insert("legend".into(), Value::Bool(data.legend));
    let formats = number_formats(root, data.format.as_deref());
    let mut said = Map::new();
    for (key, value) in [
        ("values", formats.values),
        ("data_labels", formats.data_labels),
        ("value_axis", formats.value_axis),
    ] {
        said.insert(key.into(), value.map_or(Value::Null, Value::from));
    }
    out.insert("number_formats".into(), Value::Object(said));
    if data.category_dates.is_some() {
        out.insert(
            "category_format".into(),
            data.category_format.as_deref().map_or(Value::Null, Value::from),
        );
    }
    if let Some(width) = gap_width(root) {
        out.insert("gap_width".into(), Value::from(width));
    }
    let mut gaps = Map::new();
    for (index, series) in data.series.iter().enumerate() {
        let positions: Vec<usize> = series
            .values
            .iter()
            .enumerate()
            .filter_map(|(k, value)| value.is_none().then_some(k))
            .collect();
        if !positions.is_empty() {
            gaps.insert(
                series_key(series.name.as_ref(), index),
                Value::from(blank_text(&positions)),
            );
        }
    }
    if !gaps.is_empty() {
        out.insert("blanks".into(), Value::from(blanks(root)));
        out.insert("blank_points".into(), Value::Object(gaps));
    }
    out
}

/// The categories a result lists (`chart_categories`): ISO dates for dates kept as serials
/// (any other point as Python's `str`, a blank `"None"`), else the points as they are.
fn categories(data: &ChartData) -> Value {
    let Some(dates) = &data.category_dates else {
        return points_json(&data.categories);
    };
    dates
        .iter()
        .zip(&data.categories)
        .map(|(day, category)| match day {
            Some(day) => Value::from(day.as_str()),
            None => Value::from(category.as_ref().map_or_else(|| "None".to_owned(), Point::text)),
        })
        .collect()
}

/// A series as a key: its name as Python's `str`, else its position.
fn series_key(name: Option<&Point>, index: usize) -> String {
    name.map_or_else(|| index.to_string(), Point::text)
}

/// Where a series' blank points are (`tools/sources.py`'s `blank_text`, from 0):
/// `"1 blank point (category 2)"`, `"3 blank points (categories 4-5, 9)"`.
fn blank_text(positions: &[usize]) -> String {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for &position in positions {
        match runs.last_mut() {
            Some((_, last)) if *last + 1 == position => *last = position,
            _ => runs.push((position, position)),
        }
    }
    let shown: Vec<String> = runs
        .iter()
        .map(|&(first, last)| {
            if first == last {
                first.to_string()
            } else {
                format!("{first}-{last}")
            }
        })
        .collect();
    let count = positions.len();
    let (plural, one) = if count == 1 {
        ("", "category")
    } else {
        ("s", "categories")
    };
    format!("{count} blank point{plural} ({one} {})", shown.join(", "))
}

/// ooxml-edit's `chart_style_json`: `types`, `scales`, and where any series sets them,
/// `series_format` and `point_labels`.
pub(crate) fn style_json(
    root: &Element,
    data: &ChartData,
    out: &mut Map<String, Value>,
) -> Result<(), ooxml_chart::data::Unreadable> {
    out.insert(
        "types".into(),
        data.types.iter().map(|t| Value::from(t.as_str())).collect(),
    );
    let scales: Map<String, Value> = axis_names(root)
        .into_iter()
        .filter_map(|name| axis_scale(root, name).map(|scale| (name.to_owned(), scale_json(&scale))))
        .collect();
    out.insert("scales".into(), Value::Object(scales));
    let mut looks = Map::new();
    let mut labels = Map::new();
    for (index, ser) in series_elements(root).into_iter().enumerate() {
        let key = series_key(data.series.get(index).and_then(|s| s.name.as_ref()), index);
        let look = series_look(ser);
        if !look.is_empty() {
            let mut said = look_json(&look.own);
            if !look.points.is_empty() {
                let points: Map<String, Value> = look
                    .points
                    .iter()
                    .map(|(at, own)| (at.to_string(), Value::Object(look_json(own))))
                    .collect();
                said.insert("points".into(), Value::Object(points));
            }
            looks.insert(key.clone(), Value::Object(said));
        }
        let own = point_labels(ser)?;
        if own.iter().any(Option::is_some) {
            let own: Vec<Value> = own.into_iter().map(|l| l.map_or(Value::Null, Value::from)).collect();
            labels.insert(key, Value::Array(own));
        }
    }
    if !looks.is_empty() {
        out.insert("series_format".into(), Value::Object(looks));
    }
    if !labels.is_empty() {
        out.insert("point_labels".into(), Value::Object(labels));
    }
    Ok(())
}

/// An axis' scale (`axis_scale`): bounds and units but on a category axis, then its number
/// format, direction and gridlines.
fn scale_json(scale: &Scale) -> Value {
    let mut out = Map::new();
    if let Some(bounds) = &scale.bounds {
        for (key, bound) in ["minimum", "maximum", "major_unit", "minor_unit"]
            .into_iter()
            .zip(bounds)
        {
            out.insert(key.into(), point_json(bound.as_ref()));
        }
    }
    out.insert(
        "number_format".into(),
        scale.number_format.as_deref().map_or(Value::Null, Value::from),
    );
    out.insert("reverse".into(), Value::Bool(scale.reverse));
    out.insert("major_gridlines".into(), Value::Bool(scale.gridlines.0));
    out.insert("minor_gridlines".into(), Value::Bool(scale.gridlines.1));
    Value::Object(out)
}

/// A look's settings, in its order.
fn look_json(look: &Look) -> Map<String, Value> {
    look.iter()
        .map(|(key, said)| {
            let value = match said {
                Said::Null => Value::Null,
                Said::Yes => Value::Bool(true),
                Said::Text(text) => Value::from(text.as_str()),
                Said::Number(number) => point_json(Some(number)),
            };
            ((*key).to_owned(), value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::blank_text;

    #[test]
    fn blank_points_are_runs_from_zero() {
        assert_eq!(blank_text(&[2]), "1 blank point (category 2)");
        assert_eq!(blank_text(&[0, 1, 2, 5]), "4 blank points (categories 0-2, 5)");
    }
}
