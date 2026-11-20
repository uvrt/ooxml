//! Owns: what a Word drawing's chart holds, as docx-agent reads it for `word_read` and
//! `word_inspect` (`edit/charts.py`'s `chart_json` and `chart_summary`): the chart part its
//! frame's first `c:chart` names through the frame's part's relationships, read into
//! `ooxml_chart::data`, and that data as docx-agent's JSON.
//!
//! Must not: change a chart, or read its embedded workbook; a chart whose part is missing or
//! unreadable is no chart here (docx-agent's `None`), never an error.

use crate::docx::drawings::C;
use crate::docx::Docx;
use crate::w::NS_R;
use ooxml_chart::data::{chart_data, ChartData, Point};
use ooxml_xml::Element;
use serde_json::{Map, Value};

/// The chart `frame` (a `wp:inline` or `wp:anchor`, or a group member) of the part `part`
/// holds: `None` where it names none, or its part is missing or unreadable.
#[must_use]
pub fn chart_of(docx: &Docx, part: &str, frame: &Element) -> Option<ChartData> {
    let node = crate::parts::descendants(frame, C, "chart").into_iter().next()?;
    let id = node.attribute(Some(NS_R), "id")?;
    let relationships = crate::parts::relationships(&docx.package, part).ok()?;
    let target = relationships
        .into_iter()
        .find(|r| r.id == id && !r.external)?
        .target_part?;
    let tree = docx.package.tree(&target).ok()??;
    chart_data(tree.root()).ok()
}

/// A chart's JSON as docx-agent gives it (ooxml-edit's `chart_model`): `types`, `title`,
/// `axis_titles`, `legend`, `format`, `categories`, `series` (`name`, `values`), and
/// `category_dates` and `category_format` where the categories are dates kept as serials.
#[must_use]
pub fn chart_json(data: &ChartData) -> Value {
    let mut out = Map::new();
    out.insert(
        "types".into(),
        data.types.iter().map(|t| Value::from(t.as_str())).collect(),
    );
    out.insert("title".into(), point(data.title.as_ref()));
    let axes: Map<String, Value> = data
        .axis_titles
        .iter()
        .map(|(axis, title)| ((*axis).to_owned(), point(title.as_ref())))
        .collect();
    out.insert("axis_titles".into(), Value::Object(axes));
    out.insert("legend".into(), Value::Bool(data.legend));
    out.insert("format".into(), data.format.as_deref().map_or(Value::Null, Value::from));
    out.insert("categories".into(), points(&data.categories));
    let series = data
        .series
        .iter()
        .map(|s| {
            let mut one = Map::new();
            one.insert("name".into(), point(s.name.as_ref()));
            one.insert("values".into(), points(&s.values));
            Value::Object(one)
        })
        .collect();
    out.insert("series".into(), Value::Array(series));
    if let Some(dates) = &data.category_dates {
        let dates = dates
            .iter()
            .map(|d| d.as_deref().map_or(Value::Null, Value::from))
            .collect();
        out.insert("category_dates".into(), Value::Array(dates));
        out.insert(
            "category_format".into(),
            data.category_format.as_deref().map_or(Value::Null, Value::from),
        );
    }
    Value::Object(out)
}

fn points(values: &[Option<Point>]) -> Value {
    values.iter().map(Option::as_ref).map(point).collect()
}

fn point(value: Option<&Point>) -> Value {
    match value {
        None => Value::Null,
        Some(Point::Text(t)) => Value::from(t.as_str()),
        Some(Point::Int(i)) => Value::from(*i),
        Some(Point::Float(f)) => Value::from(*f),
    }
}
