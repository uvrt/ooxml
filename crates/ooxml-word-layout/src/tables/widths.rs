//! Owns: a table's grid columns' widths, twips, as Word sizes them
//! ([`table_widths`]): as authored, or on the cells' stated widths ([`column_widths`]);
//! a fixed table made as wide as its stated width; a table in percent shared out; an
//! autofit table sized from its cells' content -- docx2svg's `table.column_widths` to
//! `table._edges`, each rule as measured there (ROADMAP.md, "Tables -- measured").
//!
//! Must not: measure content (the extents are given: `super::content`), or place the
//! grid (`super::grid`).

use super::grid::{cell_width, left_offset, outer_part, right_offset, TableSpec};
use crate::table::Unsupported;
use ooxml_base::Q;
use ooxml_word_model::value::Value;
use std::collections::{BTreeSet, HashMap};

/// `(row, cell index) -> (narrowest, widest)` content of every cell not merged into the
/// one above, twips.
pub type Extents = HashMap<(usize, usize), (Q, Q)>;

/// `w:tblW` in `pct`: fiftieths of a percent.
const PERCENT_WHOLE: i64 = 5000;
/// Twips each column of a fixed table keeps besides its cells' margins when its columns
/// are made as wide as the table.
const FIXED_KEPT: i64 = 20;

fn unsupported(why: &str) -> Unsupported {
    Unsupported(why.to_owned())
}

fn int(x: i64) -> Q {
    Q::from_integer(i128::from(x))
}

fn floor(x: Q) -> i64 {
    i64::try_from(x.floor().to_integer()).unwrap_or(0)
}

fn round_half_up(x: Q) -> i64 {
    floor(x + Q::new(1, 2))
}

/// Columns of cumulative edges.
fn from_edges(edges: &[i64]) -> Vec<i64> {
    let mut previous = 0;
    edges
        .iter()
        .map(|e| {
            let w = e - previous;
            previous = *e;
            w
        })
        .collect()
}

/// Columns of `widths`, each grid line at the nearest twip of its cumulative width, or at
/// its floor (`_edges`).
fn edges_of(widths: &[Q], nearest: bool) -> Vec<i64> {
    let mut x = Q::from_integer(0);
    let edges: Vec<i64> = widths
        .iter()
        .map(|w| {
            x += *w;
            if nearest {
                round_half_up(x)
            } else {
                floor(x)
            }
        })
        .collect();
    from_edges(&edges)
}

/// The grid columns' widths: `w:tblGrid` as authored, unless every cell of every row
/// states its width in `dxa` and those put the grid lines elsewhere (`column_widths`).
pub fn column_widths(spec: &TableSpec) -> Vec<i64> {
    let grid = &spec.resolved.grid;
    let mut lines: Option<BTreeSet<i64>> = None;
    for row in spec.rows() {
        let skip = usize::try_from(row.grid_before).unwrap_or(0);
        let mut x: i64 = grid.iter().take(skip).sum();
        let mut edges = BTreeSet::new();
        for cell in &row.cells {
            match cell_width(cell) {
                Some((w, "dxa")) if w > 0 => {
                    x += w;
                    edges.insert(x);
                }
                _ => return grid.clone(),
            }
        }
        lines = Some(match lines {
            Some(mut all) => {
                all.extend(edges);
                all
            }
            None => edges,
        });
    }
    let Some(lines) = lines.filter(|_| !spec.rows().is_empty()) else {
        return grid.clone();
    };
    let ordered: Vec<i64> = lines.into_iter().collect();
    let derived = from_edges(&ordered);
    if derived.len() != grid.len() || spec.rows().iter().any(|r| r.grid_before != 0 || r.grid_after != 0) {
        return grid.clone();
    }
    derived
}

/// The whole a width in percent is of: below mode 15 the column and the first and last
/// cells' outer offsets, in mode 15 (and nested) the column less the outer borders'
/// outer halves.
fn percent_whole(spec: &TableSpec, column_width: Q) -> Q {
    let (first, last) = spec.outer_cells();
    if spec.placed15() {
        column_width
            - int(first.map_or(0, |c| outer_part(c.left_edge.0.as_ref())))
            - int(last.map_or(0, |c| outer_part(c.right_edge.0.as_ref())))
    } else {
        column_width + int(first.map_or(0, left_offset) + last.map_or(0, right_offset))
    }
}

/// `whole * part // 5000` (Python's floor division of an exact length).
pub fn percent_of(whole: Q, part: i64) -> i64 {
    floor(whole * int(part) / int(PERCENT_WHOLE))
}

