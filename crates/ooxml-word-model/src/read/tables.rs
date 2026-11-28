//! Owns: a `w:tbl` in the final view (Accept All; measured, docx2svg's ROADMAP "Tables --
//! measured" stage 7f and "Revisions -- measured" R.3): deleted rows and cells left out, the
//! grid columns only they covered going with them, a deleted cell whose content is kept
//! absorbed by the cell before it, a cell whose last mark is deleted taking the next cell
//! in, content moved out of the table before its row; and where each cell is in its table,
//! for the table style's regions ([`cell_conditions`], measured round 58).
//!
//! Must not: resolve a table's properties (`crate::resolve::table`).

use super::fields::Fields;
use super::join::{join, joined};
use super::{At, Reader};
use crate::docx::tables::{merge_heads, VMerge};
use crate::model::{Block, Look, Paragraph, Table};
use crate::props::table_properties;
use crate::value::{text, Key, Props, Text, Value};
use crate::view::{cell_ends, content_kept, CellEnds, View};
use crate::w;
use crate::xmlread::{attr, attr_on, child, elements, local};
use ooxml_xml::Element;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Whether the final view removes a cell.
fn removed(cell: &Element) -> bool {
    View::Final.removes_cell(cell)
}

/// Whether a cell the final view removes widens the cell before it (its content stays).
fn widens(cell: &Element) -> bool {
    removed(cell) && content_kept(cell)
}

/// `parent`'s `w:tr` or `w:tc` children, also those inside row- and cell-level content
/// controls and `w:customXml`.
pub(crate) fn row_level<'a>(parent: &'a Element, name: &str, out: &mut Vec<&'a Element>) {
    for node in elements(parent) {
        let l = local(node);
        if l == name {
            out.push(node);
        } else if l == "sdt" {
            if let Some(content) = child(node, "sdtContent") {
                row_level(content, name, out);
            }
        } else if l == "customXml" {
            row_level(node, name, out);
        }
    }
}

fn level_of<'a>(parent: &'a Element, name: &str) -> Vec<&'a Element> {
    let mut out = Vec::new();
    row_level(parent, name, &mut out);
    out
}

/// [`level_of`] with each element's place below `parent` (child indexes, wrappers passed
/// through).
fn level_placed<'a>(parent: &'a Element, name: &str) -> Vec<(Vec<usize>, &'a Element)> {
    fn walk<'a>(parent: &'a Element, name: &str, above: &mut Vec<usize>, out: &mut Vec<(Vec<usize>, &'a Element)>) {
        for (k, node) in parent.children().iter().enumerate() {
            let Some(node) = node.as_element() else { continue };
            above.push(k);
            match local(node) {
                l if l == name => out.push((above.clone(), node)),
                "sdt" => {
                    let content = node
                        .children()
                        .iter()
                        .enumerate()
                        .find_map(|(c, n)| n.as_element().filter(|e| local(e) == "sdtContent").map(|e| (c, e)));
                    if let Some((c, content)) = content {
                        above.push(c);
                        walk(content, name, above, out);
                        above.pop();
                    }
                }
                "customXml" => walk(node, name, above, out),
                _ => {}
            }
            above.pop();
        }
    }
    let mut out = Vec::new();
    walk(parent, name, &mut Vec::new(), &mut out);
    out
}

fn absorb(previous: &mut Props, deleted: &Props) {
    let span = |p: &Props| {
        p.get("gridSpan")
            .and_then(Value::int)
            .filter(|s| *s != 0)
            .unwrap_or(1)
            .max(1)
    };
    let total = span(previous) + span(deleted);
    previous.insert(Key::Borrowed("gridSpan"), Value::Int(total));
    if let (Some(Value::Width(mine, mk)), Some(Value::Width(theirs, tk))) = (previous.get("tcW"), deleted.get("tcW")) {
        if &**mk == "dxa" && &**tk == "dxa" {
            let sum = Value::Width(mine + theirs, text("dxa"));
            previous.insert(Key::Borrowed("tcW"), sum);
        }
    }
}

