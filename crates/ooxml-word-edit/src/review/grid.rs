//! Owns: the tables a review leaves, tidied as docx-agent's review tidies them
//! (`revisions/review.py`: `_tidy`, `_emptied_rows`, `_fix_grid`, `_skip_after`,
//! `_normalise_grid`, `_fits`, `_grid_map`, `_content_kept`, `_cell_before`, `_absorb`;
//! `edit/tables.py`'s `fit_grid`): rows left without a cell or emptied go, grid columns
//! only removed cells covered go, a row ending short skips the rest, a grid set back is
//! rebuilt from its cells, a table that fitted its grid fits it again; then what the
//! review emptied in every paragraph is pruned.
//!
//! Must not: apply records.

use super::edges::cells_of;
use super::part::{GridMap, GridRow, PartReview};
use super::xml::{
    ancestor, attr, digits, find_all, find_path, has_content, insert_in_order, int, keep_markers, make, mark_record,
    prune, set_attr, DELETED,
};
use crate::names::W;
use crate::validate::Orders;
use crate::work::{Id, Work};
use std::collections::{BTreeMap, HashSet};

/// Twips a grid column takes where nothing states its width.
const UNSIZED_COLUMN: i64 = 360;

/// A table's structure changes, tracked: while one is left, its rows may be off its grid.
const STRUCTURE: [&str; 6] = [
    "cellIns",
    "cellDel",
    "cellMerge",
    "tcPrChange",
    "tblGridChange",
    "trPrChange",
];

/// Run content a reader sees.
const SHOWN: [&str; 10] = [
    "t",
    "tab",
    "br",
    "cr",
    "drawing",
    "pict",
    "object",
    "sym",
    "noBreakHyphen",
    "ptab",
];

/// Each row of `table` (its own) with its cells' first grid columns and spans
/// (`_grid_map`: a span or skip read only when all digits).
pub(crate) fn grid_map(work: &Work, table: Id) -> GridMap {
    let mut out = Vec::new();
    for row in work.iter(table, W, &["tr"]) {
        if ancestor(work, row, "tbl") != Some(table) {
            continue;
        }
        let properties = work.find(row, W, "trPr");
        let mut column = digits(work, properties.and_then(|p| work.find(p, W, "gridBefore")), "val").unwrap_or(0);
        let mut cells = Vec::new();
        for cell in work.iter(row, W, &["tc"]) {
            if ancestor(work, cell, "tr") != Some(row) {
                continue;
            }
            let span = digits(work, find_path(work, cell, &["tcPr", "gridSpan"]), "val").unwrap_or(1);
            cells.push((cell, column, span));
            column += span;
        }
        out.push((row, cells));
    }
    out
}

/// Whether a deleted cell keeps anything: content not deleted, or a mark not deleted.
pub(crate) fn content_kept(work: &Work, cell: Id) -> bool {
    for node in work.iter(cell, W, &SHOWN) {
        if work.is(node, W, "t") && work.text(node).is_empty() && work.children(node).is_empty() {
            continue;
        }
        let mut above = work.parent(node);
        let mut deleted = false;
        while let Some(a) = above {
            if work.is_any(a, W, &DELETED) {
                deleted = true;
                break;
            }
            above = work.parent(a);
        }
        if !deleted {
            return true;
        }
    }
    for paragraph in work.iter(cell, W, &["p"]) {
        if ancestor(work, paragraph, "tc") != Some(cell) {
            continue;
        }
        let Some(record) = mark_record(work, paragraph) else {
            return true;
        };
        let holder = work.parent(record);
        let any_deleted = holder.is_some_and(|h| work.children(h).iter().any(|c| work.is_any(*c, W, &DELETED)));
        if !any_deleted {
            return true;
        }
    }
    false
}

/// The cell before `cell` in its row.
pub(crate) fn cell_before(work: &Work, cell: Id) -> Option<Id> {
    let mut node = work.previous(cell);
    while let Some(n) = node {
        if work.is(n, W, "tc") {
            return Some(n);
        }
        node = work.previous(n);
    }
    None
}