/// The columns of a table whose width is stated in percent (`percent_widths`).
fn percent_widths(spec: &TableSpec, column_width: Q) -> Result<Vec<i64>, Unsupported> {
    let rows = spec.rows();
    let first_row = rows.first().filter(|r| !r.cells.is_empty());
    let table_width = spec.width().map_or(0, |(w, _)| w);
    let Some(first_row) = first_row.filter(|_| table_width > 0) else {
        return Err(unsupported("a table width in percent that is not a number"));
    };
    if spec.fixed() {
        return Err(unsupported("a fixed table with a width in percent (not measured)"));
    }
    let shares: Vec<Option<(i64, &str)>> = first_row.cells.iter().map(|c| cell_width(c)).collect();
    let kinds: BTreeSet<Option<&str>> = shares.iter().map(|s| s.map(|(_, k)| k)).collect();
    let dxa = kinds.len() == 1 && kinds.contains(&Some("dxa"));
    let pct = kinds.len() == 1 && kinds.contains(&Some("pct"));
    if dxa && spec.minimums.is_none() {
        return Err(unsupported(
            "autofit: a table in percent whose cells' widths are in twips, shared by their content",
        ));
    }
    if !(dxa || pct) || shares.iter().any(|s| s.is_none_or(|(w, _)| w <= 0)) {
        return Err(unsupported(
            "a table in percent whose cells' widths are not all in percent",
        ));
    }
    let columns = spec.columns();
    for row in rows {
        let widths: Vec<Option<(i64, &str)>> = row.cells.iter().map(|c| cell_width(c)).collect();
        if row.grid_before != 0
            || row.grid_after != 0
            || row.cells.len() != columns
            || row.cells.iter().any(|c| c.span != 1)
            || widths != shares
        {
            return Err(unsupported(
                "a table in percent whose rows differ, or with a cell across columns (not measured)",
            ));
        }
    }
    let whole = percent_whole(spec, column_width);
    let total = percent_of(whole, table_width);
    let values: Vec<i64> = shares.iter().flatten().map(|(w, _)| *w).collect();
    if dxa {
        return dxa_shares(&values, total, spec.minimums.as_deref());
    }
    let mut cumulative = Vec::with_capacity(values.len());
    let mut sum = 0;
    for v in &values {
        sum += v;
        cumulative.push(sum);
    }
    let edges: Vec<i64> = if sum > table_width {
        let mut e: Vec<i64> = cumulative
            .iter()
            .take(cumulative.len().saturating_sub(1))
            .map(|c| percent_of(whole, *c))
            .collect();
        e.push(total);
        e
    } else {
        cumulative.iter().map(|c| (total * c).div_euclid(sum)).collect()
    };
    Ok(from_edges(&edges))
}

/// The columns of a table in percent whose cells state their widths in `dxa`, sharing
/// `total` twips (`dxa_shares`).
fn dxa_shares(preferred: &[i64], total: i64, minimums: Option<&[Q]>) -> Result<Vec<i64>, Unsupported> {
    let Some(minimums) = minimums.filter(|m| m.len() == preferred.len()) else {
        return Err(unsupported(
            "autofit: a table in percent whose cells' widths are in twips, shared by their content",
        ));
    };
    let wanted: i64 = preferred.iter().sum();
    let shares: Vec<Q> = if wanted <= total {
        preferred
            .iter()
            .map(|p| Q::new(i128::from(total * p), i128::from(wanted)))
            .collect()
    } else {
        let room: Vec<Q> = preferred.iter().zip(minimums).map(|(p, m)| int(*p) - m).collect();
        let room_sum: Q = room.iter().sum();
        let minimum_sum: Q = minimums.iter().sum();
        if room.iter().any(|r| *r < Q::from_integer(0)) || room_sum <= Q::from_integer(0) || minimum_sum > int(total) {
            return Err(unsupported(
                "autofit: a table in percent narrower than its content (not measured)",
            ));
        }
        let excess = int(wanted - total);
        preferred
            .iter()
            .zip(&room)
            .map(|(p, r)| int(*p) - excess * r / room_sum)
            .collect()
    };
    let mut x = Q::from_integer(0);
    let mut edges: Vec<i64> = shares
        .iter()
        .map(|s| {
            x += *s;
            floor(x)
        })
        .collect();
    if let Some(last) = edges.last_mut() {
        *last = total;
    }
    Ok(from_edges(&edges))
}

