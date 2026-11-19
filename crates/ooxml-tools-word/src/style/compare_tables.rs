//! Owns: two tables compared as docx-agent's `edit/compare.py` compares them, as they state
//! themselves: header rows, columns and rows, merges, the grid's widths and the table's,
//! style, alignment, shading and borders; each cell's shading, borders and vertical
//! alignment, its paragraphs paired with its reference cell's; rows corresponding by the
//! reference's structure, cells by grid column; with the `word_format_table` and
//! `word_edit_table` calls that give the reference's values.
//!
//! Must not: change a document, or compare paragraphs (`super::compare_paragraphs` does).

use super::compare::{difference, fix, Block, Difference, Groups, Para, Side};
use super::compare_paragraphs::{align, described, paragraphs};
use crate::table_read::{border, fill, grid_widths, own_rows, row_cells, width, Placed};
use ooxml_word_edit::pairing;
use ooxml_word_model::docx::find_path;
use ooxml_word_model::ids::W;
use ooxml_xml::Element;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Two widths, points, equal within this (the module's "Width tolerance").
const WIDTH_TOLERANCE: f64 = 0.5;

/// The border styles `word_format_table` writes, each at half a point in automatic colour.
const TOOL_BORDERS: [&str; 5] = ["single", "double", "thick", "dotted", "dashed"];
const CELL_SIDES: [&str; 4] = ["top", "left", "bottom", "right"];
const TABLE_SIDES: [&str; 6] = ["top", "left", "bottom", "right", "insideH", "insideV"];

/// A cell at its first grid column: the cell, its span, whether it is merged up.
#[derive(Clone)]
struct Origin {
    cell: Placed,
    span: usize,
    up: bool,
}

/// A table's rows, each by first grid column.
type Grid = Vec<BTreeMap<usize, Origin>>;

/// `block` compared with `source` (both tables).
pub(crate) fn tables(block: &Block<'_>, mine: &Side<'_>, source: &Block<'_>, theirs: &Side<'_>) -> Vec<Difference> {
    let mut out = Vec::new();
    let (rows, reference_rows) = (own_rows(&block.element), own_rows(&source.element));
    if rows.is_empty() || reference_rows.is_empty() {
        return out;
    }
    let table_fix = |values: Option<Map<String, Value>>| match values {
        Some(values) => {
            let mut call = Map::new();
            call.insert("table".into(), Value::from(block.id.as_str()));
            call.insert("scope".into(), Value::from("table"));
            call.extend(values);
            fix("word_format_table", vec![Value::Object(call)])
        }
        None => Value::Null,
    };
    let mut table = |name: &str, value: Value, reference: Value, values: Option<Map<String, Value>>| {
        if value != reference {
            out.push(difference(name, None, value, reference, table_fix(values)));
        }
    };
    let (header, reference_header) = (header_rows(&rows), header_rows(&reference_rows));
    table(
        "header_rows",
        json!(header),
        json!(reference_header),
        Some(one("header_rows", json!(reference_header))),
    );
    let widths = points_of(&grid_widths(&block.element));
    let reference_widths = points_of(&grid_widths(&source.element));
    table("columns", json!(widths.len()), json!(reference_widths.len()), None);
    let body = i64::try_from(rows.len()).unwrap_or(0) - i64::try_from(reference_header).unwrap_or(0);
    let reference_body = reference_rows.len().saturating_sub(reference_header);
    if body != i64::try_from(reference_body).unwrap_or(0) {
        out.push(difference(
            "rows",
            None,
            json!(body.max(0)),
            json!(reference_body),
            Value::Null,
        ));
    }
    let mapping = pairing::row_map(rows.len(), reference_rows.len(), reference_header);
    let (grid, reference_grid) = (origins(&rows), origins(&reference_rows));
    out.extend(table_merges(block, &grid, &reference_grid, &mapping));
    out.extend(width_differences(block, &widths, &reference_widths));
    out.extend(properties(block, mine, source, theirs));
    out.extend(cells(block, mine, source, theirs, (&grid, &reference_grid), &mapping));
    out
}

fn one(key: &str, value: Value) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert(key.into(), value);
    out
}

