//! Owns: a data source's points as JSON, as docx-agent and `edit_chart` give them: text,
//! integers and floats as written, `null` for a blank.
//!
//! Must not: read the chart (the points are read already).

use super::Point;
use serde_json::Value;

/// Points as JSON: text, integers and floats as written, `null` for a blank.
#[must_use]
pub fn points_json(values: &[Option<Point>]) -> Value {
    values.iter().map(Option::as_ref).map(point_json).collect()
}

/// A point as JSON (`null` for none).
#[must_use]
pub fn point_json(value: Option<&Point>) -> Value {
    match value {
        None => Value::Null,
        Some(Point::Text(t)) => Value::from(t.as_str()),
        Some(Point::Int(i)) => Value::from(*i),
        Some(Point::Float(f)) => Value::from(*f),
    }
}
