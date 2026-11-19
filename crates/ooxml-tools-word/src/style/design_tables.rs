//! Owns: `word_design_facts`' `table_styles` (docx-agent's `edit/design.py`, "table
//! styles"): every table style in use in the body, headers and footers, with the tables'
//! most common style options and cell paragraph style, and its look by place -- a sample
//! table of five rows and four columns resolved as the layout model resolves a table, each
//! place's regions, fill, borders after the conflicts between neighbours and text, a theme
//! colour named by the element that declared it (the cell's origins); and the places of a
//! declared table style ([`places`]).
//!
//! Must not: change a document, or read tables other than as the layout model reads them
//! ([`TableFacts`]).

use super::design_look::{border_text, fill, Design};
use ooxml_base::Cancel;
use ooxml_tools::ToolError;
use ooxml_word_model::docx::cells::{model_path, TableFacts};
use ooxml_word_model::docx::formatting::Cell;
use ooxml_word_model::docx::StoryPart;
use ooxml_word_model::ids::W;
use ooxml_word_model::model::{Block, Look, Table};
use ooxml_word_model::read::cell_conditions;
use ooxml_word_model::resolve::{
    banded, border_key, resolve_table_traced, second_wins, Origins, ResolvedTable, TableOrigins,
};
use ooxml_word_model::styles::StyleKind;
use ooxml_word_model::value::{key, Props, Text};
use ooxml_xml::{Element, Node};
use serde_json::{Map, Value};
use std::sync::Arc;

/// The sample a table style's look is read from: rows and columns.
const SAMPLE_ROWS: usize = 5;
const SAMPLE_COLUMNS: usize = 4;
/// Example tables listed per style.
const EXAMPLES: usize = 3;

/// Counts kept in the order first seen (Python's `Counter`, whose `most_common` keeps that
/// order among ties).
pub(crate) struct Tally<K>(Vec<(K, usize)>);

impl<K: PartialEq> Tally<K> {
    pub(crate) fn new() -> Self {
        Tally(Vec::new())
    }

    pub(crate) fn add(&mut self, item: K) {
        match self.0.iter_mut().find(|(k, _)| *k == item) {
            Some((_, n)) => *n += 1,
            None => self.0.push((item, 1)),
        }
    }

    /// The most common, the first seen among ties.
    pub(crate) fn most_common(&self) -> Option<&K> {
        let mut best: Option<&(K, usize)> = None;
        for entry in &self.0 {
            if best.is_none_or(|b| entry.1 > b.1) {
                best = Some(entry);
            }
        }
        best.map(|(k, _)| k)
    }
}

/// The tables in use with one table style.
struct Group {
    style: Option<Text>,
    tables: usize,
    looks: Tally<Look>,
    cells: Tally<Option<Text>>,
    ids: Vec<String>,
}

/// The block-level tables of a story part's root (through content controls and custom
/// XML, as the layout model reads blocks), by place.
fn block_tables(container: &Element, path: &mut Vec<u32>, out: &mut Vec<Vec<u32>>) {
    for (k, node) in container.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        if child.name().namespace() != Some(W) {
            continue;
        }
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        match child.name().local() {
            "tbl" => out.push(path.clone()),
            "body" | "customXml" | "sdtContent" => block_tables(child, path, out),
            "sdt" => {
                if let Some(j) = child
                    .children()
                    .iter()
                    .position(|n| matches!(n, Node::Element(c) if c.name().is(Some(W), "sdtContent")))
                {
                    path.push(u32::try_from(j).unwrap_or(u32::MAX));
                    if let Some(content) = child.children().get(j).and_then(Node::as_element) {
                        block_tables(content, path, out);
                    }
                    path.pop();
                }
            }
            _ => {}
        }
        path.pop();
    }
}

/// The element at `path` below `root`.
pub(crate) fn element_at(root: &Arc<Element>, path: &[u32]) -> Option<Arc<Element>> {
    let mut node = Arc::clone(root);
    for k in path {
        node = Arc::clone(node.children().get(*k as usize)?.as_element()?);
    }
    Some(node)
}