/// Twips as points, rounded as docx-agent rounds them (`round(w / 20, 2)`).
fn points_of(twips: &[i64]) -> Vec<f64> {
    twips.iter().map(|w| round2(*w as f64 / 20.0)).collect()
}

/// Python's `round(value, 2)`: the nearest of two decimals, ties to even on the exact value.
pub(crate) fn round2(value: f64) -> f64 {
    format!("{value:.2}").parse().unwrap_or(value)
}

fn header_rows(rows: &[Placed]) -> usize {
    rows.iter()
        .take_while(|row| {
            find_path(&row.element, &["trPr", "tblHeader"])
                .is_some_and(|node| pairing::is_header(node.attribute(Some(W), "val")))
        })
        .count()
}

fn origins(rows: &[Placed]) -> Grid {
    rows.iter()
        .map(|row| {
            row_cells(row)
                .into_iter()
                .map(|(cell, column, span)| {
                    let up = find_path(&cell.element, &["tcPr", "vMerge"])
                        .is_some_and(|m| matches!(m.attribute(Some(W), "val"), None | Some("continue")));
                    (column, Origin { cell, span, up })
                })
                .collect()
        })
        .collect()
}

fn merges(line: &BTreeMap<usize, Origin>) -> Vec<String> {
    let mut out = Vec::new();
    for (column, origin) in line {
        if origin.span > 1 {
            out.push(format!("c{column}..c{}", column + origin.span - 1));
        }
        if origin.up {
            out.push(format!("c{column} up"));
        }
    }
    out
}

fn table_merges(block: &Block<'_>, grid: &Grid, reference_grid: &Grid, mapping: &[usize]) -> Vec<Difference> {
    let mut groups: Groups<(Vec<String>, Vec<String>), usize> = Groups::new();
    for (r, line) in grid.iter().enumerate() {
        let Some(reference) = mapping.get(r).and_then(|m| reference_grid.get(*m)) else {
            continue;
        };
        let key = (merges(line), merges(reference));
        if key.0 != key.1 {
            groups.add(key, r);
        }
    }
    let mut out = Vec::new();
    for ((value, reference), rows) in groups {
        let mut calls: Option<Vec<Value>> = Some(Vec::new());
        for r in &rows {
            let line = grid.get(*r);
            let theirs = mapping.get(*r).and_then(|m| reference_grid.get(*m));
            match line.zip(theirs).and_then(|(l, t)| merge_calls(&block.id, *r, l, t)) {
                Some(found) => calls.get_or_insert_with(Vec::new).extend(found),
                None => {
                    calls = None;
                    break;
                }
            }
        }
        let fix = calls
            .filter(|c| !c.is_empty())
            .map_or(Value::Null, |c| fix("word_edit_table", c));
        out.push(difference(
            "merges",
            Some(("rows", json!(rows))),
            json!(value),
            json!(reference),
            fix,
        ));
    }
    out
}

/// `word_edit_table` calls that give row `r` the reference row's horizontal merges; `None`
/// where a vertical merge is involved or the grids differ.
fn merge_calls(
    table: &str,
    r: usize,
    line: &BTreeMap<usize, Origin>,
    reference: &BTreeMap<usize, Origin>,
) -> Option<Vec<Value>> {
    if line.values().chain(reference.values()).any(|o| o.up) {
        return None;
    }
    let set = |l: &BTreeMap<usize, Origin>| -> Vec<(usize, usize)> { l.iter().map(|(c, o)| (*c, o.span)).collect() };
    let (mine, theirs) = (set(line), set(reference));
    let total = |s: &[(usize, usize)]| s.iter().map(|(_, span)| span).sum::<usize>();
    if total(&mine) != total(&theirs) {
        return None;
    }
    let mut calls = Vec::new();
    for (c, span) in mine.iter().filter(|m| !theirs.contains(m) && m.1 > 1) {
        calls.push(
            json!({"table": table, "action": "split", "first": {"row": r, "col": c}, "rows": 1, "columns": span}),
        );
    }
    for (c, span) in theirs.iter().filter(|t| !mine.contains(t) && t.1 > 1) {
        calls.push(json!({"table": table, "action": "merge", "first": {"row": r, "col": c},
                          "last": {"row": r, "col": c + span - 1}}));
    }
    Some(calls)
}