/// The columns of a fixed table, made as wide as its stated width (`fixed_widths`).
fn fixed_widths(spec: &TableSpec, column_width: Q) -> Vec<i64> {
    let widths = column_widths(spec);
    let Some((stated, kind)) = spec.width().filter(|(w, _)| *w > 0) else {
        return widths;
    };
    if !spec.fixed() || widths.is_empty() {
        return widths;
    }
    let target = match kind {
        "dxa" => stated,
        "pct" => percent_of(percent_whole(spec, column_width), stated),
        _ => return widths,
    };
    if target == widths.iter().sum::<i64>() {
        return widths;
    }
    let mut kept = vec![0; widths.len()];
    for row in spec.rows() {
        for cell in &row.cells {
            let column = usize::try_from(cell.column).ok();
            if let Some(slot) = column.and_then(|c| kept.get_mut(c)) {
                if cell.span == 1 && *slot == 0 {
                    *slot = cell.margins[1] + cell.margins[3] + FIXED_KEPT;
                }
            }
        }
    }
    let mut content: Vec<i64> = widths.iter().zip(&kept).map(|(w, k)| w - k).collect();
    let mut room = target - kept.iter().sum::<i64>();
    if content.iter().any(|p| *p < 0) || content.iter().sum::<i64>() <= 0 || room <= 0 {
        kept = vec![0; widths.len()];
        content.clone_from(&widths);
        room = target;
    }
    let content_sum: i64 = content.iter().sum();
    if content_sum == 0 {
        return widths;
    }
    let mut x = Q::from_integer(0);
    let mut edges: Vec<i64> = kept
        .iter()
        .zip(&content)
        .map(|(k, part)| {
            x += int(*k) + Q::new(i128::from(*part) * i128::from(room), i128::from(content_sum));
            floor(x)
        })
        .collect();
    if let Some(last) = edges.last_mut() {
        *last = target;
    }
    from_edges(&edges)
}

/// The widest an autofit table goes, twips (`autofit_room`).
pub fn autofit_room(spec: &TableSpec, column_width: Q) -> Q {
    let (first, last) = spec.outer_cells();
    if spec.placed15() {
        column_width
            - int(first.map_or(0, |c| outer_part(c.left_edge.0.as_ref())))
            - int(last.map_or(0, |c| outer_part(c.right_edge.0.as_ref())))
    } else {
        column_width + int(last.map_or(0, right_offset))
    }
}

/// The grid columns' widths, twips (`table_widths`).
pub fn table_widths(spec: &TableSpec, column_width: Q) -> Result<Vec<i64>, Unsupported> {
    if let Some(extents) = &spec.extents {
        return super::autofit::autofit_widths(spec, extents, column_width);
    }
    if spec.in_percent() && !spec.fixed() {
        return percent_widths(spec, column_width);
    }
    Ok(fixed_widths(spec, column_width))
}

/// What of an autofit table Word would size from its content that the model, laying it
/// out on its stated widths, does not (`autofit_unmodelled`).
pub fn autofit_unmodelled(spec: &TableSpec) -> Option<&'static str> {
    if spec.in_percent() {
        return None;
    }
    let without = spec
        .rows()
        .iter()
        .flat_map(|r| &r.cells)
        .any(|cell| cell.merge.as_deref() != Some("continue") && !matches!(cell_width(cell), Some((_, "dxa"))));
    without.then_some("autofit: a cell with no width in twips (w:tcW), sized to its content")
}

/// Columns of exact widths, each grid line at the nearest twip or the floor of its
/// cumulative width.
pub fn edges(widths: &[Q], nearest: bool) -> Vec<i64> {
    edges_of(widths, nearest)
}

/// A cell's `w:tcW` in `dxa`; `Unsupported` for one in percent with a value.
pub fn dxa_width(cell: &ooxml_word_model::resolve::ResolvedCell) -> Result<Option<i64>, Unsupported> {
    match cell.properties.get("tcW") {
        Some(Value::Width(w, kind)) if &**kind == "dxa" && *w > 0 => Ok(Some(*w)),
        Some(Value::Width(w, kind)) if &**kind == "pct" && *w != 0 => Err(unsupported(
            "autofit: a cell width in percent in a table not in percent (not measured)",
        )),
        _ => Ok(None),
    }
}

/// `round_half_up` of exact twips.
pub fn nearest(x: Q) -> i64 {
    round_half_up(x)
}

/// `floor` of exact twips.
pub fn floor_twips(x: Q) -> i64 {
    floor(x)
}