/// The tables of one top-level table of `part` at `path` as the layout model reads them,
/// each with its id where the part's index has it.
fn model_tables(
    design: &Design<'_>,
    part: &StoryPart,
    indexed: &[(Box<[u32]>, String)],
    path: &[u32],
) -> Vec<(Arc<Table>, Option<String>)> {
    let Some(element) = element_at(part.root(), path) else {
        return Vec::new();
    };
    let facts = match part.ids.table_at(path) {
        Some(id) => design.shared().table_facts(&part.part, &id),
        None => Arc::new(TableFacts::read(design.formatting(), &element)),
    };
    let ids: Vec<(String, String)> = indexed
        .iter()
        .filter(|(p, _)| p.starts_with(path))
        .filter_map(|(p, id)| Some((model_path(&element, p.get(path.len()..)?)?, id.clone())))
        .collect();
    facts
        .tables()
        .iter()
        .map(|table| {
            let id = ids.iter().find(|(p, _)| **p == *table.path).map(|(_, id)| id.clone());
            (Arc::clone(table), id)
        })
        .collect()
}

/// Every table style in use, with its look by place, most used first.
pub(crate) fn table_styles(design: &Design<'_>, cancel: &dyn Cancel) -> Result<Vec<Value>, ToolError> {
    let sources = design.formatting().sources();
    let mut groups: Vec<Group> = Vec::new();
    for part in &design.docx.parts {
        let story = part.story.as_str();
        if !(story == "body" || story.starts_with("header") || story.starts_with("footer")) {
            continue;
        }
        cancel.check().map_err(crate::describe::cancelled)?;
        let mut found = Vec::new();
        block_tables(part.root(), &mut Vec::new(), &mut found);
        // each indexed table's place and id, read once for the part
        let indexed: Vec<(Box<[u32]>, String)> = part.ids.tables().iter().map(|t| (t.path, t.id)).collect();
        for path in found {
            for (table, id) in model_tables(design, part, &indexed, &path) {
                let style = sources.table_style_of(&table);
                let k = match groups.iter().position(|g| g.style == style) {
                    Some(k) => k,
                    None => {
                        groups.push(Group {
                            style: style.clone(),
                            tables: 0,
                            looks: Tally::new(),
                            cells: Tally::new(),
                            ids: Vec::new(),
                        });
                        groups.len() - 1
                    }
                };
                let Some(group) = groups.get_mut(k) else { continue };
                group.tables += 1;
                group.looks.add(table.look);
                for row in table.rows.iter() {
                    for cell in row.iter() {
                        for block in cell.iter() {
                            if let Block::Paragraph(p) = block {
                                group.cells.add(p.properties.as_ref().and_then(|p| p.style_id.clone()));
                            }
                        }
                    }
                }
                if let Some(id) = id.filter(|_| group.ids.len() < EXAMPLES) {
                    group.ids.push(id);
                }
            }
        }
    }
    let mut out = Vec::new();
    for group in &groups {
        cancel.check().map_err(crate::describe::cancelled)?;
        let look = group.looks.most_common().copied().unwrap_or_default();
        let cell_style = group.cells.most_common().cloned().flatten();
        let name = group
            .style
            .as_deref()
            .and_then(|s| design.sheet()?.get(s))
            .map(|s| s.name.clone().unwrap_or_else(|| s.style_id.clone()));
        let mut entry = Map::new();
        entry.insert("style".into(), name.as_deref().map_or(Value::Null, Value::from));
        entry.insert("tables".into(), Value::from(group.tables));
        entry.insert("examples".into(), Value::from(group.ids.clone()));
        entry.insert("options".into(), options(look));
        if let Some(style) = &group.style {
            entry.insert(
                "places".into(),
                Value::Object(places(design, style, look, cell_style.as_deref())),
            );
        }
        out.push((group.tables, name, Value::Object(entry)));
    }
    out.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.as_deref().unwrap_or("").cmp(b.1.as_deref().unwrap_or("")))
    });
    Ok(out.into_iter().map(|(_, _, v)| v).collect())
}