fn width_differences(block: &Block<'_>, widths: &[f64], reference_widths: &[f64]) -> Option<Difference> {
    let differing: Vec<usize> = (0..widths.len().min(reference_widths.len()))
        .filter(|k| !close_numbers(widths.get(*k), reference_widths.get(*k)))
        .collect();
    if differing.is_empty() {
        return None;
    }
    let fix = if widths.len() == reference_widths.len() {
        let calls = differing
            .iter()
            .map(|k| json!({"table": block.id, "scope": "column", "column": k, "width": reference_widths.get(*k)}))
            .collect();
        fix("word_format_table", calls)
    } else {
        Value::Null
    };
    let mut out = difference("widths", None, json!(widths), json!(reference_widths), fix);
    // `columns` stands after `reference` in today's reply
    let fixed = out.remove("fix").unwrap_or(Value::Null);
    out.insert("columns".into(), json!(differing));
    out.insert("fix".into(), fixed);
    Some(out)
}

fn close_numbers(a: Option<&f64>, b: Option<&f64>) -> bool {
    matches!((a, b), (Some(a), Some(b)) if (a - b).abs() <= WIDTH_TOLERANCE)
}

/// `_close`: two numbers within the tolerance, anything else equal.
fn close(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(a), Some(b)) => (a - b).abs() <= WIDTH_TOLERANCE,
        _ => a == b,
    }
}

/// The table's own properties: width, style, alignment, shading and borders.
fn properties(block: &Block<'_>, mine: &Side<'_>, source: &Block<'_>, theirs: &Side<'_>) -> Vec<Difference> {
    let (props, reference) = (table_props(block, mine), table_props(source, theirs));
    let mut out = Vec::new();
    let mut table = |name: &str, value: &Value, reference: &Value, values: Option<Map<String, Value>>| {
        if value != reference {
            let fix = values.map_or(Value::Null, |values| {
                let mut call = Map::new();
                call.insert("table".into(), Value::from(block.id.as_str()));
                call.insert("scope".into(), Value::from("table"));
                call.extend(values);
                fix("word_format_table", vec![Value::Object(call)])
            });
            out.push(difference(name, None, value.clone(), reference.clone(), fix));
        }
    };
    let get = |p: &Map<String, Value>, key: &str| p.get(key).cloned().unwrap_or(Value::Null);
    let (width, reference_width) = (get(&props, "width"), get(&reference, "width"));
    if !close(&width, &reference_width) {
        let fix = match (&reference_width, reference_width.as_str()) {
            (Value::Number(n), _) if n.as_f64().is_some_and(|w| w >= 1.0) => {
                Some(one("width", reference_width.clone()))
            }
            (_, Some(text)) if text.ends_with('%') => text
                .strip_suffix('%')
                .and_then(|t| t.parse::<f64>().ok())
                .map(|p| one("width_percent", json!(p))),
            _ => None,
        };
        table("width", &width, &reference_width, fix);
    }
    let style = get(&reference, "style");
    let fix = style
        .as_str()
        .filter(|s| mine.has_style(s, "table"))
        .map(|_| one("style", style.clone()));
    table("table_style", &get(&props, "style"), &style, fix);
    let alignment = get(&reference, "alignment");
    let fix =
        matches!(alignment.as_str(), Some("left" | "center" | "right")).then(|| one("alignment", alignment.clone()));
    table("table_alignment", &get(&props, "alignment"), &alignment, fix);
    let shading = get(&reference, "shading");
    let written = if shading.is_null() {
        json!("none")
    } else {
        shading.clone()
    };
    table(
        "table_shading",
        &get(&props, "shading"),
        &shading,
        Some(one("shading", written)),
    );
    let borders = get(&reference, "borders");
    let fix = border_fix(&borders, &TABLE_SIDES).map(|style| one("borders", json!(style)));
    table("table_borders", &get(&props, "borders"), &borders, fix);
    out
}