/// `before` widened by `cell`'s grid columns and width (`_absorb`).
pub(crate) fn absorb(work: &mut Work, before: Id, cell: Id, orders: &Orders) {
    let span = |work: &Work, of: Id| -> i64 {
        digits(work, find_path(work, of, &["tcPr", "gridSpan"]), "val").map_or(1, |v| v.max(1))
    };
    let total = span(work, before) + span(work, cell);
    let properties = match work.find(before, W, "tcPr") {
        Some(p) => p,
        None => {
            let p = make(work, "tcPr", &[]);
            work.insert(before, 0, p);
            p
        }
    };
    let node = match work.find(properties, W, "gridSpan") {
        Some(n) => n,
        None => {
            let n = make(work, "gridSpan", &[]);
            insert_in_order(work, properties, n, orders);
            n
        }
    };
    set_attr(work, node, "val", &total.to_string());
    let mine = work.find(properties, W, "tcW");
    let theirs = find_path(work, cell, &["tcPr", "tcW"]);
    if let (Some(mine), Some(theirs)) = (mine, theirs) {
        let dxa = |work: &Work, w: Id| matches!(attr(work, w, "type"), None | Some("dxa"));
        if dxa(work, mine) && dxa(work, theirs) {
            if let (Some(a), Some(b)) = (digits(work, Some(mine), "w"), digits(work, Some(theirs), "w")) {
                set_attr(work, mine, "w", &(a + b).to_string());
            }
        }
    }
}

/// Whether every row of `table` reaches exactly across its grid.
fn fits(work: &Work, table: Id) -> bool {
    let count = work
        .find(table, W, "tblGrid")
        .map_or(0, |g| find_all(work, g, "gridCol").len());
    for (row, cells) in grid_map(work, table) {
        let after = find_path(work, row, &["trPr", "gridAfter"]);
        let extra = digits(work, after, "val").unwrap_or(0);
        let reach = cells.last().map_or(0, |(_, start, span)| start + span) + extra;
        if usize::try_from(reach).ok() != Some(count) {
            return false;
        }
    }
    true
}

/// Rebuild a table's grid from its cells' widths (`_normalise_grid`); left as it is where
/// a cell states no width in twips, a row skips grid columns, or the rows disagree.
fn normalise_grid(work: &mut Work, table: Id, orders: &Orders) {
    let Some(grid) = work.find(table, W, "tblGrid") else {
        return;
    };
    let mut extents: Vec<Vec<(Id, i64, i64)>> = Vec::new();
    for (row, cells) in grid_map(work, table) {
        if let Some(properties) = work.find(row, W, "trPr") {
            if work.find(properties, W, "gridBefore").is_some() || work.find(properties, W, "gridAfter").is_some() {
                return;
            }
        }
        let mut x = 0;
        let mut placed = Vec::new();
        for (cell, _, _) in cells {
            let width = find_path(work, cell, &["tcPr", "tcW"]);
            let Some(width) = width.filter(|w| matches!(attr(work, *w, "type"), None | Some("dxa"))) else {
                return;
            };
            let Some(w) = digits(work, Some(width), "w") else {
                return;
            };
            placed.push((cell, x, x + w));
            x += w;
        }
        extents.push(placed);
    }
    let totals: HashSet<i64> = extents.iter().filter_map(|p| p.last().map(|l| l.2)).collect();
    if totals.len() != 1 {
        return;
    }
    let mut lines: Vec<i64> = std::iter::once(0)
        .chain(extents.iter().flatten().flat_map(|(_, a, b)| [*a, *b]))
        .collect();
    lines.sort_unstable();
    lines.dedup();
    let index = |line: i64| lines.iter().position(|l| *l == line).unwrap_or(0);
    for column in find_all(work, grid, "gridCol") {
        work.remove(column);
    }
    for pair in lines.windows(2) {
        if let [a, b] = pair {
            let column = make(work, "gridCol", &[("w", &(b - a).to_string())]);
            insert_in_order(work, grid, column, orders);
        }
    }
    for (cell, a, b) in extents.into_iter().flatten() {
        let Some(properties) = work.find(cell, W, "tcPr") else {
            continue;
        };
        if let Some(node) = work.find(properties, W, "gridSpan") {
            work.remove(node);
        }
        let span = index(b) - index(a);
        if span > 1 {
            let node = make(work, "gridSpan", &[("val", &span.to_string())]);
            insert_in_order(work, properties, node, orders);
        }
    }
}