/// `w:tblLook` as Word's Table Design options name them.
fn options(look: Look) -> Value {
    let mut out = Map::new();
    for (name, on) in [
        ("header_row", look.first_row),
        ("total_row", look.last_row),
        ("first_column", look.first_column),
        ("last_column", look.last_column),
        ("banded_rows", !look.no_h_band),
        ("banded_columns", !look.no_v_band),
    ] {
        out.insert(name.into(), Value::Bool(on));
    }
    Value::Object(out)
}

/// The sample table of `style` under `look`, its cells' regions.
fn sample(style: &str, look: Look) -> Table {
    let conditions: Vec<Arc<[Arc<[Text]>]>> = (0..SAMPLE_ROWS)
        .map(|r| {
            (0..SAMPLE_COLUMNS)
                .map(|c| Arc::from(cell_conditions(look, r, c, SAMPLE_ROWS, SAMPLE_COLUMNS, 0)))
                .collect()
        })
        .collect();
    let empty: Arc<[Block]> = Arc::from(Vec::new());
    Table {
        style_id: Some(key(style)),
        rows: (0..SAMPLE_ROWS)
            .map(|_| (0..SAMPLE_COLUMNS).map(|_| Arc::clone(&empty)).collect())
            .collect(),
        path: key("T"),
        grid: Arc::from(vec![1440; SAMPLE_COLUMNS]),
        properties: Props::new(),
        row_properties: (0..SAMPLE_ROWS).map(|_| Props::new()).collect(),
        cell_properties: (0..SAMPLE_ROWS)
            .map(|_| (0..SAMPLE_COLUMNS).map(|_| Props::new()).collect())
            .collect(),
        look,
        conditions: Arc::from(conditions),
        ..Table::default()
    }
}

/// The sample table's places under `look` (header row, first and second row band or the
/// body, first and last column, total row, corner, as the options turn them on): each
/// one's regions, fill, borders and text.
pub(crate) fn places(design: &Design<'_>, style: &str, look: Look, cell_style: Option<&str>) -> Map<String, Value> {
    let table = sample(style, look);
    let (resolved, origins) = resolve_table_traced::<Origins>(&design.formatting().sources(), &table);
    let chain = design
        .sheet()
        .map(|s| s.chain(Some(style), StyleKind::Table))
        .unwrap_or_default();
    let first_row = usize::from(look.first_row);
    let first_column = usize::from(look.first_column);
    let mut wanted = Vec::new();
    if look.first_row {
        wanted.push(("header_row", 0, first_column));
    }
    if look.no_h_band {
        wanted.push(("body", first_row, first_column));
    } else {
        wanted.push(("band1", first_row, first_column));
        let size = chain
            .iter()
            .rev()
            .find_map(|s| s.table.get("tblStyleRowBandSize").map(|v| v.int().unwrap_or(0)))
            .filter(|s| *s > 0)
            .unwrap_or(1);
        let at = (first_row + usize::try_from(size).unwrap_or(1)).min(SAMPLE_ROWS - 1);
        wanted.push(("band2", at, first_column));
    }
    if look.first_column {
        wanted.push(("first_column", first_row, 0));
    }
    if look.last_column {
        wanted.push(("last_column", first_row, SAMPLE_COLUMNS - 1));
    }
    if look.last_row {
        wanted.push(("total_row", SAMPLE_ROWS - 1, first_column));
    }
    if look.first_row && look.first_column {
        wanted.push(("corner", 0, 0));
    }
    let mut out = Map::new();
    for (name, r, c) in wanted {
        let conditions = table
            .conditions
            .get(r)
            .and_then(|row| row.get(c))
            .cloned()
            .unwrap_or_else(|| Arc::from(Vec::new()));
        let mut place = Map::new();
        let named: Vec<Value> = banded(&chain, &conditions).iter().map(|c| Value::from(&**c)).collect();
        place.insert("conditions".into(), Value::Array(named));
        cell_fill(design, &resolved, &origins, (r, c), &mut place);
        place.insert("borders".into(), borders(design, &resolved, &origins, r, c));
        let cell = Cell {
            style: key(style),
            conditions,
        };
        let look = design.style_look(cell_style, Some(&cell), None);
        let mut text = design.text_look(&look.run, &look.run_origins);
        if look.paragraph.alignment.as_deref() != Some("left") {
            text.insert(
                "alignment".into(),
                look.paragraph.alignment.clone().map_or(Value::Null, Value::from),
            );
        }
        place.insert("text".into(), Value::Object(text));
        out.insert(name.into(), Value::Object(place));
    }
    out
}

