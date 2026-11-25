//! Owns: a table's, row's, column's and cell's properties written on an editable copy of
//! the table as docx-agent's `edit/table_format.py` writes them (Word 16.106's forms): the
//! table's style, width (the grid shared again), alignment (on every row too), style
//! options, borders and shading (`_apply_table`); header rows; a row's height and whether
//! it may break (`set_row`); a grid column's width with its cells' (`set_column_width`);
//! a cell's shading, width, vertical alignment and borders, an edge written on both its
//! cells (`set_cell`).
//!
//! Must not: record revisions (the tracker does), resolve styles, or read outside the
//! table.

use super::shape::{grid_widths, neighbours, own_rows, row_cells, set_grid, share};
use crate::names::W;
use crate::review::xml::{find_path, insert_in_order, int, make};
use crate::units::Twips;
use crate::validate::Orders;
use crate::work::{Id, Work};

/// A table's width as set: a length, or a percent of the room it sits in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TableWidth {
    /// A length.
    Length(Twips),
    /// A percent.
    Percent(f64),
}

/// The six style options, as `w:tblLook` writes them: first row, last row, first column,
/// last column, banded rows, banded columns.
pub type Look = [bool; 6];

/// `w:tblLook`'s flags: the attribute, its bit in `w:val`, and whether it says "no".
const LOOK: [(&str, u32, bool); 6] = [
    ("firstRow", 0x0020, false),
    ("lastRow", 0x0040, false),
    ("firstColumn", 0x0080, false),
    ("lastColumn", 0x0100, false),
    ("noHBand", 0x0200, true),
    ("noVBand", 0x0400, true),
];

/// The table's sides, as `w:tblBorders` names them.
const TABLE_SIDES: [&str; 6] = ["top", "bottom", "left", "right", "insideH", "insideV"];
/// A cell's sides with the side across each edge: top, bottom, left, right.
const CELL_SIDES: [(&str, &str); 4] = [
    ("top", "bottom"),
    ("bottom", "top"),
    ("left", "right"),
    ("right", "left"),
];

/// The child `w:{local}` of `parent`, made in schema order where missing.
pub(crate) fn ensure(work: &mut Work, parent: Id, local: &str, orders: &Orders) -> Id {
    if let Some(found) = work.find(parent, W, local) {
        return found;
    }
    let node = make(work, local, &[]);
    insert_in_order(work, parent, node, orders);
    node
}

/// Remove every child `w:{local}` of `parent`.
pub(crate) fn drop_all(work: &mut Work, parent: Id, local: &str) {
    for node in work.children(parent).to_vec() {
        if work.is(node, W, local) {
            work.remove(node);
        }
    }
}

/// A `w:{local}` border of `style` (`none`: `nil`), half a point wide, as `border` writes one.
fn border(work: &mut Work, local: &str, style: &str) -> Id {
    if style == "none" {
        return make(work, local, &[("val", "nil")]);
    }
    make(
        work,
        local,
        &[("val", style), ("sz", "4"), ("space", "0"), ("color", "auto")],
    )
}

/// A clear `w:shd` of `fill`.
fn shading(work: &mut Work, fill: &str) -> Id {
    make(work, "shd", &[("val", "clear"), ("color", "auto"), ("fill", fill)])
}

/// The `w:tblPr` of `table`, made where missing.
fn table_properties(work: &mut Work, table: Id, orders: &Orders) -> Id {
    ensure(work, table, "tblPr", orders)
}

/// A table's properties to set; `None` leaves one as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableValues {
    /// A table style's name.
    pub style: Option<String>,
    /// Its width.
    pub width: Option<TableWidth>,
    /// `left`, `center` or `right`.
    pub alignment: Option<String>,
    /// Its indent from the margin (`w:tblInd`).
    pub indent: Option<Twips>,
    /// How many rows from the top repeat on each page.
    pub header_rows: Option<usize>,
    /// A fill (`Some(None)`: none).
    pub shading: Option<Option<String>>,
    /// Every side's border style (`none`: no border).
    pub borders: Option<String>,
    /// The style options.
    pub look: Option<Look>,
}

