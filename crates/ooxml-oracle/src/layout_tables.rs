//! Owns: the tables' part of the Word layout's differential -- which of docx2svg's tables
//! the comparison stops before ([`table_cut`]: what the floats' and notes' lane lays out,
//! and what docx2svg draws as a stand-in box), each table compared as measured
//! ([`table_record`] against [`normalised`]: its grid, rows' edges, heights and keeps, every cell's blocks with
//! their lines, pitches and spacing, its approximation), and the lines a page's table rows
//! draw ([`table_lines`]), compared with the page's other lines in drawing order.
//!
//! Must not: change what is compared to make it pass.

use ooxml_base::Q;
use ooxml_word_layout::flow::FlowItem;
use ooxml_word_layout::paginate::LaidPage;
use ooxml_word_layout::px::Px;
use ooxml_word_layout::tables::{Content, MeasuredTable};
use ooxml_word_layout::DocumentLayout;
use serde_json::{json, Value};

/// Why the comparison stops at a table docx2svg laid out: what docx2svg's stand-in boxes
/// lay out of it, or a floating table nested in one of its cells, if anything. A floating
/// table of the body is compared: its rows where it is positioned, the lines beside it.
pub fn table_cut(table: &Value) -> Option<&'static str> {
    let unmodelled = table.get("unmodelled").and_then(Value::as_array);
    if unmodelled.is_some_and(|u| !u.is_empty()) {
        return Some("a table holding a stand-in box or an unmodelled drawing");
    }
    let rows = table.get("rows").and_then(Value::as_array).into_iter().flatten();
    for row in rows {
        let cells = row.get("cells").and_then(Value::as_array).into_iter().flatten();
        for entry in cells.filter_map(Value::as_array).flatten() {
            match entry.get(1).and_then(Value::as_str) {
                Some("box") => return Some("a table holding a stand-in box"),
                Some("table") => {
                    let nested = entry.get(4);
                    if nested
                        .and_then(|t| t.get("floating"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        return Some("a floating table nested in a cell");
                    }
                    if let Some(why) = nested.and_then(table_cut) {
                        return Some(why);
                    }
                }
                _ => {
                    let more = entry.get(4);
                    let flag = |k: &str| more.and_then(|m| m.get(k));
                    if flag("notes").and_then(Value::as_bool).unwrap_or(false) {
                        return Some("notes, drawings or an approximation: a note's reference in a cell");
                    }
                    if flag("anchors").and_then(Value::as_i64).unwrap_or(0) > 0 {
                        return Some("notes, drawings or an approximation: a floating drawing in a cell");
                    }
                }
            }
        }
    }
    None
}

fn q(x: Q) -> String {
    if x.is_integer() {
        x.numer().to_string()
    } else {
        format!("{}/{}", x.numer(), x.denom())
    }
}

/// A table as docx2svg's record writes it, its blocks counted from `base` (a nested
/// table's from its outer table's first).
pub fn table_record(table: &MeasuredTable, base: usize) -> Value {
    let rows: Vec<Value> = table
        .rows
        .iter()
        .enumerate()
        .map(|(r, row)| {
            let cells: Vec<Value> = row
                .cells
                .iter()
                .map(|cell| Value::Array(cell.entries.iter().map(|e| entry_record(e, base)).collect()))
                .collect();
            json!({
                "top_border": row.top_border.to_string(), "height": row.height.to_string(),
                "keep": table.row_keeps(r), "cells": cells,
            })
        })
        .collect();
    json!({
        "approximation": table.approximation.is_some(),
        "grid": table.grid.iter().map(|x| q(*x)).collect::<Vec<_>>(),
        "rows": rows,
        "bottom_border": table.bottom_border.to_string(),
        "header_rows": table.header_rows,
    })
}

fn entry_record(entry: &ooxml_word_layout::tables::CellBlock, base: usize) -> Value {
    let block = base + entry.block;
    let pitches: Vec<String> = entry.pitches.iter().map(Px::to_string).collect();
    match &entry.content {
        Content::Table(nested) => json!([block, "table", null, pitches, table_record(nested, block)]),
        Content::Paragraph(m) => {
            let lines: Vec<Value> = m
                .measured
                .lines
                .iter()
                .map(|l| json!([l.start, l.end, l.forced]))
                .collect();
            json!([block, "para", lines, pitches, {
                "hidden": entry.hidden, "before": entry.before, "after": entry.after,
                "top_border": entry.top_border.to_string(), "border": entry.border.to_string(),
            }])
        }
    }
}