fn table_props(block: &Block<'_>, side: &Side<'_>) -> Map<String, Value> {
    let properties = block.element.find(Some(W), "tblPr");
    let find = |tag: &str| properties.and_then(|p| p.find(Some(W), tag)).map(|e| &**e);
    let style = find("tblStyle").map(|s| side.style_name(s.attribute(Some(W), "val")));
    let alignment = find("jc").and_then(|jc| jc.attribute(Some(W), "val")).map(|a| match a {
        "start" => "left",
        "end" => "right",
        other => other,
    });
    let mut out = Map::new();
    out.insert("style".into(), json!(style.flatten()));
    out.insert("width".into(), width(find("tblW")));
    out.insert("alignment".into(), json!(alignment));
    out.insert("shading".into(), json!(fill(find("shd"))));
    out.insert("borders".into(), borders(find("tblBorders"), &TABLE_SIDES));
    out
}

/// A border as compared: `unset`, `none`, or its style, size, colour, space and theme.
fn side(node: Option<&Element>) -> String {
    let Some(node) = node else { return "unset".into() };
    let read = border(node);
    let text = |key: &str| read.get(key).and_then(Value::as_str);
    let style = text("style");
    if matches!(style, Some("nil" | "none")) {
        return "none".into();
    }
    let size = read.get("size").and_then(Value::as_f64).unwrap_or(0.0);
    let mut words = vec![
        style.unwrap_or("None").to_owned(),
        format!("{}pt", crate::table_read::python_g(size)),
        text("color").unwrap_or("auto").to_owned(),
    ];
    if let Some(space) = read.get("space").and_then(Value::as_i64).filter(|s| *s != 0) {
        words.push(format!("space {space}"));
    }
    if let Some(theme) = text("theme_color").filter(|t| !t.is_empty()) {
        words.push(theme.to_owned());
    }
    words.join(" ")
}

/// A container's borders by side (`null`: none stated).
fn borders(container: Option<&Element>, sides: &[&str]) -> Value {
    let Some(container) = container else { return Value::Null };
    let mut found = Map::new();
    for name in sides {
        let value = side(container.find(Some(W), name).map(|e| &**e));
        if value != "unset" {
            found.insert((*name).to_owned(), Value::from(value));
        }
    }
    if found.is_empty() {
        Value::Null
    } else {
        Value::Object(found)
    }
}

/// `word_format_table`'s `borders` value that writes `reference`, or `None`.
fn border_fix(reference: &Value, sides: &[&str]) -> Option<String> {
    let reference = reference.as_object()?;
    if reference.len() != sides.len() || !sides.iter().all(|s| reference.contains_key(*s)) {
        return None;
    }
    let mut values: Vec<&str> = reference.values().filter_map(Value::as_str).collect();
    values.sort_unstable();
    values.dedup();
    match values.as_slice() {
        ["none"] => Some("none".into()),
        [one] => {
            let (style, rest) = one.split_once(' ').unwrap_or((one, ""));
            (TOOL_BORDERS.contains(&style) && rest == "0.5pt auto").then(|| style.to_owned())
        }
        _ => None,
    }
}

