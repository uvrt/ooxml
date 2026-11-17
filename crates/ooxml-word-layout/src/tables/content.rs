//! Owns: how wide a cell's content goes in Word's autofit, twips -- a paragraph's
//! narrowest (its widest word) and widest (its longest line unbroken), with the indents in
//! front ([`paragraph_extent`], [`paragraph_minimum`]); a cell's with its offsets
//! ([`cell_extent`]); each grid column's narrowest and a nested table's
//! ([`column_minimums`]) -- docx2svg's `table.paragraph_minimum` to `table.table_minimum`.
//!
//! Must not: size columns (`super::widths` and `super::autofit` do) or break lines.

use super::grid::{left_offset, right_offset, TableSpec};
use crate::input::text_paragraph;
use crate::table::{TableContext, Unsupported};
use ooxml_base::Q;
use ooxml_text::{Piece, PieceKind, Units};
use ooxml_word_model::model::{Block, Paragraph, Table};
use ooxml_word_model::resolve::{border_twips, ResolvedCell};
use ooxml_word_model::value::{Props, Value};
use std::sync::Arc;

/// Twips an empty cell's content takes in Word's autofit, beyond its margins.
const EMPTY_CONTENT: i64 = 6;

fn q(units: Units) -> Q {
    let (n, d) = units.parts();
    Q::new(n, d)
}

fn twips(units: Q) -> Q {
    // 4096/20 layout units a twip.
    units * Q::new(5, 1024)
}

fn int(x: i64) -> Q {
    Q::from_integer(i128::from(x))
}

fn prop(pp: &Props, key: &str) -> i64 {
    pp.get(key).and_then(Value::int).unwrap_or(0)
}

/// A paragraph's pieces (no list label) and its resolved properties.
fn pieces(cx: &mut TableContext<'_, '_>, paragraph: &Paragraph) -> Result<(Vec<Piece>, Arc<Props>), Unsupported> {
    let resolved = cx.model.resolve_paragraph(cx.document, paragraph);
    let input = text_paragraph(cx.document, paragraph, &resolved, None, &[]);
    let found = ooxml_text::pieces(cx.faces, &input).map_err(|e| {
        Unsupported(format!(
            "a cell paragraph that cannot be measured ({}: {e})",
            paragraph.path
        ))
    })?;
    Ok((found, Arc::clone(&resolved.paragraph)))
}

/// The narrowest a paragraph's text goes, twips: its widest word and its left and right
/// indents (`paragraph_minimum`).
pub fn paragraph_minimum(cx: &mut TableContext<'_, '_>, paragraph: &Paragraph) -> Result<Q, Unsupported> {
    let (found, pp) = pieces(cx, paragraph)?;
    let (mut widest, mut word) = (Q::from_integer(0), Q::from_integer(0));
    for piece in &found {
        if matches!(piece.kind, PieceKind::Space | PieceKind::Break | PieceKind::Tab) {
            word = Q::from_integer(0);
            continue;
        }
        word += q(piece.width);
        widest = widest.max(word);
    }
    let indents = prop(&pp, "ind.left") + prop(&pp, "ind.right");
    Ok(twips(widest) + int(indents.max(0)))
}

/// The narrowest and widest a paragraph's text goes in Word's autofit, twips
/// (`paragraph_extent`): below mode 15 every word counts from the paragraph's nearest
/// line start, in mode 15 each line from where it starts.
pub fn paragraph_extent(
    cx: &mut TableContext<'_, '_>,
    paragraph: &Paragraph,
    mode15: bool,
) -> Result<(Q, Q), Unsupported> {
    let (found, pp) = pieces(cx, paragraph)?;
    if found
        .iter()
        .any(|p| matches!(p.kind, PieceKind::Tab | PieceKind::Object))
    {
        return Err(Unsupported(
            "autofit: a tab or a drawing in a cell sized by its content (not measured)".into(),
        ));
    }
    let left = prop(&pp, "ind.left");
    let right = prop(&pp, "ind.right");
    let first = prop(&pp, "ind.firstLine") - prop(&pp, "ind.hanging");
    let first_start = if mode15 { left + first } else { left + first.min(0) };
    let rest_start = if mode15 { left } else { first_start };
    let zero = Q::from_integer(0);
    let (mut widest, mut longest) = (zero, zero);
    let (mut word, mut line, mut trailing) = (zero, zero, zero);
    let (mut start, mut first_word, mut first_line) = (first_start, true, true);
    let kinds = found
        .iter()
        .map(|p| (p.kind, q(p.width)))
        .chain(std::iter::once((PieceKind::Break, zero)));
    for (kind, width) in kinds {
        if kind == PieceKind::Break {
            longest = longest.max(int(start) + twips(line - trailing));
            line = zero;
            trailing = zero;
            first_line = false;
        }
        if matches!(kind, PieceKind::Space | PieceKind::Break) {
            if word != zero {
                let from = if first_word { first_start } else { rest_start };
                widest = widest.max(int(from) + twips(word));
                first_word = false;
            }
            word = zero;
            if kind == PieceKind::Space {
                line += width;
                trailing += width;
            }
            start = if first_line { first_start } else { rest_start };
            continue;
        }
        word += width;
        line += width;
        trailing = zero;
    }
    Ok((widest.max(zero) + int(right), longest.max(zero) + int(right)))
}