fn accepted_cell_properties(reader: &Reader<'_>, cell: &Element) -> Props {
    let props = child(cell, "tcPr");
    let mut out = table_properties(props, reader.colours());
    match props
        .and_then(|p| w::TcPr::new(p).merge())
        .and_then(w::CellMerge::v_merge)
    {
        Some("rest") => {
            out.insert(Key::Borrowed("vMerge"), Value::Str(text("restart")));
        }
        Some("cont") => {
            out.insert(Key::Borrowed("vMerge"), Value::Str(text("continue")));
        }
        _ => {}
    }
    out
}

/// A cell's properties with the merge Accept All gives it where the rows above it in its
/// merge go ([`merge_heads`]).
fn headed(props: &mut Props, head: Option<&VMerge>) {
    match head {
        Some(VMerge::Restart) => {
            props.insert(Key::Borrowed("vMerge"), Value::Str(text("restart")));
        }
        Some(_) => {
            props.remove("vMerge");
        }
        None => {}
    }
}

fn cell_taken(mut blocks: Vec<Block>, taken: Vec<Block>) -> Vec<Block> {
    if let (Some(Block::Paragraph(last)), Some(Block::Paragraph(first))) = (blocks.last(), taken.first()) {
        let made = Block::Paragraph(Arc::new(joined(last, first)));
        blocks.pop();
        blocks.push(made);
        blocks.extend(taken.into_iter().skip(1));
        return blocks;
    }
    blocks.extend(taken);
    blocks
}

fn moved_out(blocks: &[Block]) -> Vec<Block> {
    blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(p) => Block::Paragraph(Arc::new(Paragraph {
                table_style_id: None,
                table_conditions: Arc::from(Vec::new()),
                ..(**p).clone()
            })),
            other => other.clone(),
        })
        .collect()
}

/// The grid columns only cells with a pending deletion (or gone, `gone`) cover, in every
/// row covering them. Only the grid's `count` columns are answered, so only those are
/// counted: a span past them stops there.
fn vacated_columns(rows: &[&Element], count: usize, gone: &HashSet<*const Element>) -> HashSet<usize> {
    let mut covered: std::collections::BTreeMap<i64, bool> = std::collections::BTreeMap::new();
    let end = i64::try_from(count).unwrap_or(i64::MAX);
    for row in rows {
        let props = child(row, "trPr").map(w::TrPr::new);
        let mut column = props.and_then(w::TrPr::grid_before).and_then(w::Val::int).unwrap_or(0);
        let mut skipped: Vec<i64> = (0..column.min(end)).collect();
        let mut kept_before = false;
        for cell in level_of(row, "tc") {
            let span = child(cell, "tcPr")
                .and_then(|p| w::TcPr::new(p).span())
                .and_then(w::Val::int)
                .filter(|s| *s != 0)
                .unwrap_or(1)
                .max(1);
            let deleted =
                (removed(cell) && !(kept_before && content_kept(cell))) || gone.contains(&std::ptr::from_ref(cell));
            kept_before = kept_before || !removed(cell);
            for c in column..column.saturating_add(span).min(end) {
                let was = covered.get(&c).copied().unwrap_or(true);
                covered.insert(c, was && deleted);
            }
            column = column.saturating_add(span);
        }
        let after = props.and_then(w::TrPr::grid_after).and_then(w::Val::int).unwrap_or(0);
        skipped.extend(column..column.saturating_add(after).min(end));
        for c in skipped {
            covered.insert(c, false);
        }
    }
    covered
        .into_iter()
        .filter(|(c, only)| *only && *c >= 0)
        .filter_map(|(c, _)| usize::try_from(c).ok())
        .filter(|c| *c < count)
        .collect()
}

/// `w:tblLook`: which regions the table turns on (ECMA-376's default where absent).
pub(crate) fn table_look(element: Option<&Element>) -> Look {
    let Some(element) = element else {
        return Look {
            first_row: true,
            first_column: true,
            no_v_band: true,
            ..Look::default()
        };
    };
    let mask = attr(element, "val").filter(|m| !m.is_empty() && m.chars().all(|c| c.is_ascii_hexdigit()));
    let bits = mask.and_then(|m| i64::from_str_radix(m, 16).ok()).unwrap_or(0);
    let flag = |name: &str, bit: i64| match attr(element, name) {
        Some(_) => attr_on(element, name),
        None => bits & bit != 0,
    };
    Look {
        first_row: flag("firstRow", 0x20),
        last_row: flag("lastRow", 0x40),
        first_column: flag("firstColumn", 0x80),
        last_column: flag("lastColumn", 0x100),
        no_h_band: flag("noHBand", 0x200),
        no_v_band: flag("noVBand", 0x400),
    }
}