/// Make `table`'s grid fit its rows as Word writes it (`edit/tables.py`'s `fit_grid`).
fn fit_grid(work: &mut Work, table: Id, orders: &Orders) {
    let grid = match work.find(table, W, "tblGrid") {
        Some(g) => g,
        None => {
            let g = make(work, "tblGrid", &[]);
            insert_in_order(work, table, g, orders);
            g
        }
    };
    let mut widths: Vec<i64> = find_all(work, grid, "gridCol")
        .into_iter()
        .map(|c| attr(work, c, "w").map_or(0, |w| if w.is_empty() { 0 } else { w.trim().parse().unwrap_or(0) }))
        .collect();
    let rows: Vec<Id> = work
        .iter(table, W, &["tr"])
        .into_iter()
        .filter(|r| ancestor(work, *r, "tbl") == Some(table))
        .collect();
    let mut reaches = Vec::new();
    for &row in &rows {
        let properties = work.find(row, W, "trPr");
        let before = int(work, properties.and_then(|p| work.find(p, W, "gridBefore")), "val", 0);
        let after = int(work, properties.and_then(|p| work.find(p, W, "gridAfter")), "val", 0);
        let cells = cells_of(work, row);
        let end = cells.last().map_or(before, |(_, start, span)| start + span);
        reaches.push((row, end, after));
    }
    let Some(longest) = reaches.iter().map(|(_, end, after)| end + after).max() else {
        return;
    };
    let count = |w: &Vec<i64>| i64::try_from(w.len()).unwrap_or(i64::MAX);
    if reaches.iter().all(|(_, end, after)| end + after == count(&widths)) {
        return;
    }
    for &(row, _, _) in &reaches {
        for (cell, start, span) in cells_of(work, row) {
            if start + span <= count(&widths) {
                continue;
            }
            let added = start + span - count(&widths).max(start);
            let width = find_path(work, cell, &["tcPr", "tcW"]);
            let stated = width
                .filter(|w| matches!(attr(work, *w, "type"), None | Some("dxa")))
                .and_then(|w| digits(work, Some(w), "w"))
                .unwrap_or(0);
            let covered: i64 = widths.iter().skip(usize::try_from(start).unwrap_or(0)).sum();
            let rest = if stated > covered {
                stated - covered
            } else {
                UNSIZED_COLUMN * added
            };
            let missing = (start - count(&widths)).max(0);
            widths.extend(std::iter::repeat_n(
                UNSIZED_COLUMN,
                usize::try_from(missing).unwrap_or(0),
            ));
            for k in 0..added {
                widths.push(rest.div_euclid(added) + i64::from(k < rest.rem_euclid(added)));
            }
        }
    }
    let missing = (longest - count(&widths)).max(0);
    widths.extend(std::iter::repeat_n(
        UNSIZED_COLUMN,
        usize::try_from(missing).unwrap_or(0),
    ));
    widths.truncate(usize::try_from(longest).unwrap_or(0));
    for column in find_all(work, grid, "gridCol") {
        work.remove(column);
    }
    for width in &widths {
        let column = make(work, "gridCol", &[("w", &width.to_string())]);
        insert_in_order(work, grid, column, orders);
    }
    for (row, end, after) in reaches {
        if end + after != count(&widths) {
            let skipped = count(&widths) - end;
            let width: i64 = widths.iter().skip(usize::try_from(end).unwrap_or(0)).sum();
            set_after(work, row, skipped, width, orders);
        }
    }
}

/// A row's `w:gridAfter` and `w:wAfter`: `count` columns of `width` twips; none at 0.
fn set_after(work: &mut Work, row: Id, count: i64, width: i64, orders: &Orders) {
    let properties = work.find(row, W, "trPr");
    for local in ["gridAfter", "wAfter"] {
        if let Some(node) = properties.and_then(|p| work.find(p, W, local)) {
            work.remove(node);
        }
    }
    if count <= 0 {
        if let Some(p) = properties.filter(|p| work.children(*p).is_empty()) {
            work.remove(p);
        }
        return;
    }
    let properties = match properties {
        Some(p) => p,
        None => {
            let p = make(work, "trPr", &[]);
            insert_in_order(work, row, p, orders);
            p
        }
    };
    let after = make(work, "gridAfter", &[("val", &count.to_string())]);
    insert_in_order(work, properties, after, orders);
    let w = make(work, "wAfter", &[("w", &width.to_string()), ("type", "dxa")]);
    insert_in_order(work, properties, w, orders);
}

