//! Owns: a table made to follow its sample table (docx-agent's `follow._follow_table`):
//! its properties but its alt text, the grid's widths where the columns are as many, each
//! cell's properties from the sample's cell over the same grid columns but its merges, and
//! each cell's paragraphs paired with the sample cell's ([`crate::pairing`]); the rows'
//! properties returned to follow after the tracked record (Word does not track them).
//!
//! Must not: change the table's text, rows, columns or merges.

use super::paragraph::follow_paragraph;
use super::take::{take, Styles, Taking};
use super::Log;
use crate::names::W;
use crate::pairing::{self, Signature};
use crate::review::xml::{find_path, insert_in_order, make};
use crate::tables::shape::{grid_widths, own_rows, row_cells, set_grid};
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::Result;
use ooxml_word_model::docx::{style_name, Docx};
use std::collections::BTreeMap;

/// What a table keeps of its own properties.
const KEEP_TABLE: [&str; 3] = ["tblCaption", "tblDescription", "tblPrChange"];
/// What a row keeps of its own.
const KEEP_ROW: [&str; 8] = [
    "ins",
    "del",
    "trPrChange",
    "cnfStyle",
    "gridBefore",
    "gridAfter",
    "wBefore",
    "wAfter",
];
/// What a cell keeps of its own.
const KEEP_CELL: [&str; 8] = [
    "gridSpan",
    "vMerge",
    "hMerge",
    "cellIns",
    "cellDel",
    "cellMerge",
    "tcPrChange",
    "cnfStyle",
];

/// The table properties by the names `word_format_table` and `word_compare_blocks` use.
fn table_name(local: &str) -> &str {
    match local {
        "tblStyle" => "style",
        "tblW" => "width",
        "jc" => "alignment",
        "tblInd" => "indent",
        "tblBorders" => "borders",
        "shd" => "shading",
        "tblLayout" => "layout",
        "tblCellMar" => "cell_margins",
        "tblCellSpacing" => "cell_spacing",
        "tblLook" | "tblStyleRowBandSize" | "tblStyleColBandSize" => "look",
        "tblpPr" | "tblOverlap" => "floating",
        "bidiVisual" => "right_to_left",
        other => other,
    }
}

/// A row's cells by first grid column: the cell, its span and whether it continues a
/// vertical merge.
type Line = BTreeMap<usize, (Id, usize, bool)>;

fn origins(work: &Work, rows: &[Id]) -> Vec<Line> {
    rows.iter()
        .map(|row| {
            row_cells(work, *row)
                .into_iter()
                .map(|placed| {
                    let up = find_path(work, placed.cell, &["tcPr", "vMerge"])
                        .is_some_and(|m| matches!(work.get(m, Some(W), "val"), None | Some("continue")));
                    (placed.start, (placed.cell, placed.span, up))
                })
                .collect()
        })
        .collect()
}

/// A paragraph's signature, its style's name read in `docx`.
fn signature(work: &Work, docx: &Docx, paragraph: Id) -> Signature {
    let id = find_path(work, paragraph, &["pPr", "pStyle"])
        .map(|s| work.get(s, Some(W), "val").map(str::to_owned))
        .unwrap_or_else(|| {
            docx.default_style("paragraph")
                .and_then(|s| s.attribute(Some(W), "styleId"))
                .map(str::to_owned)
        });
    let name = id.map(|id| match docx.style(&id) {
        Some(style) => style_name(style).map(str::to_owned),
        None => Some(id),
    });
    pairing::signature(name.flatten(), &crate::text::paragraph_text(work, paragraph))
}

fn paragraphs_of(work: &Work, cell: Id) -> Vec<Id> {
    work.children(cell)
        .iter()
        .copied()
        .filter(|c| work.is(*c, W, "p"))
        .collect()
}