/// A cell's blocks in the table.
pub fn cell_blocks<'t>(table: &'t Table, cell: &ResolvedCell) -> &'t [Block] {
    table
        .rows
        .get(cell.row)
        .and_then(|row| row.get(cell.index))
        .map_or(&[] as &[Block], |blocks| &**blocks)
}

/// A cell's narrowest and widest content in Word's autofit, twips, with its offsets
/// (`cell_extent`).
pub fn cell_extent(
    cx: &mut TableContext<'_, '_>,
    table: &Table,
    cell: &ResolvedCell,
    mode15: bool,
) -> Result<(Q, Q), Unsupported> {
    let zero = Q::from_integer(0);
    let (mut low, mut high) = (zero, zero);
    for block in cell_blocks(table, cell) {
        let Block::Paragraph(paragraph) = block else {
            return Err(Unsupported(
                "autofit: a table nested in a cell sized by its content (not measured)".into(),
            ));
        };
        let (narrow, wide) = paragraph_extent(cx, paragraph, mode15)?;
        low = low.max(narrow);
        high = high.max(wide);
    }
    if high == zero {
        low = int(EMPTY_CONTENT);
        high = low;
    }
    let offsets = int(left_offset(cell) + right_offset(cell));
    Ok((low + offsets, high + offsets))
}

/// Each grid column's narrowest content, twips, with its cells' offsets
/// (`column_minimums`).
pub fn column_minimums(cx: &mut TableContext<'_, '_>, spec: &TableSpec, table: &Table) -> Result<Vec<Q>, Unsupported> {
    let columns = spec.columns();
    let mut out = vec![Q::from_integer(0); columns];
    for cell in spec.rows().iter().flat_map(|r| &r.cells) {
        if cell.merge.as_deref() == Some("continue") {
            continue;
        }
        let slot = usize::try_from(cell.column).ok().filter(|_| cell.span == 1);
        let Some(slot) = slot.filter(|c| *c < columns) else {
            return Err(Unsupported(
                "autofit: a table in percent with a merged cell, shared by content (not measured)".into(),
            ));
        };
        let mut content = Q::from_integer(0);
        for block in cell_blocks(table, cell) {
            let found = match block {
                Block::Table(inner) => table_minimum(cx, inner)?,
                Block::Paragraph(p) => paragraph_minimum(cx, p)?,
            };
            content = content.max(found);
        }
        let with_offsets = content + int(left_offset(cell) + right_offset(cell));
        if let Some(value) = out.get_mut(slot) {
            *value = (*value).max(with_offsets);
        }
    }
    Ok(out)
}

/// The narrowest a nested table goes, twips: its columns' narrowest and the outer halves
/// of its outer borders (`table_minimum`).
fn table_minimum(cx: &mut TableContext<'_, '_>, table: &Table) -> Result<Q, Unsupported> {
    let resolved = cx.model.resolve_table(cx.document, table);
    let spec = TableSpec::new(resolved, false, false);
    let minimums = column_minimums(cx, &spec, table)?;
    let (first, last) = spec.outer_cells();
    let edges = first.map_or(Q::from_integer(0), |c| {
        Q::new(i128::from(border_twips(c.left_edge.0.as_ref())), 2)
    }) + last.map_or(Q::from_integer(0), |c| {
        Q::new(i128::from(border_twips(c.right_edge.0.as_ref())), 2)
    });
    Ok(minimums.iter().sum::<Q>() + edges)
}
