//! Owns: a `w:tbl` as the layout's model holds it in a view (the final view: Accept All;
//! measured, docx2svg's ROADMAP "Tables -- measured" stage 7f and "Revisions -- measured"
//! R.3): its rows and cells found as the table's grid finds them (`own_rows`, `row_cells`:
//! row- and cell-level content controls and custom XML seen through), deleted rows and
//! cells left out, the grid as Word rebuilds it from the cells left ([`ViewGrid`]: a removed
//! cell merged into the cell beside it or gone with the cells after it moving left, columns
//! between the cells' edges, spans and `w:gridBefore`/`w:gridAfter` counted in them, a
//! vertical merge the view breaks gone), a removed cell merged into another's properties
//! absorbed by it, a cell whose last mark is deleted taking the next cell in, content moved
//! out of the table before its row; and where each cell is in its table, for the table
//! style's regions ([`cell_conditions`], measured round 58). Names in exactly their
//! namespace (`Ns::W`), transitional or strict.
//!
//! Must not: read a cell's blocks (its [`CellReader`] does), or resolve a table's
//! properties (`crate::resolve::table`).

use super::{merge_heads, own_rows, row_cells, VMerge};
use crate::ids::BlockKey;
use crate::model::{Block, Look, Paragraph, Table};
use crate::props::table_properties;
use crate::theme::ThemeColours;
use crate::value::{text, Key, Props, Text, Value};
use crate::view::{cell_ends, CellEnds, View, ViewGrid};
use crate::w;
use crate::xmlread::{attr_in, attr_on_in, Ns};
use ooxml_xml::Element;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// What reads a table's cells' blocks for [`table_blocks`].
pub(crate) trait CellReader {
    /// The view the table is read in.
    fn view(&self) -> View;
    /// The theme's colours its properties are read with.
    fn colours(&self) -> &ThemeColours;
    /// The blocks of `cell`, its place as written `place` (its row among the table's rows,
    /// its own among the row's cells) and `rel` below the table (child indexes, wrappers
    /// passed through), its paragraphs in the table style `style`'s regions `conditions`,
    /// joined as the view joins a container's blocks.
    fn cell_blocks(
        &mut self,
        cell: &Element,
        place: (u32, u32),
        rel: &[usize],
        style: &Text,
        conditions: &Arc<[Text]>,
    ) -> Vec<Block>;
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

/// A cell's `w:tcPr`.
fn cell_pr(cell: &Element) -> Option<&Element> {
    w::Tc::new(cell).properties().map(w::TcPr::element)
}

/// A cell's properties in `view`: its side of a pending properties change
/// (`View::cell_properties`), in the final view a pending tracked merge as accepting leaves
/// it, and a vertical merge only the other side has (`View::merge_element`).
fn view_cell_properties(view: View, colours: &ThemeColours, cell: &Element) -> Props {
    let props = cell_pr(cell);
    let mut out = table_properties(props.map(|p| view.cell_properties(p)), colours);
    let tracked = props
        .filter(|_| view == View::Final)
        .and_then(|p| w::TcPr::new(p).merge())
        .and_then(w::CellMerge::v_merge);
    match tracked {
        Some("rest") => {
            out.insert(Key::Borrowed("vMerge"), Value::Str(text("restart")));
        }
        Some("cont") => {
            out.insert(Key::Borrowed("vMerge"), Value::Str(text("continue")));
        }
        _ if !out.contains_key("vMerge") => {
            if let Some(merge) = view.merge_element(cell) {
                let value = attr_in(merge, Some(Ns::W), "val")
                    .filter(|v| !v.is_empty())
                    .unwrap_or("continue");
                out.insert(Key::Borrowed("vMerge"), Value::Str(text(value)));
            }
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
        let made = Block::Paragraph(Arc::new(crate::read::joined(last, first)));
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
    let mask =
        attr_in(element, Some(Ns::W), "val").filter(|m| !m.is_empty() && m.chars().all(|c| c.is_ascii_hexdigit()));
    let bits = mask.and_then(|m| i64::from_str_radix(m, 16).ok()).unwrap_or(0);
    let flag = |name: &str, bit: i64| match attr_in(element, Some(Ns::W), name) {
        Some(_) => attr_on_in(element, Some(Ns::W), name),
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
#[must_use]
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
#[must_use]
pub fn leading_header_rows(rows: &[&Element]) -> usize {
    rows.iter()
        .take_while(|row| {
            let flag = w::Tr::new(row).properties().and_then(w::TrPr::header);
            flag.is_some_and(|f| !matches!(f.val(), Some("0" | "false" | "off")))
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
    look: Look,
    rows: usize,
    headers: usize,
    /// The cells continuing a merge that Accept All starts again or leaves alone.
    heads: &'t HashMap<(usize, usize), VMerge>,
    /// The table's grid in the view, its rows those read.
    grid: &'t ViewGrid,
}

/// A row of the table as written: the `w:tr`, its place below the table, and its cells
/// with their places below it.
struct SourceRow {
    row: Arc<Element>,
    place: Vec<u32>,
    cells: Vec<(Arc<Element>, Box<[u32]>)>,
}

/// One row's plan: its cells shown, and their groups.
struct Plan<'a> {
    /// Its place among the table's rows.
    index: usize,
    source: &'a SourceRow,
    shown: Vec<&'a Element>,
    groups: Vec<Vec<usize>>,
}

/// A row's or cell's place as a count (a table of more than `u32::MAX` rows is not read).
fn count(k: usize) -> u32 {
    u32::try_from(k).unwrap_or(u32::MAX)
}

/// Child indexes as `usize`.
fn indexes(place: &[u32]) -> impl Iterator<Item = usize> + '_ {
    place.iter().map(|k| usize::try_from(*k).unwrap_or(usize::MAX))
}

/// A row's cells kept, by their place among its cells, with their properties (a removed
/// cell merged into one beside it absorbed by that cell: [`ViewGrid`]).
fn cells_kept<'a>(
    reader: &dyn CellReader,
    k: usize,
    plan: &Plan<'a>,
    shape: &Shape<'_>,
    kept: &HashSet<*const Element>,
) -> (Vec<(usize, &'a Element)>, Vec<Props>) {
    let (view, colours) = (reader.view(), reader.colours());
    let all = &plan.source.cells;
    let mut cells: Vec<(usize, &Element)> = Vec::new();
    let mut props_of: Vec<Props> = Vec::new();
    let laid = shape.grid.row(k);
    for (j, (cell, _)) in all.iter().enumerate() {
        if !kept.contains(&Arc::as_ptr(cell)) {
            continue;
        }
        cells.push((j, &**cell));
        let mut props = view_cell_properties(view, colours, cell);
        headed(&mut props, shape.heads.get(&(plan.index, j)));
        let merged = laid.and_then(|r| r.cells.iter().find(|c| c.cell == j));
        for part in merged.map(|c| c.parts.as_slice()).unwrap_or_default() {
            let Some((other, _)) = all.get(*part).filter(|_| *part != j) else {
                continue;
            };
            if view.removes_cell(other) {
                absorb(&mut props, &table_properties(cell_pr(other), colours));
            }
        }
        props_of.push(props);
    }
    (cells, props_of)
}

fn row(reader: &mut dyn CellReader, k: usize, plan: &Plan<'_>, shape: &Shape<'_>, built: &mut Built) {
    let style = shape.style;
    let kept: HashSet<*const Element> = plan
        .groups
        .iter()
        .filter_map(|g| g.first())
        .filter_map(|&n| plan.shown.get(n))
        .map(|c| std::ptr::from_ref(*c))
        .collect();
    let all = &plan.source.cells;
    // the row's own cell `j`: its place as written and below the table
    let at = |j: usize| {
        let rel: Vec<usize> = indexes(&plan.source.place)
            .chain(
                all.get(j)
                    .map(|(_, p)| indexes(p).collect::<Vec<_>>())
                    .unwrap_or_default(),
            )
            .collect();
        ((count(plan.index), count(j)), rel)
    };
    let (cells, props_of) = cells_kept(reader, k, plan, shape, &kept);
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
        let (place, rel) = at(*j);
        let mut blocks = reader.cell_blocks(cell, place, &rel, style, conds);
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
                let j = all.iter().position(|(c, _)| std::ptr::eq(&**c, other)).unwrap_or(0);
                let (place, rel) = at(j);
                let taken = reader.cell_blocks(other, place, &rel, style, conds);
                blocks = cell_taken(blocks, taken);
            }
        }
        row_blocks.push(Arc::from(blocks));
    }
    built.rows.push(Arc::from(row_blocks));
    let row_pr = w::Tr::new(&plan.source.row).properties().map(w::TrPr::element);
    built.row_properties.push(table_properties(row_pr, reader.colours()));
    built.cell_properties.push(Arc::from(props_of));
    built.conditions.push(Arc::from(row_conditions));
    built.cell_places.push(Arc::from(places));
}

/// A `w:tbl`'s blocks in `reader`'s view: the table, or its parts with the blocks moved
/// out between them; named `name` (its key in its story, else the path it is named by).
pub(crate) fn table_blocks(
    element: &Element,
    name: (Option<BlockKey>, Option<Text>),
    reader: &mut dyn CellReader,
) -> Vec<Block> {
    let view = reader.view();
    let table = w::Tbl::new(element);
    let tbl_pr = table.properties();
    let style = text(tbl_pr.and_then(w::TblPr::style).and_then(w::Val::val).unwrap_or(""));
    let look = table_look(tbl_pr.and_then(w::TblPr::look).map(w::TblLook::element));
    let kept = |row: &Element| !view.removes_row(row);
    let source: Vec<SourceRow> = own_rows(element)
        .into_iter()
        .map(|(row, place)| SourceRow {
            cells: row_cells(&row),
            row,
            place,
        })
        .collect();
    let all_rows: Vec<&Element> = source.iter().map(|r| &*r.row).collect();
    let heads = merge_heads(&all_rows, &kept, &|cell| view.merge(cell));
    let grid_nodes: Vec<i64> = table
        .grid()
        .map(|g| g.columns().map(|c| c.w().unwrap_or(0)).collect())
        .unwrap_or_default();
    let mut plans = Vec::new();
    for (index, row) in source.iter().enumerate().filter(|(_, r)| kept(&r.row)) {
        let shown: Vec<&Element> = row
            .cells
            .iter()
            .map(|(c, _)| &**c)
            .filter(|c| !view.removes_cell(c))
            .collect();
        let CellEnds { groups, .. } = cell_ends(&shown);
        plans.push(Plan {
            index,
            source: row,
            shown,
            groups,
        });
    }
    let rows_only: Vec<&Element> = plans.iter().map(|p| &*p.source.row).collect();
    let headers = leading_header_rows(&rows_only);
    let grid_of = |start: usize, end: usize| -> ViewGrid {
        let rows: Vec<&Element> = rows_only.get(start..end).unwrap_or_default().to_vec();
        ViewGrid::of(view, &rows, &grid_nodes)
    };
    let whole = grid_of(0, plans.len());
    let mut built = Built::default();
    let shape = Shape {
        style: &style,
        look,
        rows: plans.len(),
        headers,
        heads: &heads,
        grid: &whole,
    };
    for (k, plan) in plans.iter().enumerate() {
        row(reader, k, plan, &shape, &mut built);
    }
    let (key, unkeyed) = name;
    let table = Table {
        style_id: (!style.is_empty()).then(|| style.clone()),
        rows: Arc::from(std::mem::take(&mut built.rows)),
        key,
        unkeyed_cells: unkeyed.clone(),
        unkeyed,
        grid: Arc::from(whole.widths()),
        properties: table_properties(tbl_pr.map(w::TblPr::element), reader.colours()),
        row_properties: Arc::from(std::mem::take(&mut built.row_properties)),
        cell_properties: Arc::from(std::mem::take(&mut built.cell_properties)),
        look,
        conditions: Arc::from(std::mem::take(&mut built.conditions)),
        cell_places: Arc::from(std::mem::take(&mut built.cell_places)),
        joined: false,
    };
    split(table, built.moved, &|start, end| {
        let grid = grid_of(start, end);
        (Arc::from(grid.widths()), grid)
    })
}

/// `table` with each cell's span and each row's `w:gridBefore` and `w:gridAfter` counted in
/// `grid`'s columns, where they differ from the source grid's, and a vertical merge the view
/// breaks gone (`grid` the view's grid of `table`'s rows, row by row).
fn regridded(table: Table, grid: &ViewGrid) -> Table {
    let count = |span: usize| Value::Int(i64::try_from(span).unwrap_or(i64::MAX));
    let set = |props: &mut Props, name: &'static str, value: Value, none: bool| {
        if none {
            props.remove(name);
        } else {
            props.insert(Key::Borrowed(name), value);
        }
    };
    let source = grid.is_source();
    let cell_properties = table
        .cell_properties
        .iter()
        .enumerate()
        .map(|(k, row)| {
            let laid = grid.row(k);
            let cells: Vec<Props> = row
                .iter()
                .enumerate()
                .map(|(n, props)| {
                    let mut props = props.clone();
                    let placed = laid.and_then(|r| r.cells.get(n));
                    if let (false, Some(placed)) = (source, placed) {
                        let span = placed.columns.len();
                        set(&mut props, "gridSpan", count(span), span == 1);
                    }
                    let continues = matches!(props.get("vMerge").and_then(Value::str), Some("continue"));
                    if continues && grid.merge_broken(k, n) {
                        props.remove("vMerge");
                    }
                    props
                })
                .collect();
            Arc::from(cells)
        })
        .collect();
    let row_properties = if source {
        table.row_properties.clone()
    } else {
        table
            .row_properties
            .iter()
            .enumerate()
            .map(|(k, props)| {
                let mut props = props.clone();
                if let Some(laid) = grid.row(k) {
                    for (name, span) in [("gridBefore", laid.before), ("gridAfter", laid.after)] {
                        if props.contains_key(name) {
                            set(&mut props, name, count(span), false);
                        }
                    }
                }
                props
            })
            .collect()
    };
    Table {
        cell_properties,
        row_properties,
        ..table
    }
}

fn part_of(table: &Table, start: usize, end: usize, grid: (Arc<[i64]>, ViewGrid)) -> Table {
    fn slice<T: Clone>(items: &[T], start: usize, end: usize) -> Arc<[T]> {
        Arc::from(items.get(start..end).unwrap_or_default().to_vec())
    }
    let part = Table {
        rows: slice(&table.rows, start, end),
        grid: grid.0,
        row_properties: slice(&table.row_properties, start, end),
        cell_properties: slice(&table.cell_properties, start, end),
        conditions: slice(&table.conditions, start, end),
        cell_places: slice(&table.cell_places, start, end),
        joined: true,
        ..table.clone()
    };
    regridded(part, &grid.1)
}

/// A part's grid: its widths and its columns.
type PartGrid<'g> = &'g dyn Fn(usize, usize) -> (Arc<[i64]>, ViewGrid);

fn split(table: Table, moved: Vec<(usize, Vec<Block>)>, grid_of: PartGrid<'_>) -> Vec<Block> {
    if moved.is_empty() {
        return if table.rows.is_empty() {
            Vec::new()
        } else {
            let (_, grid) = grid_of(0, table.rows.len());
            vec![Block::Table(Arc::new(regridded(table, &grid)))]
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