/// Where cell (row, column) is in its table, for the table style's regions, lowest first:
/// the row band, the column band, first and last column, first and last row, the corner.
/// Bands are positions (`hBand:<n>`, `vBand:<n>`) named against the style later.
pub fn cell_conditions(
    look: Look,
    row: usize,
    column: usize,
    rows: usize,
    columns: usize,
    header_rows: usize,
) -> Vec<Text> {
    let heading = if look.first_row {
        header_rows.max(1)
    } else {
        header_rows
    };
    let first_row = look.first_row && row < heading;
    let last_row = look.last_row && row + 1 == rows;
    let first_col = look.first_column && column == 0;
    let last_col = look.last_column && column + 1 == columns;
    let signed = |a: usize, b: usize| i64::try_from(a).unwrap_or(i64::MAX) - i64::try_from(b).unwrap_or(i64::MAX);
    let mut out = Vec::new();
    if !look.no_h_band && !(first_row || last_row) {
        out.push(text(&format!("hBand:{}", signed(row, heading))));
    }
    if !look.no_v_band && !(first_col || last_col) {
        out.push(text(&format!(
            "vBand:{}",
            signed(column, usize::from(look.first_column))
        )));
    }
    for (name, on) in [
        ("firstCol", first_col),
        ("lastCol", last_col),
        ("firstRow", first_row),
        ("lastRow", last_row),
    ] {
        if on {
            out.push(text(name));
        }
    }
    if (first_row || last_row) && (first_col || last_col) {
        let corner = match (first_col, first_row) {
            (true, true) => "nwCell",
            (true, false) => "swCell",
            (false, true) => "neCell",
            (false, false) => "seCell",
        };
        out.push(text(corner));
    }
    out
}

/// How many rows at the start repeat as header rows (`w:tblHeader`, not off).
pub fn leading_header_rows(rows: &[&Element]) -> usize {
    rows.iter()
        .take_while(|row| {
            let flag = child(row, "trPr").and_then(|p| child(p, "tblHeader"));
            flag.is_some_and(|f| !matches!(attr(f, "val"), Some("0" | "false" | "off")))
        })
        .count()
}

/// A table being read: its rows' blocks and properties, and the blocks moved out of it.
#[derive(Default)]
struct Built {
    rows: Vec<Arc<[Arc<[Block]>]>>,
    row_properties: Vec<Props>,
    cell_properties: Vec<Arc<[Props]>>,
    conditions: Vec<Arc<[Arc<[Text]>]>>,
    cell_places: Vec<Arc<[(u32, u32)]>>,
    moved: Vec<(usize, Vec<Block>)>,
}

/// What every row of a table is read with.
struct Shape<'t> {
    style: &'t Text,
    at: &'t At<'t>,
    look: Look,
    rows: usize,
    headers: usize,
    /// The cells continuing a merge that Accept All starts again or leaves alone.
    heads: &'t HashMap<(usize, usize), VMerge>,
}

/// One row's plan: its cells shown, and their groups.
struct Plan<'a> {
    index: usize,
    /// Its place below the table.
    rel: Vec<usize>,
    row: &'a Element,
    shown: Vec<&'a Element>,
    groups: Vec<Vec<usize>>,
}