/// Write `values` but the header rows into `table` (docx-agent's `_apply_table`), the style
/// as `style_id`; `room` the twips a percent width is of.
pub(crate) fn apply_table(
    work: &mut Work,
    table: Id,
    values: &TableValues,
    style_id: Option<&str>,
    room: i64,
    orders: &Orders,
) {
    let properties = table_properties(work, table, orders);
    if let Some(id) = style_id {
        drop_all(work, properties, "tblStyle");
        let node = make(work, "tblStyle", &[("val", id)]);
        insert_in_order(work, properties, node, orders);
    }
    if let Some(width) = values.width {
        set_width(work, table, properties, width, room, orders);
    }
    if let Some(alignment) = &values.alignment {
        set_alignment(work, table, properties, alignment, orders);
    }
    if let Some(indent) = values.indent {
        drop_all(work, properties, "tblInd");
        let twips = indent.0.to_string();
        let node = make(work, "tblInd", &[("w", twips.as_str()), ("type", "dxa")]);
        insert_in_order(work, properties, node, orders);
    }
    if let Some(look) = values.look {
        drop_all(work, properties, "tblLook");
        let node = look_node(work, look);
        insert_in_order(work, properties, node, orders);
    }
    if let Some(style) = &values.borders {
        let node = ensure(work, properties, "tblBorders", orders);
        for side in TABLE_SIDES {
            drop_all(work, node, side);
            if style != "none" {
                let made = border(work, side, style);
                insert_in_order(work, node, made, orders);
            }
        }
        if work.children(node).is_empty() {
            work.remove(node);
        }
    }
    if let Some(fill) = &values.shading {
        drop_all(work, properties, "shd");
        if let Some(fill) = fill {
            let node = shading(work, fill);
            insert_in_order(work, properties, node, orders);
        }
    }
}

fn set_width(work: &mut Work, table: Id, properties: Id, width: TableWidth, room: i64, orders: &Orders) {
    let (value, kind) = match width {
        #[allow(clippy::cast_possible_truncation)]
        TableWidth::Percent(p) => ((p * 50.0).round_ties_even() as i64, "pct"),
        TableWidth::Length(length) => (length.0, "dxa"),
    };
    drop_all(work, properties, "tblW");
    let node = make(work, "tblW", &[("w", &value.to_string()), ("type", kind)]);
    insert_in_order(work, properties, node, orders);
    let total = if kind == "pct" {
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let total = (((room - 10) * value) as f64 / 5000.0).round_ties_even() as i64;
        total
    } else {
        value
    };
    let widths = share(total, &grid_widths(work, table));
    set_grid(work, table, &widths, orders);
}

fn set_alignment(work: &mut Work, table: Id, properties: Id, alignment: &str, orders: &Orders) {
    drop_all(work, properties, "jc");
    let stated = alignment != "left";
    if stated {
        let node = make(work, "jc", &[("val", alignment)]);
        insert_in_order(work, properties, node, orders);
    }
    // Word writes the alignment on every row too (measured).
    for row in own_rows(work, table) {
        let mut row_properties = work.find(row, W, "trPr");
        if let Some(found) = row_properties {
            drop_all(work, found, "jc");
        }
        if stated {
            let found = match row_properties {
                Some(found) => found,
                None => ensure(work, row, "trPr", orders),
            };
            row_properties = Some(found);
            let node = make(work, "jc", &[("val", alignment)]);
            insert_in_order(work, found, node, orders);
        }
        if let Some(found) = row_properties.filter(|p| work.children(*p).is_empty()) {
            work.remove(found);
        }
    }
}

/// `w:tblLook` saying `look`, its `w:val` the bits of what it says.
fn look_node(work: &mut Work, look: Look) -> Id {
    let mut bits = 0u32;
    let mut attributes: Vec<(&str, &str)> = Vec::new();
    for ((attribute, bit, negated), on) in LOOK.into_iter().zip(look) {
        let flag = if negated { !on } else { on };
        attributes.push((attribute, if flag { "1" } else { "0" }));
        if flag {
            bits |= bit;
        }
    }
    let value = format!("{bits:04X}");
    let mut all = vec![("val", value.as_str())];
    all.extend(attributes);
    make(work, "tblLook", &all)
}

/// The first `count` rows repeat as header rows, the others not (`set_table`'s
/// `header_rows`).
pub(crate) fn set_header_rows(work: &mut Work, table: Id, count: usize, orders: &Orders) {
    for (k, row) in own_rows(work, table).into_iter().enumerate() {
        if k < count {
            let properties = ensure(work, row, "trPr", orders);
            ensure(work, properties, "tblHeader", orders);
        } else if let Some(properties) = work.find(row, W, "trPr") {
            drop_all(work, properties, "tblHeader");
            if work.children(properties).is_empty() {
                work.remove(properties);
            }
        }
    }
}

/// A row's height (`exact` or at least) and whether it may break (`set_row`).
pub(crate) fn set_row(
    work: &mut Work,
    row: Id,
    height: Option<(Twips, bool)>,
    cant_split: Option<bool>,
    orders: &Orders,
) {
    let properties = ensure(work, row, "trPr", orders);
    if let Some((height, exact)) = height {
        drop_all(work, properties, "trHeight");
        let value = height.0.to_string();
        let mut attributes = Vec::new();
        if exact {
            attributes.push(("hRule", "exact"));
        }
        attributes.push(("val", value.as_str()));
        let node = make(work, "trHeight", &attributes);
        insert_in_order(work, properties, node, orders);
    }
    if let Some(on) = cant_split {
        drop_all(work, properties, "cantSplit");
        if on {
            let node = make(work, "cantSplit", &[]);
            insert_in_order(work, properties, node, orders);
        }
    }
    if work.children(properties).is_empty() {
        work.remove(properties);
    }
}

