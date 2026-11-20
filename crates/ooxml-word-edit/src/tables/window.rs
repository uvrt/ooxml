//! Owns: Word's AutoFit to Window as docx-agent's `fit_table_to_window` writes it (round 48,
//! measured with Word 16.106): the room the table fits ([`window_room`], `_window_room`:
//! its cell's text width, nested, else its section's column), the grid that width in
//! percent comes to ([`write`] through `_window_plan`: in mode 15 the room less the outer
//! borders' outer halves, below it the room and the first and last cells' text offsets;
//! each line at the floor of its cumulative share), and what is written: `w:tblW` in
//! fiftieths of a percent, no indent (`w:tblInd w:w="0"` where its style indents it), the
//! layout autofit, every cell's `w:tcW` in percent, the grid.
//!
//! Must not: write but on the table's work.

use super::props::{drop_all, ensure};
use super::shape::{grid_widths, own_rows, row_cells, set_grid};
use super::Found;
use crate::names::W;
use crate::review::xml::{insert_in_order, make};
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::docx::cells::TableFacts;
use ooxml_word_model::docx::formatting::Formatting;
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_word_model::resolve::{border_twips, resolve_table, ResolvedCell};
use ooxml_word_model::styles::StyleKind;
use ooxml_word_model::value::Value;
use ooxml_xml::Element;
use std::sync::Arc;

/// The width (twips) AutoFit to Window fits a table to, and whether it is nested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Room {
    pub(crate) twips: i64,
    pub(crate) nested: bool,
}

fn inner_part(border: Option<&ooxml_word_model::value::Border>) -> i64 {
    border_twips(border) / 2
}

fn outer_part(border: Option<&ooxml_word_model::value::Border>) -> i64 {
    (border_twips(border) + 1) / 2
}

/// Twips from a cell's left grid line to its text (docx2svg's `left_offset`).
fn left_offset(cell: &ResolvedCell) -> i64 {
    cell.margins[1].max(inner_part(cell.borders[1].as_ref()))
}

/// Twips from where a cell's text may end to its right grid line (`right_offset`).
fn right_offset(cell: &ResolvedCell) -> i64 {
    cell.margins[3].max(inner_part(cell.right_edge.0.as_ref()))
}

/// The layout model's reading of the `w:tbl` `table`, resolved.
fn resolved(
    formatting: &Formatting,
    table: &Element,
) -> Option<(
    Arc<ooxml_word_model::model::Table>,
    ooxml_word_model::resolve::ResolvedTable,
)> {
    let facts = TableFacts::read(formatting, table);
    let model = Arc::clone(facts.tables().first()?);
    let resolved = resolve_table(&formatting.sources(), &model);
    Some((model, resolved))
}

/// The steps from `root` down to `node` in `work`.
fn steps(work: &Work, root: Id, node: Id) -> Vec<u32> {
    let mut out = Vec::new();
    let mut at = node;
    while at != root {
        let (Some(parent), Some(k)) = (work.parent(at), work.index(at)) else {
            return Vec::new();
        };
        out.push(u32::try_from(k).unwrap_or(u32::MAX));
        at = parent;
    }
    out.reverse();
    out
}

/// docx-agent's `_window_room` for the table at `found`; `room`, its `_room_for`.
pub(crate) fn window_room(doc: &WordDoc, docx: &Docx, found: &Found, room: i64) -> Room {
    let Some(story) = docx.part(&found.part) else {
        return Room {
            twips: room,
            nested: false,
        };
    };
    let root = story.root();
    let at = |n: usize| found.path.get(..n).and_then(|p| at_path(root, p));
    let cell = (1..found.path.len())
        .rev()
        .find(|n| at(*n).is_some_and(|e| e.name().is(Some(W), "tc")));
    if let Some(cell) = cell {
        let outer = (1..cell)
            .rev()
            .find(|n| at(*n).is_some_and(|e| e.name().is(Some(W), "tbl")));
        let twips = outer.and_then(|outer| {
            let element = at(outer)?;
            let formatting = Formatting::read(docx);
            let (_, resolved) = resolved(&formatting, element)?;
            let below = found.path.get(outer..cell)?;
            let work = Work::of(element, Vec::new());
            let grid = grid_widths(&work, work.root());
            for (r, row) in own_rows(&work, work.root()).into_iter().enumerate() {
                for (k, placed) in row_cells(&work, row).into_iter().enumerate() {
                    if steps(&work, work.root(), placed.cell) == below {
                        let cell = resolved.rows.get(r)?.cells.get(k)?;
                        let width: i64 = grid.get(placed.start..placed.start + placed.span)?.iter().sum();
                        return Some(width - left_offset(cell) - right_offset(cell));
                    }
                }
            }
            None
        });
        return Room {
            twips: twips.unwrap_or(room),
            nested: true,
        };
    }
    if found.part == docx.main {
        if let Some(twips) = section_column(doc, docx, &found.id) {
            return Room { twips, nested: false };
        }
    }
    Room {
        twips: room,
        nested: false,
    }
}

