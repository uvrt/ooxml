//! Owns: the columns of an autofit table sized from its cells' content
//! ([`autofit_widths`], docx2svg's `table.autofit_widths` and `column_wide`): a column as
//! wide as its cells' width in `dxa` or its widest content, never narrower than its
//! narrowest; grown to a stated table width, or giving way to the room -- each rule as
//! `make_autofit_width_probe.py` and `make_autofit_over_probe.py` measured it.
//!
//! Must not: measure content (the extents are given).

use super::grid::{left_offset, right_offset, TableSpec};
use super::widths::{autofit_room, column_widths, dxa_width, edges, floor_twips, nearest, percent_of, Extents};
use crate::table::Unsupported;
use ooxml_base::Q;
use ooxml_word_model::resolve::ResolvedCell;
use ooxml_word_model::value::Value;

fn unsupported(why: &str) -> Unsupported {
    Unsupported(why.to_owned())
}

fn int(x: i64) -> Q {
    Q::from_integer(i128::from(x))
}

fn zero() -> Q {
    Q::from_integer(0)
}

/// What each grid column asks for.
struct Columns {
    lows: Vec<Q>,
    offsets: Vec<Q>,
    highs: Vec<Option<Q>>,
    dxas: Vec<Option<i64>>,
    /// Cells across columns: the cell, its narrowest and widest, its width in `dxa`.
    spanned: Vec<(i64, i64, Q, Q, Option<i64>)>,
}

/// The table's stated width as a target, twips: in `dxa`, or in percent of its whole.
fn target_of(spec: &TableSpec, column_width: Q, room: Q) -> Option<i64> {
    match spec.width() {
        Some((w, "dxa")) if w > 0 => Some(w),
        Some((w, "pct")) if w > 0 => {
            let (first, last) = spec.outer_cells();
            let whole = if spec.placed15() {
                room
            } else {
                column_width + int(first.map_or(0, left_offset) + last.map_or(0, right_offset))
            };
            Some(percent_of(whole, w))
        }
        _ => None,
    }
}

fn cells(spec: &TableSpec) -> Vec<&ResolvedCell> {
    spec.rows()
        .iter()
        .flat_map(|r| r.cells.iter().map(|c| &**c))
        .filter(|c| c.merge.as_deref() != Some("continue"))
        .collect()
}

fn extent(extents: &Extents, cell: &ResolvedCell) -> (Q, Q) {
    extents.get(&(cell.row, cell.index)).copied().unwrap_or_default()
}

