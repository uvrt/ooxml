//! Owns: a table's parts as docx-agent's `table_format` reads them for the reading tools
//! (`word_inspect`'s cells, `word_compare_blocks`): its own rows and a row's own cells,
//! seen through row- and cell-level wrappers but not into nested tables, each with its
//! place below the table; the grid's widths; a border, a shading's fill and a width as
//! values; Python's `{:g}`.
//!
//! Must not: change anything, or know a tool's reply.

use ooxml_word_model::docx::tables::placed_row_cells;
use ooxml_word_model::ids::W;
use ooxml_word_model::xmlread::py_int;
use ooxml_xml::{Element, Node};
use serde_json::{json, Map, Value};
use std::sync::Arc;

/// An element of a table, with its place below the table.
#[derive(Debug, Clone)]
pub(crate) struct Placed {
    /// The element.
    pub(crate) element: Arc<Element>,
    /// Its place: child indexes from the table.
    pub(crate) path: Vec<u32>,
}

fn step(k: usize) -> u32 {
    u32::try_from(k).unwrap_or(u32::MAX)
}

fn is_w(element: &Element, local: &str) -> bool {
    element.name().is(Some(W), local)
}

/// The descendants of `element` (at `path`) named `local` whose nearest ancestor named
/// `owner` is `element` itself: neither is looked into once found.
fn own(element: &Element, path: &mut Vec<u32>, local: &str, owner: &str, out: &mut Vec<Placed>) {
    for (k, node) in element.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        path.push(step(k));
        if is_w(child, local) {
            out.push(Placed {
                element: Arc::clone(child),
                path: path.clone(),
            });
        } else if !is_w(child, owner) {
            own(child, path, local, owner, out);
        }
        path.pop();
    }
}

/// The table's own rows (docx-agent's `own_rows`): not a nested table's.
pub(crate) fn own_rows(table: &Element) -> Vec<Placed> {
    let mut out = Vec::new();
    own(table, &mut Vec::new(), "tr", "tbl", &mut out);
    out
}

/// A row's own cells (docx-agent's `_cells`), each with its first grid column (after
/// `w:gridBefore`) and span, by the table grid's rules (`ooxml_word_model`'s `Grid`); their places below the
/// table the row's place is.
pub(crate) fn row_cells(row: &Placed) -> Vec<(Placed, usize, usize)> {
    placed_row_cells(&row.element)
        .into_iter()
        .map(|(element, place, cell)| {
            let path = [&row.path[..], &place[..]].concat();
            (Placed { element, path }, cell.first, cell.span)
        })
        .collect()
}

/// A `w:val` read as Python's `int()` reads it.
/// The grid's column widths, twips (docx-agent's `grid_widths`).
pub(crate) fn grid_widths(table: &Element) -> Vec<i64> {
    table
        .find(Some(W), "tblGrid")
        .map(|grid| {
            grid.child_elements()
                .filter(|c| is_w(c, "gridCol"))
                .map(|c| c.attribute(Some(W), "w").and_then(py_int).unwrap_or(0))
                .collect()
        })
        .unwrap_or_default()
}

/// A shading's fill as compared (`RRGGBB`, upper case; `None` for none or `auto`).
pub(crate) fn fill(node: Option<&Element>) -> Option<String> {
    let fill = node?.attribute(Some(W), "fill")?;
    (fill != "auto").then(|| fill.to_uppercase())
}

fn attribute_value(element: &Element, local: &str) -> Value {
    element.attribute(Some(W), local).map_or(Value::Null, Value::from)
}

/// Twips as points (`None` where not an integer).
pub(crate) fn points(value: Option<&str>) -> Value {
    value
        .and_then(|v| v.trim().parse::<i64>().ok())
        .and_then(|v| serde_json::Number::from_f64(v as f64 / 20.0))
        .map_or(Value::Null, Value::Number)
}

/// A border as docx-agent reads it (`read_border`): its style, size (points), colour, space
/// and theme colour, each where stated.
pub(crate) fn border(node: &Element) -> Value {
    let mut out = Map::new();
    out.insert("style".into(), attribute_value(node, "val"));
    let int = |local: &str| {
        node.attribute(Some(W), local)
            .map(|v| v.trim().parse::<i64>().unwrap_or(0))
    };
    if let Some(size) = int("sz") {
        out.insert("size".into(), json!(size as f64 / 8.0));
    }
    if node.attribute(Some(W), "color").is_some() {
        out.insert("color".into(), attribute_value(node, "color"));
    }
    if let Some(space) = int("space") {
        out.insert("space".into(), json!(space));
    }
    if node.attribute(Some(W), "themeColor").is_some() {
        out.insert("theme_color".into(), attribute_value(node, "themeColor"));
    }
    Value::Object(out)
}

/// A width as docx-agent reads it (`read_width`): `auto`, `n%` or points.
pub(crate) fn width(node: Option<&Element>) -> Value {
    let Some(node) = node else { return Value::Null };
    let kind = node
        .attribute(Some(W), "type")
        .filter(|k| !k.is_empty())
        .unwrap_or("dxa");
    let raw = node.attribute(Some(W), "w").filter(|w| !w.is_empty()).unwrap_or("0");
    match kind {
        "auto" => Value::from("auto"),
        "nil" => Value::Null,
        "pct" => {
            let text = raw.trim_end_matches('%');
            let value = if raw.ends_with('%') {
                text.parse::<f64>().ok()
            } else {
                text.parse::<i64>().ok().map(|v| v as f64 / 50.0)
            };
            value.map_or(Value::Null, |v| Value::from(format!("{}%", python_g(v))))
        }
        _ => points(Some(raw)),
    }
}

/// Python's `{:g}` for the values a width in percent takes.
pub(crate) fn python_g(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e16 {
        format!("{}", value as i64)
    } else {
        let text = format!("{value:.6}");
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}