impl Reader<'_> {
    fn cell_blocks(
        &self,
        cell: &Element,
        style: &Text,
        conditions: &Arc<[Text]>,
        at: &At<'_>,
        fields: &mut Fields,
    ) -> Vec<Block> {
        join(self.block_list(cell, Some((style, conditions)), at, fields))
    }

    fn row(&self, k: usize, plan: &Plan<'_>, shape: &Shape<'_>, fields: &mut Fields, built: &mut Built) {
        let style = shape.style;
        let kept: HashSet<*const Element> = plan
            .groups
            .iter()
            .filter_map(|g| g.first())
            .filter_map(|&n| plan.shown.get(n))
            .map(|c| std::ptr::from_ref(*c))
            .collect();
        let placed = level_placed(plan.row, "tc");
        let all: Vec<&Element> = placed.iter().map(|(_, c)| *c).collect();
        // the cell `j` of the row's own, where it is read
        let at = |j: usize| {
            let rel: Vec<usize> = plan
                .rel
                .iter()
                .chain(placed.get(j).map(|(r, _)| r.as_slice()).unwrap_or_default())
                .copied()
                .collect();
            shape.at.cell((count(plan.index), count(j)), &rel)
        };
        let mut cells: Vec<(usize, &Element)> = Vec::new();
        let mut props_of: Vec<Props> = Vec::new();
        for (j, cell) in all.iter().enumerate() {
            if kept.contains(&std::ptr::from_ref(*cell)) {
                cells.push((j, cell));
                let mut props = accepted_cell_properties(self, cell);
                headed(&mut props, shape.heads.get(&(plan.index, j)));
                props_of.push(props);
            } else if let Some(last) = props_of.last_mut().filter(|_| widens(cell)) {
                absorb(last, &table_properties(child(cell, "tcPr"), self.colours()));
            }
        }
        let row_conditions: Vec<Arc<[Text]>> = (0..cells.len())
            .map(|n| {
                Arc::from(cell_conditions(
                    shape.look,
                    k,
                    n,
                    shape.rows,
                    cells.len(),
                    shape.headers,
                ))
            })
            .collect();
        let places: Vec<(u32, u32)> = cells.iter().map(|(j, _)| (count(plan.index), count(*j))).collect();
        let mut row_blocks: Vec<Arc<[Block]>> = Vec::new();
        for (n, (j, cell)) in cells.iter().enumerate() {
            let Some(conds) = row_conditions.get(n) else {
                continue;
            };
            let mut blocks = self.cell_blocks(cell, style, conds, &at(*j), fields);
            let group = plan.groups.iter().find(|g| {
                g.first()
                    .and_then(|&f| plan.shown.get(f))
                    .is_some_and(|c| std::ptr::eq(*c, *cell))
            });
            let others: Vec<&Element> = group
                .map(|g| g.iter().skip(1).filter_map(|&o| plan.shown.get(o).copied()).collect())
                .unwrap_or_default();
            if !others.is_empty() {
                if n == 0 && blocks.len() > 1 {
                    let last = blocks.split_off(blocks.len() - 1);
                    built.moved.push((k, moved_out(&blocks)));
                    blocks = last;
                }
                for other in others {
                    let j = all.iter().position(|c| std::ptr::eq(*c, other)).unwrap_or(0);
                    let taken = self.cell_blocks(other, style, conds, &at(j), fields);
                    blocks = cell_taken(blocks, taken);
                }
            }
            row_blocks.push(Arc::from(blocks));
        }
        built.rows.push(Arc::from(row_blocks));
        built
            .row_properties
            .push(table_properties(child(plan.row, "trPr"), self.colours()));
        built.cell_properties.push(Arc::from(props_of));
        built.conditions.push(Arc::from(row_conditions));
        built.cell_places.push(Arc::from(places));
    }

    /// A `w:tbl`'s blocks in the final view: the table, or its parts with the blocks moved
    /// out between them.
    pub(crate) fn table_blocks(&self, element: &Element, at: &At<'_>, fields: &mut Fields) -> Vec<Block> {
        let props_element = child(element, "tblPr");
        let tbl_pr = props_element.map(w::TblPr::new);
        let style = text(tbl_pr.and_then(w::TblPr::style).and_then(w::Val::val).unwrap_or(""));
        let look = table_look(tbl_pr.and_then(w::TblPr::look).map(w::TblLook::element));
        let kept = |row: &Element| !View::Final.removes_row(row);
        let placed_rows = level_placed(element, "tr");
        let all_rows: Vec<&Element> = placed_rows.iter().map(|(_, r)| *r).collect();
        let heads = merge_heads(&all_rows, &kept, &|cell| View::Final.merge(cell));
        let row_elements: Vec<(usize, &Element)> = all_rows
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, row)| kept(row))
            .collect();
        let grid_nodes: Vec<i64> = child(element, "tblGrid")
            .map(|g| w::TblGrid::new(g).columns().map(|c| c.w().unwrap_or(0)).collect())
            .unwrap_or_default();
        let mut gone: HashSet<*const Element> = HashSet::new();
        let mut plans = Vec::new();
        for (index, row) in &row_elements {
            let rel = placed_rows.get(*index).map(|(r, _)| r.clone()).unwrap_or_default();
            let shown: Vec<&Element> = level_of(row, "tc").into_iter().filter(|c| !removed(c)).collect();
            let CellEnds { groups, gone: lost } = cell_ends(&shown);
            gone.extend(
                lost.iter()
                    .filter_map(|&n| shown.get(n))
                    .map(|c| std::ptr::from_ref(*c)),
            );
            plans.push(Plan {
                index: *index,
                rel,
                row,
                shown,
                groups,
            });
        }
        let rows_only: Vec<&Element> = row_elements.iter().map(|(_, r)| *r).collect();
        let headers = leading_header_rows(&rows_only);
        let mut built = Built::default();
        let shape = Shape {
            style: &style,
            at,
            look,
            rows: row_elements.len(),
            headers,
            heads: &heads,
        };
        for (k, plan) in plans.iter().enumerate() {
            self.row(k, plan, &shape, fields, &mut built);
        }
        let grid_of = |start: usize, end: usize| -> Arc<[i64]> {
            let rows: Vec<&Element> = rows_only.get(start..end).unwrap_or_default().to_vec();
            let vacated = vacated_columns(&rows, grid_nodes.len(), &gone);
            grid_nodes
                .iter()
                .enumerate()
                .filter(|(k, _)| !vacated.contains(k))
                .map(|(_, w)| *w)
                .collect()
        };
        let (key, unkeyed) = at.name();
        let table = Table {
            style_id: (!style.is_empty()).then(|| style.clone()),
            rows: Arc::from(std::mem::take(&mut built.rows)),
            key,
            unkeyed_cells: unkeyed.clone(),
            unkeyed,
            grid: grid_of(0, plans.len()),
            properties: table_properties(props_element, self.colours()),
            row_properties: Arc::from(std::mem::take(&mut built.row_properties)),
            cell_properties: Arc::from(std::mem::take(&mut built.cell_properties)),
            look,
            conditions: Arc::from(std::mem::take(&mut built.conditions)),
            cell_places: Arc::from(std::mem::take(&mut built.cell_places)),
            joined: false,
        };
        split(table, built.moved, &grid_of)
    }
}