/// The text width of the section `id` is in, or one column's in a section of columns.
fn section_column(doc: &WordDoc, docx: &Docx, id: &str) -> Option<i64> {
    let section = crate::sections::find_section(doc, docx, id).ok()?;
    let properties = &section.section.properties;
    let int = |node: Option<&Element>, local: &str| {
        node.and_then(|n| n.attribute(Some(W), local)?.trim().parse::<i64>().ok())
    };
    let size = properties.find(Some(W), "pgSz")?;
    let margins = properties.find(Some(W), "pgMar").map(|m| &**m);
    let text = int(Some(size), "w").unwrap_or(0)
        - int(margins, "left").unwrap_or(0)
        - int(margins, "right").unwrap_or(0)
        - int(margins, "gutter").unwrap_or(0);
    if text == 0 {
        return None;
    }
    let columns = properties.find(Some(W), "cols");
    let listed: Vec<&Arc<Element>> = columns
        .map(|c| c.child_elements().filter(|e| e.name().is(Some(W), "col")).collect())
        .unwrap_or_default();
    if let Some(first) = listed.first() {
        // its width in points, twips again (docx-agent reads them so)
        return Some(int(Some(first), "w").unwrap_or(0));
    }
    let count = int(columns.map(|c| &**c), "num").filter(|n| *n != 0).unwrap_or(1);
    let space = int(columns.map(|c| &**c), "space").unwrap_or(720);
    Some((text - space * (count - 1)).div_euclid(count))
}

/// `part` of `total` as a share of `whole` fiftieths of a percent, rounded half up
/// (docx-agent's `_fiftieths`).
fn fiftieths(whole: i64, part: i64, total: i64) -> i64 {
    if total == 0 {
        return 0;
    }
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    let out = (whole as f64 * part as f64 / total as f64 + 0.5).floor() as i64;
    out
}

/// What a table's style chain indents it by (the last of its styles stating `w:tblInd`).
fn style_indents(formatting: &Formatting, model: &ooxml_word_model::model::Table) -> bool {
    let sources = formatting.sources();
    let chain = sources
        .styles
        .map(|s| s.chain(model.style_id.as_deref(), StyleKind::Table))
        .unwrap_or_default();
    let mut indent = 0;
    for style in &chain {
        if let Some(value) = style.table.get("tblInd") {
            indent = match value {
                Value::Width(w, _) => *w,
                _ => 0,
            };
        }
    }
    indent != 0
}

/// AutoFit to Window on the table `table` of `work` at `pct` fiftieths of a percent, in
/// `room`, mode 15 or not (`modern`), its styles read through `formatting`.
///
/// # Errors
/// A table with no grid of columns.
pub(crate) fn write(
    work: &mut Work,
    table: Id,
    (pct, room, modern): (i64, Room, bool),
    formatting: &Formatting,
    orders: &Orders,
) -> Result<()> {
    let refused = || EditError::Refused("the table has no grid of columns to fit".to_owned());
    let element = work.build_element(table).ok_or_else(refused)?;
    let (model, resolved) = resolved(formatting, &element).ok_or_else(refused)?;
    let mut weights = grid_widths(work, table);
    if weights.is_empty() {
        return Err(refused());
    }
    if weights.iter().sum::<i64>() == 0 {
        weights = vec![1; weights.len()];
    }
    let total_weight: i64 = weights.iter().sum();
    let first_row = resolved.rows.first();
    let first = first_row.and_then(|r| r.cells.first());
    let last = first_row.and_then(|r| r.cells.last());
    let whole = if modern || room.nested {
        room.twips
            - first.map_or(0, |c| outer_part(c.left_edge.0.as_ref()))
            - last.map_or(0, |c| outer_part(c.right_edge.0.as_ref()))
    } else {
        room.twips + first.map_or(0, |c| left_offset(c)) + last.map_or(0, |c| right_offset(c))
    };
    let total = (whole * pct).div_euclid(5000);
    let shares: Vec<i64> = weights.iter().map(|w| fiftieths(pct, *w, total_weight)).collect();
    let cumulative: Vec<i64> = shares
        .iter()
        .scan(0, |sum, s| {
            *sum += s;
            Some(*sum)
        })
        .collect();
    let end = cumulative.last().copied().unwrap_or(0);
    let edges: Vec<i64> = if end > pct {
        let mut edges: Vec<i64> = cumulative
            .get(..cumulative.len() - 1)
            .unwrap_or_default()
            .iter()
            .map(|c| (whole * c).div_euclid(5000))
            .collect();
        edges.push(total);
        edges
    } else {
        cumulative.iter().map(|c| (total * c).div_euclid(end.max(1))).collect()
    };
    let grid: Vec<i64> = edges
        .iter()
        .scan(0, |before, edge| {
            let width = edge - *before;
            *before = *edge;
            Some(width)
        })
        .collect();
    let style_indent = style_indents(formatting, &model);
    let properties = ensure(work, table, "tblPr", orders);
    drop_all(work, properties, "tblW");
    let width = pct.to_string();
    let node = make(work, "tblW", &[("w", width.as_str()), ("type", "pct")]);
    insert_in_order(work, properties, node, orders);
    drop_all(work, properties, "tblInd");
    if style_indent {
        let node = make(work, "tblInd", &[("w", "0"), ("type", "dxa")]);
        insert_in_order(work, properties, node, orders);
    }
    drop_all(work, properties, "tblLayout");
    for row in own_rows(work, table) {
        for placed in row_cells(work, row) {
            let cell_properties = ensure(work, placed.cell, "tcPr", orders);
            drop_all(work, cell_properties, "tcW");
            let spanned: i64 = weights
                .get(placed.start..placed.start + placed.span)
                .unwrap_or_default()
                .iter()
                .sum();
            let share = fiftieths(pct, spanned, total_weight).to_string();
            let node = make(work, "tcW", &[("w", share.as_str()), ("type", "pct")]);
            insert_in_order(work, cell_properties, node, orders);
        }
    }
    set_grid(work, table, &grid, orders);
    Ok(())
}
