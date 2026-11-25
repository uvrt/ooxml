//! Owns: a table as it is laid out across ([`TableSpec`]: its resolved formats with what
//! the layout settles on top -- nested, laid out on its grid, sized from content), a
//! cell's offsets from its grid lines (its margins against its borders' inner halves),
//! where the grid starts in its column and every grid line ([`grid_lines`]), and where a
//! cell's text starts and how much a line of it may fill ([`cell_box`]) -- docx2svg's
//! `table.left_offset` to `table.cell_box`.
//!
//! Must not: size columns (`super::widths` does) or measure text.

use super::widths::{table_widths, Extents};
use crate::px::Px;
use crate::table::Unsupported;
use ooxml_base::Q;
use ooxml_word_model::docx::tables::TABLE_COLUMNS_MAX;
use ooxml_word_model::resolve::{border_twips, ResolvedCell, ResolvedRow, ResolvedTable};
use ooxml_word_model::value::{Border, Props, Value};
use std::sync::Arc;

/// Twips per layout unit's inverse: 4096 / 20 layout units in a twip.
const UNITS_PER_TWIP: (i128, i128) = (1024, 5);

/// A table as it is laid out across.
#[derive(Debug, Clone)]
pub struct TableSpec {
    /// Its resolved formats (the model's).
    pub resolved: Arc<ResolvedTable>,
    /// Its properties, with what the layout settled on top (laid out on its grid).
    pub properties: Props,
    /// Whether any level states `w:tblInd`.
    pub indent_stated: bool,
    /// Nested in a cell: placed as a mode-15 table is, in every mode.
    pub nested: bool,
    /// The document is in compatibility mode 15 or later.
    pub mode15: bool,
    /// Each grid column's narrowest content, twips (a table in percent over cells in `dxa`).
    pub minimums: Option<Vec<Q>>,
    /// Every cell's narrowest and widest content, twips (an autofit table sized by it).
    pub extents: Option<Extents>,
}

impl TableSpec {
    /// The table `resolved` as stated.
    ///
    /// # Errors
    /// [`Unsupported`] for a table a row of which reaches past the widest a table is
    /// fitted to (`ResolvedTable::too_wide`): its grid is not widened to lay it out on.
    pub fn new(resolved: Arc<ResolvedTable>, nested: bool, mode15: bool) -> Result<TableSpec, Unsupported> {
        if let Some((row, reach)) = resolved.too_wide {
            return Err(Unsupported(format!(
                "row {row} reaches {reach} grid columns, past the {} a Word table takes and its w:tblGrid's {}",
                TABLE_COLUMNS_MAX,
                resolved.grid.len()
            )));
        }
        Ok(TableSpec {
            properties: resolved.properties.clone(),
            indent_stated: resolved.indent_stated,
            resolved,
            nested,
            mode15,
            minimums: None,
            extents: None,
        })
    }

    /// Its rows.
    pub fn rows(&self) -> &[ResolvedRow] {
        &self.resolved.rows
    }

    /// How many grid columns it is laid out on.
    pub fn columns(&self) -> usize {
        self.resolved.grid.len()
    }

    /// Placed as a mode-15 table is (mode 15, or nested).
    pub fn placed15(&self) -> bool {
        self.mode15 || self.nested
    }

    /// A property as a string, `None` where unstated or empty (Python's `get(...) or`).
    pub fn text(&self, key: &str) -> Option<&str> {
        self.properties.get(key).and_then(Value::str).filter(|s| !s.is_empty())
    }

    /// `w:tblLayout` is `fixed`.
    pub fn fixed(&self) -> bool {
        self.text("tblLayout") == Some("fixed")
    }

    /// `w:tblW` and its type.
    pub fn width(&self) -> Option<(i64, &str)> {
        match self.properties.get("tblW") {
            Some(Value::Width(w, kind)) => Some((*w, kind)),
            _ => None,
        }
    }

    /// `w:tblW` is in percent.
    pub fn in_percent(&self) -> bool {
        self.width().is_some_and(|(_, kind)| kind == "pct")
    }

    /// The first row's first and last cells.
    pub fn outer_cells(&self) -> (Option<&ResolvedCell>, Option<&ResolvedCell>) {
        let row = self.rows().first();
        (
            row.and_then(|r| r.cells.first()).map(|c| &**c),
            row.and_then(|r| r.cells.last()).map(|c| &**c),
        )
    }

    /// Lay it out on its `w:tblGrid` as a fixed table (`table.on_grid`).
    pub fn on_grid(&mut self) {
        self.extents = None;
        self.minimums = None;
        self.properties.remove("tblW");
        self.properties
            .insert("tblLayout".into(), Value::Str(ooxml_word_model::value::key("fixed")));
    }
}

/// A cell width (`w:tcW`) and its type.
pub fn cell_width(cell: &ResolvedCell) -> Option<(i64, &str)> {
    match cell.properties.get("tcW") {
        Some(Value::Width(w, kind)) => Some((*w, kind)),
        _ => None,
    }
}

/// A cell margin or indent in twips: `dxa` as written, anything else none.
pub fn twips_of(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::Width(w, kind)) if &**kind == "dxa" => *w,
        _ => 0,
    }
}

fn side(borders: &[Option<Border>; 4], k: usize) -> Option<&Border> {
    borders.get(k).and_then(Option::as_ref)
}

/// The part of a border on a cell's side of its grid line, whole twips (`inner_part`).
pub fn inner_part(border: Option<&Border>) -> i64 {
    border_twips(border) / 2
}