/// A row's or cell's place as a count (a table of more than `u32::MAX` rows is not read).
fn count(k: usize) -> u32 {
    u32::try_from(k).unwrap_or(u32::MAX)
}

fn part_of(table: &Table, start: usize, end: usize, grid: Arc<[i64]>) -> Table {
    fn slice<T: Clone>(items: &[T], start: usize, end: usize) -> Arc<[T]> {
        Arc::from(items.get(start..end).unwrap_or_default().to_vec())
    }
    Table {
        rows: slice(&table.rows, start, end),
        grid,
        row_properties: slice(&table.row_properties, start, end),
        cell_properties: slice(&table.cell_properties, start, end),
        conditions: slice(&table.conditions, start, end),
        cell_places: slice(&table.cell_places, start, end),
        joined: true,
        ..table.clone()
    }
}

fn split(table: Table, moved: Vec<(usize, Vec<Block>)>, grid_of: &dyn Fn(usize, usize) -> Arc<[i64]>) -> Vec<Block> {
    if moved.is_empty() {
        return if table.rows.is_empty() {
            Vec::new()
        } else {
            vec![Block::Table(Arc::new(table))]
        };
    }
    let mut out = Vec::new();
    let mut start = 0;
    for (row, blocks) in moved {
        if row > start {
            out.push(Block::Table(Arc::new(part_of(&table, start, row, grid_of(start, row)))));
        }
        out.extend(blocks);
        start = row;
    }
    let end = table.rows.len();
    out.push(Block::Table(Arc::new(part_of(&table, start, end, grid_of(start, end)))));
    out
}