/// What the cells of a table differ in, gathered row by row.
struct CellsFound {
    /// Cell properties that differ, by property and values: the cells `(row, column)`.
    differences: Groups<(&'static str, Value, Value), (usize, usize)>,
    /// Paragraph counts that differ, by counts: the cells.
    counts: Groups<(usize, usize), (usize, usize)>,
    /// Missing and extra paragraphs.
    lost: Vec<Difference>,
    /// The paragraphs paired, to compare.
    pairs: Vec<(Para, Para)>,
}

/// Each cell's shading, borders and vertical alignment, and its paragraphs.
fn cells(
    block: &Block<'_>,
    mine: &Side<'_>,
    source: &Block<'_>,
    theirs: &Side<'_>,
    (grid, reference_grid): (&Grid, &Grid),
    mapping: &[usize],
) -> Vec<Difference> {
    let mut found = CellsFound {
        differences: Groups::new(),
        counts: Groups::new(),
        lost: Vec::new(),
        pairs: Vec::new(),
    };
    for (r, line) in grid.iter().enumerate() {
        let Some(reference_line) = mapping.get(r).and_then(|m| reference_grid.get(*m)) else {
            continue;
        };
        for (column, origin) in line {
            if origin.up {
                continue; // its content is the merge's first cell's
            }
            let Some(matched) = reference_cell(reference_line, *column) else {
                continue;
            };
            found.values(r, (*column, origin), reference_line, matched);
            found.paragraphs(block, (mine, theirs), (r, *column), (&origin.cell, &matched.cell));
        }
    }
    let mut out = Vec::new();
    for ((name, value, reference), at) in found.differences {
        let fix = cell_fix(&block.id, name, &reference, &at, grid);
        let places = cell_refs(&block.id, &at, grid);
        out.push(difference(
            name,
            Some(("cells", places)),
            sorted(value),
            sorted(reference),
            fix,
        ));
    }
    for ((value, reference), at) in found.counts {
        let places = cell_refs(&block.id, &at, grid);
        out.push(difference(
            "paragraphs",
            Some(("cells", places)),
            json!(value),
            json!(reference),
            Value::Null,
        ));
    }
    out.extend(found.lost);
    out.extend(paragraphs(block, mine, source, theirs, &found.pairs));
    out
}

impl CellsFound {
    /// The cell at `column` of row `r` compared with `matched`, its reference cell. A merged
    /// cell the reference has as single cells: the merges fix splits it, and the cells
    /// split off take its properties (as Word's Split Cells), so each is compared with the
    /// reference's cell at its place.
    fn values(
        &mut self,
        r: usize,
        (column, origin): (usize, &Origin),
        line: &BTreeMap<usize, Origin>,
        matched: &Origin,
    ) {
        let split =
            origin.span > 1 && (column..column + origin.span).all(|c| line.get(&c).is_some_and(|o| o.span == 1));
        let places: Vec<usize> = if split {
            (column..column + origin.span).collect()
        } else {
            vec![column]
        };
        for place in places {
            let against = if split {
                line.get(&place).map_or(&matched.cell, |o| &o.cell)
            } else {
                &matched.cell
            };
            for (name, value, reference) in cell_values(&origin.cell.element, &against.element) {
                if value != reference {
                    self.differences.add((name, value, reference), (r, place));
                }
            }
        }
    }

    /// The cell's paragraphs paired with its reference cell's.
    fn paragraphs(
        &mut self,
        block: &Block<'_>,
        (mine, theirs): (&Side<'_>, &Side<'_>),
        (r, column): (usize, usize),
        (cell, reference): (&Placed, &Placed),
    ) {
        let (own, reference) = (cell_paragraphs(cell), cell_paragraphs(reference));
        if reference.is_empty() {
            return;
        }
        let aligned = align(mine, &own, theirs, &reference);
        self.pairs.extend(aligned.pairs);
        for (k, paragraph) in aligned.missing.iter().filter_map(|k| reference.get(*k).map(|p| (k, p))) {
            let mut described = described(theirs, paragraph);
            described.insert("index".into(), json!(k));
            let at = json!([format!("{}/c{r},{column}", block.id)]);
            self.lost.push(difference(
                "missing_paragraph",
                Some(("cells", at)),
                Value::Null,
                Value::Object(described),
                Value::Null,
            ));
        }
        for paragraph in aligned.extra.iter().filter_map(|k| own.get(*k)) {
            if let Some(id) = block.paragraph_id(&paragraph.1) {
                let value = Value::Object(described(mine, paragraph));
                self.lost.push(difference(
                    "extra_paragraph",
                    Some(("paragraphs", json!([id]))),
                    value,
                    Value::Null,
                    Value::Null,
                ));
            }
        }
        if (!aligned.missing.is_empty() || !aligned.extra.is_empty()) && own.len() != reference.len() {
            self.counts.add((own.len(), reference.len()), (r, column));
        }
    }
}

/// A value as docx-agent lists a grouped one: a mapping's keys sorted.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().collect())
        }
        other => other,
    }
}