/// docx2svg's table record made comparable with [`table_record`]: its approximation as
/// whether there is one, its blocks relative to the table's first, and only the fields
/// the Rust record holds.
pub fn normalised(theirs: &Value, first: i64) -> Value {
    let entry = |e: &Value| -> Value {
        let block = e.get(0).and_then(Value::as_i64).unwrap_or(0) - first;
        let kind = e.get(1).cloned().unwrap_or(Value::Null);
        let more = match kind.as_str() {
            Some("table") => e.get(4).map_or(Value::Null, |t| normalised(t, first)),
            _ => {
                let m = e.get(4).cloned().unwrap_or(Value::Null);
                json!({
                    "hidden": m.get("hidden"), "before": m.get("before"), "after": m.get("after"),
                    "top_border": m.get("top_border"), "border": m.get("border"),
                })
            }
        };
        json!([block, kind, e.get(2), e.get(3), more])
    };
    let rows: Vec<Value> = theirs
        .get("rows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|row| {
            let cells: Vec<Value> = row
                .get("cells")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|c| Value::Array(c.as_array().into_iter().flatten().map(entry).collect()))
                .collect();
            json!({"top_border": row.get("top_border"), "height": row.get("height"), "keep": row.get("keep"),
                   "cells": cells})
        })
        .collect();
    json!({
        "approximation": theirs.get("approximation").is_some_and(|a| !a.is_null()),
        "grid": theirs.get("grid"),
        "rows": rows,
        "bottom_border": theirs.get("bottom_border"),
        "header_rows": theirs.get("header_rows"),
    })
}

/// The first difference between two records, as a path and the two values.
pub fn first_difference(path: &str, mine: &Value, theirs: &Value) -> Option<String> {
    match (mine, theirs) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in a {
                let other = b.get(k).unwrap_or(&Value::Null);
                if let Some(d) = first_difference(&format!("{path}.{k}"), v, other) {
                    return Some(d);
                }
            }
            None
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(k, (x, y))| first_difference(&format!("{path}[{k}]"), x, y)),
        _ if mine == theirs => None,
        _ => Some(format!("{path}: {mine} vs {theirs}")),
    }
}

/// The table a flow item lays out, where it is one of ours.
pub fn measured(item: &FlowItem) -> Option<&MeasuredTable> {
    let FlowItem::Table(t) = item else { return None };
    t.flow.as_any().downcast_ref::<MeasuredTable>()
}

/// A page's height in whole px, as docx2svg's page has it: `round(pt * 300 / 72)`, a half
/// to the even px.
pub fn page_height_px(height_twips: i64) -> Px {
    let exact = Q::new(i128::from(height_twips) * 5, 24);
    let floor = exact.floor();
    let rest = exact - floor;
    let half = Q::new(1, 2);
    let whole = if rest > half || (rest == half && (floor.to_integer() % 2 != 0)) {
        floor.to_integer() + 1
    } else {
        floor.to_integer()
    };
    Px::from(Q::from_integer(whole))
}

/// The lines a page's table rows draw, by flow item and in drawing order: each as its
/// paragraph's block (the body's, cells included), its line, top and pitch.
pub fn table_lines(layout: &DocumentLayout, page: &LaidPage) -> Vec<(usize, usize, usize, Px, Px)> {
    let height = layout
        .pagination
        .info
        .get(page.page)
        .and_then(|i| layout.sections.get(i.section))
        .map_or(Px::int(i64::MAX / 4), |s| page_height_px(s.props.page_size.height));
    let mut out = Vec::new();
    for placed in &page.rows {
        let Some(FlowItem::Table(t)) = layout.flow.items.get(placed.item) else {
            continue;
        };
        for line in placed.flow(t).lines(&placed.row) {
            if line.drawn && line.top < height {
                out.push((placed.item, t.block + line.block, line.line, line.top, line.pitch));
            }
        }
    }
    out
}