/// A grid column's width (twips) and every cell's over it; a table width in twips by the
/// difference (`set_column_width`).
pub(crate) fn set_column_width(work: &mut Work, table: Id, column: usize, new: i64, orders: &Orders) {
    let mut widths = grid_widths(work, table);
    let Some(slot) = widths.get_mut(column) else { return };
    let difference = new - *slot;
    *slot = new;
    set_grid(work, table, &widths, orders);
    for row in own_rows(work, table) {
        for placed in row_cells(work, row) {
            if !(placed.start <= column && column < placed.start + placed.span) {
                continue;
            }
            let properties = work.find(placed.cell, W, "tcPr");
            let node = properties.and_then(|p| work.find(p, W, "tcW"));
            let kind = node.and_then(|n| work.get(n, Some(W), "type")).map(str::to_owned);
            match (node, kind.as_deref()) {
                (Some(node), None | Some("dxa")) => {
                    let value = (int(work, Some(node), "w", 0) + difference).max(0);
                    crate::review::xml::set_attr(work, node, "w", &value.to_string());
                }
                (None, _) | (Some(_), Some("auto")) => {
                    let properties = match properties {
                        Some(found) => found,
                        None => {
                            let made = make(work, "tcPr", &[]);
                            work.insert(placed.cell, 0, made);
                            made
                        }
                    };
                    drop_all(work, properties, "tcW");
                    let total: i64 = widths.iter().skip(placed.start).take(placed.span).sum();
                    let node = make(work, "tcW", &[("w", &total.to_string()), ("type", "dxa")]);
                    insert_in_order(work, properties, node, orders);
                }
                _ => {}
            }
        }
    }
    if let Some(node) = find_path(work, table, &["tblPr", "tblW"]) {
        if work.get(node, Some(W), "type") == Some("dxa") {
            let value = (int(work, Some(node), "w", 0) + difference).max(0);
            crate::review::xml::set_attr(work, node, "w", &value.to_string());
        }
    }
}

/// A cell's formatting to set; `None` leaves one as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CellValues {
    /// A fill (`Some(None)`: none).
    pub shading: Option<Option<String>>,
    /// Its width.
    pub width: Option<Twips>,
    /// `top`, `center` or `bottom`.
    pub vertical_alignment: Option<String>,
    /// Every side's border style (`none`: no border), on both cells of each edge.
    pub borders: Option<String>,
}

/// Format `cell` of `table` (`set_cell`), then drop the table's empty cell properties.
pub(crate) fn set_cell(work: &mut Work, table: Id, cell: Id, values: &CellValues, orders: &Orders) {
    let properties = match work.find(cell, W, "tcPr") {
        Some(found) => found,
        None => {
            let made = make(work, "tcPr", &[]);
            work.insert(cell, 0, made);
            made
        }
    };
    if let Some(fill) = &values.shading {
        drop_all(work, properties, "shd");
        if let Some(fill) = fill {
            let node = shading(work, fill);
            insert_in_order(work, properties, node, orders);
        }
    }
    if let Some(width) = values.width {
        drop_all(work, properties, "tcW");
        let node = make(work, "tcW", &[("w", &width.0.to_string()), ("type", "dxa")]);
        insert_in_order(work, properties, node, orders);
    }
    if let Some(alignment) = &values.vertical_alignment {
        drop_all(work, properties, "vAlign");
        if alignment != "top" {
            let node = make(work, "vAlign", &[("val", alignment)]);
            insert_in_order(work, properties, node, orders);
        }
    }
    if let Some(style) = &values.borders {
        let around = neighbours(work, table, cell);
        for ((side, opposite), other) in CELL_SIDES.into_iter().zip(around) {
            cell_border(work, cell, side, style, orders);
            if let Some(other) = other {
                cell_border(work, other, opposite, style, orders);
            }
        }
    }
    for node in std::iter::once(cell).chain(work.iter(table, W, &["tc"])) {
        if let Some(found) = work.find(node, W, "tcPr").filter(|p| work.children(*p).is_empty()) {
            work.remove(found);
        }
    }
}

/// One side of a cell's borders (`_set_cell_border`): `none` writes no border there.
fn cell_border(work: &mut Work, cell: Id, side: &str, style: &str, orders: &Orders) {
    let properties = match work.find(cell, W, "tcPr") {
        Some(found) => found,
        None => {
            let made = make(work, "tcPr", &[]);
            work.insert(cell, 0, made);
            made
        }
    };
    let node = match work.find(properties, W, "tcBorders") {
        Some(found) => found,
        None if style == "none" => return,
        None => ensure(work, properties, "tcBorders", orders),
    };
    drop_all(work, node, side);
    if style != "none" {
        let made = border(work, side, style);
        insert_in_order(work, node, made, orders);
    }
    if work.children(node).is_empty() {
        work.remove(node);
    }
}