/// A row ending short of its grid skips the rest (`_skip_after`).
fn skip_after(work: &mut Work, row: Id, cells: &[(Id, i64, i64)], widths: &[String], orders: &Orders) {
    let properties = work.find(row, W, "trPr");
    let after = properties.and_then(|p| work.find(p, W, "gridAfter"));
    let mut skipped = digits(work, after, "val").unwrap_or(0);
    let reach = cells.last().map_or(0, |(_, start, span)| start + span) + skipped;
    let count = i64::try_from(widths.len()).unwrap_or(i64::MAX);
    if reach >= count {
        return;
    }
    skipped += count - reach;
    let properties = match properties {
        Some(p) => p,
        None => {
            let p = make(work, "trPr", &[]);
            insert_in_order(work, row, p, orders);
            p
        }
    };
    let after = match after {
        Some(a) => a,
        None => {
            let a = make(work, "gridAfter", &[]);
            insert_in_order(work, properties, a, orders);
            a
        }
    };
    set_attr(work, after, "val", &skipped.to_string());
    // Python's widths[len(widths) - skipped:], a negative start counted from the end
    let mut start = count - skipped;
    if start < 0 {
        start = (start + count).max(0);
    }
    let tail = widths.get(usize::try_from(start).unwrap_or(0)..).unwrap_or(&[]);
    let width = work.find(properties, W, "wAfter");
    let all_digits = tail
        .iter()
        .all(|w| !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit()));
    if all_digits {
        let width = match width {
            Some(w) => w,
            None => {
                let w = make(work, "wAfter", &[]);
                insert_in_order(work, properties, w, orders);
                w
            }
        };
        let sum: i64 = tail.iter().filter_map(|w| w.parse::<i64>().ok()).sum();
        set_attr(work, width, "w", &sum.to_string());
        set_attr(work, width, "type", "dxa");
    } else if let Some(width) = width {
        work.remove(width);
    }
}

/// Remove the grid columns only removed cells covered, and rows left without a cell -- a
/// table, left without a row (`_fix_grid`).
fn fix_grid(review: &mut PartReview<'_, '_>, tables: &[Id], grid: &GridMap, removed: &HashSet<Id>) {
    let work = &mut *review.work;
    let orders = review.orders;
    let rows: Vec<&GridRow> = grid
        .iter()
        .filter(|(row, _)| ancestor(work, *row, "tbl").is_some_and(|t| tables.contains(&t)))
        .collect();
    let mut columns: BTreeMap<i64, Vec<bool>> = BTreeMap::new();
    for (_, cells) in &rows {
        for (cell, start, span) in cells {
            for column in *start..start + span {
                columns.entry(column).or_default().push(removed.contains(cell));
            }
        }
    }
    let vacated: Vec<i64> = columns
        .iter()
        .rev()
        .filter(|(_, flags)| !flags.is_empty() && flags.iter().all(|f| *f))
        .map(|(c, _)| *c)
        .collect();
    let mut needed = 0;
    for (row, cells) in &rows {
        let properties = work.find(*row, W, "trPr");
        let mut total = 0;
        for local in ["gridBefore", "gridAfter"] {
            total += digits(work, properties.and_then(|p| work.find(p, W, local)), "val").unwrap_or(0);
        }
        total += cells
            .iter()
            .filter(|(c, _, _)| !removed.contains(c))
            .map(|(_, _, span)| span)
            .sum::<i64>();
        needed = needed.max(total);
    }
    for &table in tables {
        let grid_node = work.find(table, W, "tblGrid");
        if let Some(g) = grid_node {
            let cols = find_all(work, g, "gridCol");
            let take = usize::try_from((i64::try_from(cols.len()).unwrap_or(0) - needed).max(0)).unwrap_or(0);
            for column in vacated.iter().take(take) {
                if let Some(c) = usize::try_from(*column).ok().and_then(|k| cols.get(k)) {
                    work.remove(*c);
                }
            }
        }
        for row in find_all(work, table, "tr") {
            if work.find(row, W, "tc").is_none() && work.find(row, W, "sdt").is_none() {
                work.remove(row);
            }
        }
        if let Some(g) = grid_node.filter(|_| !review.edged_rows.is_empty()) {
            let widths: Vec<String> = find_all(work, g, "gridCol")
                .into_iter()
                .map(|c| attr(work, c, "w").unwrap_or("").to_owned())
                .collect();
            for (row, cells) in grid_map(work, table) {
                if review.edged_rows.contains(&row) {
                    skip_after(work, row, &cells, &widths, orders);
                }
            }
        }
        let rowless = work.find(table, W, "tr").is_none()
            && work.parent(table).is_some()
            && !work
                .children(table)
                .iter()
                .any(|c| work.is_any(*c, W, &["sdt", "customXml"]));
        if rowless {
            work.remove(table);
        }
    }
}