/// Whether every cell states a width in `dxa` that holds its narrowest content.
fn stated_hold(cells: &[&ResolvedCell], extents: &Extents) -> Result<bool, Unsupported> {
    for cell in cells {
        let Some(width) = dxa_width(cell)? else {
            return Ok(false);
        };
        if extent(extents, cell).0 > int(width) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn columns_of(spec: &TableSpec, cells: &[&ResolvedCell], extents: &Extents) -> Result<Columns, Unsupported> {
    let n = spec.columns();
    let mut out = Columns {
        lows: vec![zero(); n],
        offsets: vec![zero(); n],
        highs: vec![None; n],
        dxas: vec![None; n],
        spanned: Vec::new(),
    };
    for cell in cells {
        let (low, high) = extent(extents, cell);
        let width = dxa_width(cell)?;
        if cell.span != 1 {
            out.spanned.push((cell.column, cell.span, low, high, width));
            continue;
        }
        let c = usize::try_from(cell.column).unwrap_or(usize::MAX);
        let (Some(lows), Some(offsets), Some(highs), Some(dxas)) = (
            out.lows.get_mut(c),
            out.offsets.get_mut(c),
            out.highs.get_mut(c),
            out.dxas.get_mut(c),
        ) else {
            return Err(unsupported(
                "autofit: a row with more cells than the grid (not measured)",
            ));
        };
        if low > *lows {
            *offsets = int(left_offset(cell) + right_offset(cell));
        }
        *lows = (*lows).max(low);
        match width {
            Some(w) => *dxas = Some(dxas.unwrap_or(0).max(w)),
            None => *highs = Some(highs.unwrap_or_default().max(high)),
        }
    }
    Ok(out)
}

/// The columns of an autofit table sized from its content, twips (`autofit_widths`).
pub fn autofit_widths(spec: &TableSpec, extents: &Extents, column_width: Q) -> Result<Vec<i64>, Unsupported> {
    let room = autofit_room(spec, column_width);
    let target = target_of(spec, column_width, room);
    let in_pct = matches!(spec.width(), Some((_, "pct")));
    let cells = cells(spec);
    if target.is_none() && stated_hold(&cells, extents)? {
        let stated = column_widths(spec);
        if int(stated.iter().sum::<i64>()) <= room {
            return Ok(stated);
        }
    }
    if spec.rows().iter().any(|r| r.grid_before != 0 || r.grid_after != 0) {
        return Err(unsupported(
            "autofit: w:gridBefore or w:gridAfter in a table sized by its content (not measured)",
        ));
    }
    let cols = columns_of(spec, &cells, extents)?;
    if cols
        .dxas
        .iter()
        .zip(&cols.highs)
        .any(|(d, h)| d.is_none() && h.is_none())
    {
        return Err(unsupported(
            "autofit: a grid column no cell of its own sizes (not measured)",
        ));
    }
    if cols
        .dxas
        .iter()
        .zip(&cols.highs)
        .any(|(d, h)| d.is_some() && h.is_some())
    {
        return Err(unsupported(
            "autofit: a column of cells in dxa and cells with no width (not measured)",
        ));
    }
    let auto: Vec<bool> = cols.dxas.iter().map(Option::is_none).collect();
    let wants: Vec<Q> = cols
        .dxas
        .iter()
        .zip(&cols.highs)
        .zip(&cols.lows)
        .map(|((d, h), low)| d.map_or(h.unwrap_or_default(), int).max(*low))
        .collect();
    spanned_fit(&cols, &wants)?;
    let total: Q = wants.iter().sum();
    let narrowest: Q = cols.lows.iter().sum();
    let percent_past = target.is_some() && in_pct && !spec.placed15();
    if percent_past && target.is_some_and(|t| narrowest > int(t)) {
        return Err(unsupported(
            "autofit: a table in percent narrower than its content (measured, not settled)",
        ));
    }
    if narrowest > room && cols.spanned.is_empty() && !percent_past {
        return over_room(spec, &cols, room);
    }
    let goal = match target {
        Some(t) => int(t),
        None => total.min(room),
    };
    if let Some(t) = target {
        if int(t) > room && !in_pct && !(spec.placed15() && int(t) <= column_width) {
            return Err(unsupported(
                "autofit: a table width in dxa wider than the room (not measured)",
            ));
        }
    }
    let widths = if goal > total {
        grown(&wants, &auto, goal - total)?
    } else if goal < total {
        given_way(&cells, &cols, &wants, &auto, goal)?
    } else {
        wants.iter().map(|w| w.ceil()).collect()
    };
    Ok(cumulative(&widths, goal > total))
}

/// Columns of `widths`, each grid line at the nearest twip of its cumulative width where
/// the table grew to a stated width, else at its floor.
fn cumulative(widths: &[Q], grew: bool) -> Vec<i64> {
    let mut x = zero();
    let mut previous = 0;
    widths
        .iter()
        .map(|w| {
            x += *w;
            let edge = if grew { nearest(x) } else { floor_twips(x) };
            let width = edge - previous;
            previous = edge;
            width
        })
        .collect()
}

/// A cell across columns wider than they are (measured, not settled) is refused.
fn spanned_fit(cols: &Columns, wants: &[Q]) -> Result<(), Unsupported> {
    for (column, span, low, high, width) in &cols.spanned {
        let need = width.map_or(*high, int).max(*low);
        let from = usize::try_from(*column).unwrap_or(0);
        let across: Q = wants.iter().skip(from).take(usize::try_from(*span).unwrap_or(0)).sum();
        if need > across {
            return Err(unsupported(
                "autofit: a cell across columns wider than they are (measured, not settled)",
            ));
        }
    }
    Ok(())
}

/// Narrower than its content: the table keeps to the room, each column its offsets and a
/// share of the rest in proportion to its narrowest content.
fn over_room(spec: &TableSpec, cols: &Columns, room: Q) -> Result<Vec<i64>, Unsupported> {
    let content: Vec<Q> = cols.lows.iter().zip(&cols.offsets).map(|(l, o)| l - o).collect();
    let offsets: Q = cols.offsets.iter().sum();
    let left = room - offsets;
    let content_sum: Q = content.iter().sum();
    if left <= zero() || content_sum <= zero() {
        return Err(unsupported(
            "autofit: a table whose margins are wider than the room (not measured)",
        ));
    }
    let widths: Vec<Q> = cols
        .offsets
        .iter()
        .zip(&content)
        .map(|(o, part)| o + left * part / content_sum)
        .collect();
    Ok(edges(&widths, !spec.placed15()))
}

/// The columns grown to `extra` more: those with no width in proportion to their wants,
/// or all of them.
fn grown(wants: &[Q], auto: &[bool], extra: Q) -> Result<Vec<Q>, Unsupported> {
    let grow: Vec<bool> = if auto.iter().any(|a| *a) {
        auto.to_vec()
    } else {
        vec![true; wants.len()]
    };
    let base: Q = wants.iter().zip(&grow).filter(|(_, g)| **g).map(|(w, _)| *w).sum();
    if base <= zero() {
        return Err(unsupported("autofit: a table with no content to widen (not measured)"));
    }
    Ok(wants
        .iter()
        .zip(&grow)
        .map(|(w, g)| if *g { w + extra * w / base } else { *w })
        .collect())
}

/// The columns giving way to `goal`: those with no width, or all, in proportion to their
/// wants less their narrowest.
fn given_way(
    cells: &[&ResolvedCell],
    cols: &Columns,
    wants: &[Q],
    auto: &[bool],
    goal: Q,
) -> Result<Vec<Q>, Unsupported> {
    let no_wrap = cells.iter().any(|cell| {
        cell.span == 1
            && cell.properties.get("noWrap").is_some_and(Value::truthy)
            && usize::try_from(cell.column)
                .ok()
                .and_then(|c| auto.get(c))
                .copied()
                .unwrap_or(false)
    });
    if no_wrap {
        return Err(unsupported(
            "autofit: a cell with w:noWrap and no width giving way (not measured)",
        ));
    }
    let any_auto = auto.iter().any(|a| *a);
    let fixed_wants: Q = wants.iter().zip(auto).filter(|(_, a)| !**a).map(|(w, _)| *w).sum();
    let auto_lows: Q = cols.lows.iter().zip(auto).filter(|(_, a)| **a).map(|(l, _)| *l).sum();
    let share: Vec<bool> = if any_auto && fixed_wants + auto_lows <= goal {
        auto.to_vec()
    } else if !any_auto {
        vec![true; wants.len()]
    } else {
        return Err(unsupported(
            "autofit: columns in dxa giving way beside columns at their narrowest (not measured)",
        ));
    };
    let kept: Q = wants.iter().zip(&share).filter(|(_, s)| !**s).map(|(w, _)| *w).sum();
    let shared_lows: Q = cols.lows.iter().zip(&share).filter(|(_, s)| **s).map(|(l, _)| *l).sum();
    let spare = goal - kept - shared_lows;
    let slack: Q = wants
        .iter()
        .zip(&cols.lows)
        .zip(&share)
        .filter(|(_, s)| **s)
        .map(|((w, l), _)| w - l)
        .sum();
    if spare < zero() || slack <= zero() {
        return Err(unsupported("autofit: a table narrower than its content (not measured)"));
    }
    Ok(wants
        .iter()
        .zip(&cols.lows)
        .zip(&share)
        .map(|((w, l), s)| if *s { l + spare * (w - l) / slack } else { *w })
        .collect())
}