/// A place's fill, with `theme_fill` where the shading that gave it names a theme colour.
fn cell_fill(
    design: &Design<'_>,
    resolved: &ResolvedTable,
    origins: &TableOrigins<Origins>,
    (r, c): (usize, usize),
    place: &mut Map<String, Value>,
) {
    let cell = resolved.rows.get(r).and_then(|row| row.cells.get(c));
    let found = fill(cell.and_then(|cell| cell.properties.get("shd")));
    let themed = found.as_ref().and_then(|_| {
        let origin = origins
            .cells
            .get(r)
            .and_then(|row| row.get(c))
            .and_then(|o| o.of("shd"));
        design.themed(origin, "shd", true)
    });
    match themed {
        Some((rgb, label)) => {
            place.insert("fill".into(), Value::from(rgb));
            place.insert("theme_fill".into(), Value::from(label));
        }
        None => {
            place.insert("fill".into(), found.map_or(Value::Null, Value::from));
        }
    }
}

/// A cell side's border (`k` in the resolver's side order) as drawn, with the theme colour
/// of the element that declared it.
fn side_text(
    design: &Design<'_>,
    resolved: &ResolvedTable,
    origins: &TableOrigins<Origins>,
    at: (usize, usize, usize),
) -> String {
    let (r, c, k) = at;
    let Some(cell) = resolved.rows.get(r).and_then(|row| row.cells.get(c)) else {
        return "none".into();
    };
    let columns = i64::try_from(resolved.grid.len()).unwrap_or(i64::MAX);
    let (own, from) = border_key(cell, k, resolved.rows.len(), columns);
    let origin = if own {
        origins
            .cells
            .get(r)
            .and_then(|row| row.get(c))
            .and_then(|o| o.of(&from))
    } else {
        origins.table.of(&from)
    };
    let border = cell.borders.get(k).and_then(Option::as_ref);
    border_text(border, design.themed(origin, &from, false).as_ref())
}

/// The border drawn on each side of cell (`r`, `c`) after the conflicts with its
/// neighbours: the wider, else the upper (left) one's.
fn borders(
    design: &Design<'_>,
    resolved: &ResolvedTable,
    origins: &TableOrigins<Origins>,
    r: usize,
    c: usize,
) -> Value {
    let border = |r: usize, c: usize, k: usize| {
        resolved
            .rows
            .get(r)
            .and_then(|row| row.cells.get(c))
            .and_then(|cell| cell.borders.get(k))
            .and_then(Option::as_ref)
    };
    let last_row = resolved.rows.len().saturating_sub(1);
    let last_cell = resolved.rows.get(r).map_or(0, |row| row.cells.len().saturating_sub(1));
    // (side name, the cell and side whose border is drawn there)
    let top = if r > 0 && !second_wins(border(r - 1, c, 2), border(r, c, 0)) {
        (r - 1, c, 2)
    } else {
        (r, c, 0)
    };
    let bottom = if r < last_row && second_wins(border(r, c, 2), border(r + 1, c, 0)) {
        (r + 1, c, 0)
    } else {
        (r, c, 2)
    };
    let left = if c > 0 && !second_wins(border(r, c - 1, 3), border(r, c, 1)) {
        (r, c - 1, 3)
    } else {
        (r, c, 1)
    };
    let right = if c < last_cell && second_wins(border(r, c, 3), border(r, c + 1, 1)) {
        (r, c + 1, 1)
    } else {
        (r, c, 3)
    };
    let mut out = Map::new();
    for (name, at) in [("top", top), ("bottom", bottom), ("left", left), ("right", right)] {
        out.insert(name.into(), Value::from(side_text(design, resolved, origins, at)));
    }
    Value::Object(out)
}