/// A cell's own paragraphs (`w:p` children), with their places below the table.
fn cell_paragraphs(cell: &Placed) -> Vec<Para> {
    cell.element
        .children()
        .iter()
        .enumerate()
        .filter_map(|(k, node)| match node {
            ooxml_xml::Node::Element(e) if e.name().is(Some(W), "p") => {
                let mut path = cell.path.clone();
                path.push(u32::try_from(k).unwrap_or(u32::MAX));
                Some((Arc::clone(e), path))
            }
            _ => None,
        })
        .collect()
}

fn reference_cell(line: &BTreeMap<usize, Origin>, column: usize) -> Option<&Origin> {
    line.get(&column).or_else(|| {
        line.iter()
            .find(|(start, o)| **start <= column && column < *start + o.span)
            .map(|(_, o)| o)
    })
}

fn cell_values(cell: &Element, reference: &Element) -> [(&'static str, Value, Value); 3] {
    let read = |tc: &Element| {
        let properties = tc.find(Some(W), "tcPr");
        let find = |tag: &str| properties.and_then(|p| p.find(Some(W), tag)).map(|e| &**e);
        // not stated is top (ECMA-376's w:vAlign default), which word_format_table writes
        let valign = find("vAlign")
            .and_then(|v| v.attribute(Some(W), "val"))
            .filter(|v| !v.is_empty())
            .unwrap_or("top");
        (
            json!(fill(find("shd"))),
            borders(find("tcBorders"), &CELL_SIDES),
            json!(valign),
        )
    };
    let (mine, theirs) = (read(cell), read(reference));
    [
        ("shading", mine.0, theirs.0),
        ("borders", mine.1, theirs.1),
        ("vertical_alignment", mine.2, theirs.2),
    ]
}

/// Per row, its own cells' columns (not merged up), in order.
fn own_columns(line: &BTreeMap<usize, Origin>) -> Vec<usize> {
    line.iter().filter(|(_, o)| !o.up).map(|(c, _)| *c).collect()
}

fn by_row(cells: &[(usize, usize)]) -> BTreeMap<usize, Vec<usize>> {
    let mut out: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (r, c) in cells {
        out.entry(*r).or_default().push(*c);
    }
    for found in out.values_mut() {
        found.sort_unstable();
    }
    out
}

/// `t:X/cR,C` per cell, `t:X/cR,*` for every cell of a row.
fn cell_refs(table: &str, cells: &[(usize, usize)], grid: &Grid) -> Value {
    let mut out = Vec::new();
    for (r, found) in by_row(cells) {
        let own = grid.get(r).map(own_columns).unwrap_or_default();
        if own.len() > 1 && found == own {
            out.push(format!("{table}/c{r},*"));
        } else {
            out.extend(found.iter().map(|c| format!("{table}/c{r},{c}")));
        }
    }
    json!(out)
}

fn cell_fix(table: &str, name: &str, reference: &Value, cells: &[(usize, usize)], grid: &Grid) -> Value {
    let values = match name {
        "shading" => one(
            "shading",
            if reference.is_null() {
                json!("none")
            } else {
                reference.clone()
            },
        ),
        "vertical_alignment" => match reference.as_str() {
            Some("top" | "center" | "bottom") => one("vertical_alignment", reference.clone()),
            _ => return Value::Null,
        },
        _ => match border_fix(reference, &CELL_SIDES) {
            Some(style) => one("borders", json!(style)),
            None => return Value::Null,
        },
    };
    let mut calls = Vec::new();
    for (r, found) in by_row(cells) {
        let line = grid.get(r);
        let own = line.map(own_columns).unwrap_or_default();
        let merged_up = line.is_some_and(|l| l.values().any(|o| o.up));
        if own.len() > 1 && found == own && !merged_up {
            let mut call = Map::new();
            call.insert("table".into(), json!(table));
            call.insert("scope".into(), json!("row"));
            call.insert("row".into(), json!(r));
            call.extend(values.clone());
            calls.push(Value::Object(call));
        } else {
            for c in found {
                let mut call = Map::new();
                call.insert("table".into(), json!(table));
                call.insert("scope".into(), json!("cell"));
                call.insert("row".into(), json!(r));
                call.insert("column".into(), json!(c));
                call.extend(values.clone());
                calls.push(Value::Object(call));
            }
        }
    }
    fix("word_format_table", calls)
}