/// The part of an outer border beyond its grid line (`outer_part`).
pub fn outer_part(border: Option<&Border>) -> i64 {
    (border_twips(border) + 1) / 2
}

/// Twips from a cell's left grid line to its text (`left_offset`).
pub fn left_offset(cell: &ResolvedCell) -> i64 {
    cell.margins[1].max(inner_part(side(&cell.borders, 1)))
}

/// Twips from a cell's left grid line to where its line budget starts (`budget_left`).
pub fn budget_left(cell: &ResolvedCell) -> i64 {
    cell.margins[1].max(inner_part(cell.left_edge.0.as_ref()))
}

/// Twips from where a cell's text may end to its right grid line (`right_offset`).
pub fn right_offset(cell: &ResolvedCell) -> i64 {
    cell.margins[3].max(inner_part(cell.right_edge.0.as_ref()))
}

fn int(x: i64) -> Q {
    Q::from_integer(i128::from(x))
}

/// Twips from the text column's left edge to the first grid line, given the columns'
/// widths' sum (`grid_origin`).
fn grid_origin(spec: &TableSpec, column_width: Q, width: i64) -> Q {
    let indent = twips_of(spec.properties.get("tblInd"));
    let jc = spec.text("jc").unwrap_or("left");
    let (first, last) = spec.outer_cells();
    let outer_left = first.map_or(0, |c| outer_part(c.left_edge.0.as_ref()));
    let outer_right = last.map_or(0, |c| outer_part(c.right_edge.0.as_ref()));
    let mode15 = spec.placed15();
    match jc {
        "center" | "centre" => {
            if mode15 {
                (column_width - int(width + outer_left + outer_right)) / Q::from_integer(2) + int(outer_left)
            } else {
                (column_width - int(width)) / Q::from_integer(2)
            }
        }
        "right" | "end" => {
            if spec.nested {
                column_width - int(width)
            } else if mode15 {
                column_width - int(outer_right + width)
            } else {
                column_width + int(last.map_or(0, right_offset) - width)
            }
        }
        _ if spec.nested => int(indent.max(0) + outer_left),
        _ if mode15 => int(indent + outer_left),
        _ if spec.indent_stated => int(indent - first.map_or(0, left_offset)),
        _ => Q::from_integer(0),
    }
}

/// Twips from the text column's left edge to every grid line, left to right
/// (`grid_lines`).
pub fn grid_lines(spec: &TableSpec, column_width: Q) -> Result<Vec<Q>, Unsupported> {
    let widths = table_widths(spec, column_width)?;
    let mut x = grid_origin(spec, column_width, widths.iter().sum());
    let mut out = Vec::with_capacity(widths.len() + 1);
    out.push(x);
    for width in widths {
        x += int(width);
        out.push(x);
    }
    Ok(out)
}

/// Grid line `column`, the last where the grid has fewer (`line_at`).
pub fn line_at(lines: &[Q], column: i64) -> Q {
    let last = lines.len().saturating_sub(1);
    let k = usize::try_from(column.max(0)).unwrap_or(0).min(last);
    lines.get(k).copied().unwrap_or_default()
}

/// Where a cell's text goes across.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellBox {
    /// Where its text starts, whole device px from the page's left edge (in the layout
    /// unit).
    pub text_left: Px,
    /// What a line may fill, twips.
    pub budget: i64,
    /// Its left grid line, exact px from the page's left edge.
    pub left: Px,
    /// Its right grid line.
    pub right: Px,
}

/// `vertical.twips_to_px` of an exact twip length: `floor(204.8 t + 1/2)` layout units.
pub fn twips_px(t: Q) -> Px {
    let units = (t * Q::new(UNITS_PER_TWIP.0, UNITS_PER_TWIP.1) + Q::new(1, 2)).floor();
    Px::from(units * Px::layout_unit())
}

/// Where a cell's text starts, layout units from the page's left edge, before it is
/// snapped to a whole device px (`text_left_units`).
fn text_left_units(spec: &TableSpec, page_left: Q, lines: &[Q], cell: &ResolvedCell) -> Q {
    let per_twip = Q::new(UNITS_PER_TWIP.0, UNITS_PER_TWIP.1);
    let offset = line_at(lines, cell.column) + int(left_offset(cell));
    let jc = spec.text("jc").unwrap_or("left");
    let first = spec.rows().first().and_then(|r| r.cells.first());
    if let Some(first) = first.filter(|_| {
        !spec.mode15 && !spec.nested && !spec.indent_stated && !matches!(jc, "center" | "centre" | "right" | "end")
    }) {
        let half = inner_part(side(&first.borders, 1));
        let edge = ((page_left - int(half)) * per_twip).ceil();
        return edge + ((int(half) + offset) * per_twip).floor();
    }
    ((page_left + offset) * per_twip + Q::new(1, 2)).floor()
}

/// The cell's text start (whole device px) and line budget (twips) (`cell_box`).
pub fn cell_box(spec: &TableSpec, page_left: Q, lines: &[Q], cell: &ResolvedCell) -> CellBox {
    let left = line_at(lines, cell.column);
    let right = line_at(lines, cell.column + cell.span);
    let start = text_left_units(spec, page_left, lines, cell);
    let budget = (right - left - int(budget_left(cell)) - int(right_offset(cell))).trunc();
    let whole = Px::from(start * Px::layout_unit()).round_half_up();
    CellBox {
        text_left: Px::int(whole).quantise(),
        budget: i64::try_from(budget.to_integer()).unwrap_or(0),
        left: twips_px(page_left + left),
        right: twips_px(page_left + right),
    }
}