/// `table`'s, its grid's, cells' and paragraphs' properties from `sample`'s (both in
/// `work`); its rows with the sample row's properties each is to take.
///
/// # Errors
/// A style the sink cannot write.
pub(super) fn follow_table(
    work: &mut Work,
    table: Id,
    sample: Id,
    styles: &mut Styles<'_, '_, '_>,
    log: &mut Log,
    orders: &Orders,
) -> Result<Vec<(Id, Option<Id>)>> {
    let old_style = find_path(work, table, &["tblPr", "tblStyle"]);
    let missing = styles.missing.len();
    let taking = Taking {
        local: "tblPr",
        keep: &KEEP_TABLE,
        style: Some(("tblStyle", "table")),
    };
    let source = work.find(sample, W, "tblPr");
    let (_, mut changed) = take(work, table, source, &taking, styles, orders)?;
    if let (true, Some(old_style), None) = (
        styles.missing.len() > missing,
        old_style,
        find_path(work, table, &["tblPr", "tblStyle"]),
    ) {
        // the sample's style is not here: the copy keeps its own
        let properties = match work.find(table, W, "tblPr") {
            Some(found) => found,
            None => {
                let made = make(work, "tblPr", &[]);
                insert_in_order(work, table, made, orders);
                made
            }
        };
        let kept = work.deep_copy(old_style);
        insert_in_order(work, properties, kept, orders);
        changed.retain(|n| n != "tblStyle");
    }
    for name in changed.iter().map(|n| table_name(n).to_owned()) {
        if !log.table.contains(&name) {
            log.table.push(name);
        }
    }
    let (widths, sample_widths) = (grid_widths(work, table), grid_widths(work, sample));
    let same_columns = widths.len() == sample_widths.len();
    if same_columns && widths != sample_widths {
        set_grid(work, table, &sample_widths, orders);
        log.widths = true;
    }
    let (rows, sample_rows) = (own_rows(work, table), own_rows(work, sample));
    if rows.is_empty() || sample_rows.is_empty() {
        return Ok(Vec::new());
    }
    let header = sample_rows
        .iter()
        .take_while(|row| {
            find_path(work, **row, &["trPr", "tblHeader"])
                .is_some_and(|node| pairing::is_header(work.get(node, Some(W), "val")))
        })
        .count();
    let mapping = pairing::row_map(rows.len(), sample_rows.len(), header);
    if same_columns {
        let (grid, sample_grid) = (origins(work, &rows), origins(work, &sample_rows));
        for (line, r) in grid.iter().zip(&mapping) {
            let Some(sample_line) = sample_grid.get(*r) else {
                continue;
            };
            for (column, (cell, span, up)) in line {
                let Some((sample_cell, _, _)) = sample_line.get(column).filter(|m| m.1 == *span) else {
                    continue; // merged otherwise: compare reports it
                };
                follow_cell(work, (*cell, *up), *sample_cell, styles, log, orders)?;
            }
        }
    }
    Ok(rows
        .iter()
        .zip(&mapping)
        .map(|(row, r)| (*row, sample_rows.get(*r).and_then(|s| work.find(*s, W, "trPr"))))
        .collect())
}

/// A cell's properties from its sample cell's and, unless it continues a vertical merge,
/// its paragraphs from those they pair with.
fn follow_cell(
    work: &mut Work,
    (cell, up): (Id, bool),
    sample_cell: Id,
    styles: &mut Styles<'_, '_, '_>,
    log: &mut Log,
    orders: &Orders,
) -> Result<()> {
    let taking = Taking {
        local: "tcPr",
        keep: &KEEP_CELL,
        style: None,
    };
    let source = work.find(sample_cell, W, "tcPr");
    if !take(work, cell, source, &taking, styles, orders)?.1.is_empty() {
        log.cells += 1;
    }
    if up {
        return Ok(());
    }
    let (paragraphs, sample_paragraphs) = (paragraphs_of(work, cell), paragraphs_of(work, sample_cell));
    if paragraphs.is_empty() || sample_paragraphs.is_empty() {
        return Ok(());
    }
    let here = styles.sink.docx()?;
    let mine: Vec<Signature> = paragraphs.iter().map(|p| signature(work, &here, *p)).collect();
    let there = styles.source.unwrap_or(&*here);
    let theirs: Vec<Signature> = sample_paragraphs.iter().map(|p| signature(work, there, *p)).collect();
    for (i, j) in pairing::align(&mine, &theirs).pairs {
        if let (Some(paragraph), Some(reference)) = (paragraphs.get(i), sample_paragraphs.get(j)) {
            follow_paragraph(work, *paragraph, *reference, styles, log, orders)?;
        }
    }
    Ok(())
}

/// Each row's properties from its sample row's (after the tracked record: Word does not
/// track them); how many changed.
///
/// # Errors
/// A style the sink cannot write (none: rows name no style).
pub(super) fn follow_rows(
    work: &mut Work,
    rows: &[(Id, Option<Id>)],
    styles: &mut Styles<'_, '_, '_>,
    orders: &Orders,
) -> Result<usize> {
    let taking = Taking {
        local: "trPr",
        keep: &KEEP_ROW,
        style: None,
    };
    let mut changed = 0;
    for (row, source) in rows {
        if !take(work, *row, *source, &taking, styles, orders)?.1.is_empty() {
            changed += 1;
        }
    }
    Ok(changed)
}