impl PartReview<'_, '_> {
    /// A row whose every cell's only paragraph had its deletion accepted here, with nothing
    /// left in it and no record of its own, goes (`_emptied_rows`).
    fn emptied_rows(&mut self) {
        for (row, cells) in self.emptied.clone() {
            let work = &self.work;
            if work.parent(row).is_none()
                || find_path(work, row, &["trPr", "ins"]).is_some()
                || find_path(work, row, &["trPr", "del"]).is_some()
            {
                continue;
            }
            let own: Vec<Id> = work
                .iter(row, W, &["tc"])
                .into_iter()
                .filter(|c| ancestor(work, *c, "tr") == Some(row))
                .collect();
            let full = own
                .iter()
                .any(|c| !cells.contains(c) || work.iter(*c, W, &["p"]).into_iter().any(|p| has_content(work, p)));
            if own.is_empty() || full {
                continue;
            }
            let table = ancestor(work, row, "tbl");
            self.removed_paragraphs.extend(self.work.iter(row, W, &["p"]));
            self.removing(row);
            keep_markers(self.work, row);
            self.work.remove(row);
            if let Some(table) = table {
                self.remove_if_rowless(table);
            }
        }
    }

    /// The tables the review left, tidied, then every paragraph pruned (`_tidy`).
    pub(super) fn tidy(&mut self) {
        self.emptied_rows();
        for key in self.removed_order.clone() {
            let removed = self.removed_cells.get(&key).cloned().unwrap_or_default();
            if removed.is_empty() || self.restored_grids.contains(&key) {
                continue;
            }
            let Some(grid) = self.maps.get(&key).cloned() else {
                continue;
            };
            let mut groups: Vec<(Id, Vec<Id>)> = Vec::new();
            let parts: Vec<Id> = std::iter::once(key)
                .chain(
                    self.split_parts
                        .iter()
                        .copied()
                        .filter(|p| self.origin.get(p) == Some(&key)),
                )
                .collect();
            for part in parts {
                let placed = groups.iter().any(|(_, g)| g.contains(&part));
                if self.work.parent(part).is_none() || placed {
                    continue;
                }
                let group = self.grid_group.get(&part).copied().unwrap_or(key);
                match groups.iter_mut().find(|(g, _)| *g == group) {
                    Some((_, members)) => members.push(part),
                    None => groups.push((group, vec![part])),
                }
            }
            for (_, members) in groups {
                fix_grid(self, &members, &grid, &removed);
            }
        }
        for table in self.respanned.clone() {
            if !self.restored_grids.contains(&table) && self.work.parent(table).is_some() {
                normalise_grid(self.work, table, self.orders);
            }
        }
        for key in self.touched_tables.clone() {
            if !self.maps.contains_key(&key) || self.work.parent(key).is_none() || fits(self.work, key) {
                continue;
            }
            let structured = self
                .work
                .descendants(key)
                .into_iter()
                .any(|n| self.work.is_any(n, W, &STRUCTURE));
            if structured {
                continue;
            }
            if self.restored_grids.contains(&key) {
                normalise_grid(self.work, key, self.orders);
            }
            if !fits(self.work, key) {
                fit_grid(self.work, key, self.orders);
            }
        }
        for paragraph in self.work.iter(self.work.root(), W, &["p"]) {
            let released = prune(self.work, paragraph);
            self.released.extend(released);
        }
    }
}
