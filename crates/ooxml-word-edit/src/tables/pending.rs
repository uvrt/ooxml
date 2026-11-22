//! Owns: a table's pending tracked structure as a tracked row, column, merge or split
//! edit works around it (docx-agent's `edit/tables.py`, rounds 53 and 55, and
//! `revisions/review.py`'s `merge_groups`): the pending tracked merges ([`merge_groups`]:
//! across, a cell and the cells merged away after it; down, a `w:cellMerge` run), the
//! cells that lead one, the text a merge copied from a cell into its first cell
//! ([`copies_of`]) and taken out again ([`drop_copies`]), the rectangles the merges make
//! ([`pending_boxes`]), and a new row's cells in a column whose insertion or deletion is
//! pending following that change ([`pending_column`], [`follow_pending_columns`]).
//!
//! Word is the oracle: these are the forms docx-agent measured Word's Accept All and
//! Reject All settle exactly (round 53, R107; round 55, R115, `feedback-round55/probe-C`).
//!
//! Must not: write anything but the cells and paragraphs it is given.

use super::merge::{has_runs, Position};
use super::shape::{own_rows, row_cells};
use crate::marks::{insert_in_order, mark_record, record_paragraph_change};
use crate::names::W;
use crate::policy::Stamp;
use crate::review::grid::{content_kept, grid_map};
use crate::review::xml::{ancestor, find_all, find_path, make, set_attr, INSERTED};
use crate::text::{original_text, paragraph_text};
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_xml::{Element, Node};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A pending tracked merge: its cells with their rows, the merge's first cell (where the
/// text goes) first.
pub(crate) type Group = Vec<(usize, Id)>;

/// A note or comment reference.
const REFERENCES: [&str; 3] = ["footnoteReference", "endnoteReference", "commentReference"];

/// The records that keep a cell of a pending change (`w:cellDel`, `w:cellMerge`).
const MERGE_RECORDS: [&str; 2] = ["cellDel", "cellMerge"];

/// Whether `cell` has a pending deletion (`w:tcPr/w:cellDel`).
fn deleted(work: &Work, cell: Id) -> bool {
    find_path(work, cell, &["tcPr", "cellDel"]).is_some()
}

/// Whether `cell` is a pending tracked merge's cell merged away: `w:cellDel` with its
/// content or a mark kept (`merged_away`).
pub(crate) fn merged_away(work: &Work, cell: Id) -> bool {
    deleted(work, cell) && content_kept(work, cell)
}

/// Whether `cell` is a pending tracked deletion's: `w:cellDel`, nothing kept
/// (`column_deleted`).
pub(crate) fn column_deleted(work: &Work, cell: Id) -> bool {
    deleted(work, cell) && !content_kept(work, cell)
}

/// The `w:vMerge` of a cell's `w:cellMerge` (`rest`, `cont`), if it has one.
pub(crate) fn cell_merge(work: &Work, cell: Id) -> Option<&str> {
    find_path(work, cell, &["tcPr", "cellMerge"]).map(|r| work.get(r, Some(W), "vMerge").unwrap_or_default())
}

/// `table`'s pending tracked merges (`merge_groups`): across, a row's cell and the cells
/// merged away after it (a cell a pending column deletion removes, between them, is not
/// the merge's); down, the cells a `w:cellMerge` run joins (`rest`, then `cont` in the
/// rows below, a row whose deletion is pending passed over), each with the cells merged
/// away after it.
pub(crate) fn merge_groups(work: &Work, table: Id) -> Vec<Group> {
    let grid = grid_map(work, table);
    // each cell not deleted and the cells merged away after it, in document order
    let mut segments: Vec<(Id, Group)> = Vec::new();
    let mut at: HashMap<Id, usize> = HashMap::new();
    for (r, (_, cells)) in grid.iter().enumerate() {
        let mut lead: Option<usize> = None;
        for &(cell, _, _) in cells {
            if !deleted(work, cell) {
                at.insert(cell, segments.len());
                lead = Some(segments.len());
                segments.push((cell, vec![(r, cell)]));
            } else if let Some(segment) = lead
                .filter(|_| content_kept(work, cell))
                .and_then(|k| segments.get_mut(k))
            {
                segment.1.push((r, cell));
            }
        }
    }
    let segment = |cell: Id, r: usize| {
        at.get(&cell)
            .and_then(|k| segments.get(*k))
            .map_or_else(|| vec![(r, cell)], |s| s.1.clone())
    };
    let mut groups = Vec::new();
    let mut taken: HashSet<Id> = HashSet::new();
    for (r, (_, cells)) in grid.iter().enumerate() {
        for &(cell, start, _) in cells {
            if cell_merge(work, cell) != Some("rest") || taken.contains(&cell) {
                continue;
            }
            let mut group = segment(cell, r);
            for (below, (row_below, cells_below)) in grid.iter().enumerate().skip(r + 1) {
                if find_path(work, *row_below, &["trPr", "del"]).is_some() {
                    continue;
                }
                let Some(&(found, _, _)) = cells_below.iter().find(|c| c.1 == start) else {
                    break;
                };
                if cell_merge(work, found) != Some("cont") {
                    break;
                }
                group.extend(segment(found, below));
            }
            taken.extend(group.iter().map(|g| g.1));
            groups.push(group);
        }
    }
    for (lead, group) in &segments {
        if group.len() > 1 && !taken.contains(lead) {
            groups.push(group.clone());
        }
    }
    groups
}

/// Each cell that leads a pending tracked merge in its row or down its column (every cell
/// of a merge not deleted) and the merge's cells (`merge_leads`).
pub(crate) fn merge_leads(work: &Work, table: Id) -> HashMap<Id, Vec<Id>> {
    let mut out = HashMap::new();
    for group in merge_groups(work, table) {
        let cells: Vec<Id> = group.iter().map(|g| g.1).collect();
        for cell in &cells {
            if !deleted(work, *cell) {
                out.insert(*cell, cells.clone());
            }
        }
    }
    out
}

/// Each cell a pending tracked merge took in and the merge's first cell, where its text
/// went (`_merge_origins`).
pub(crate) fn merge_origins(work: &Work, table: Id) -> HashMap<Id, Id> {
    let mut out = HashMap::new();
    for group in merge_groups(work, table) {
        if let Some(&(_, origin)) = group.first() {
            for &(_, cell) in group.iter().skip(1) {
                out.insert(cell, origin);
            }
        }
    }
    out
}

/// Whether every run of `paragraph` is inside an insertion (a merge's copy;
/// `_all_inserted`).
fn all_inserted(work: &Work, paragraph: Id) -> bool {
    let runs = work.iter(paragraph, W, &["r"]);
    !runs.is_empty()
        && runs.iter().all(|run| {
            let mut above = work.parent(*run);
            while let Some(node) = above {
                if work.is_any(node, W, &INSERTED) {
                    return true;
                }
                above = work.parent(node);
            }
            false
        })
}

/// The paragraphs a pending tracked merge copied from `cell` into `origin` (`_copies_of`):
/// for each of the cell's paragraphs with text, the first inserted paragraph of `origin`
/// (not its first) with the same text not yet matched -- a row's first cell merged down
/// has its content deleted, so its text as it was.
pub(crate) fn copies_of(work: &Work, origin: Option<Id>, cell: Id) -> Vec<Id> {
    let Some(origin) = origin else { return Vec::new() };
    let candidates: Vec<Id> = find_all(work, origin, "p")
        .into_iter()
        .skip(1)
        .filter(|p| all_inserted(work, *p))
        .collect();
    let merged_down = find_path(work, cell, &["tcPr", "cellMerge"]).is_some();
    let mut out: Vec<Id> = Vec::new();
    for paragraph in find_all(work, cell, "p").into_iter().filter(|p| has_runs(work, *p)) {
        let text = if merged_down {
            original_text(work, paragraph)
        } else {
            paragraph_text(work, paragraph)
        };
        if text.is_empty() {
            continue;
        }
        let found = candidates
            .iter()
            .copied()
            .find(|p| !out.contains(p) && paragraph_text(work, *p) == text);
        out.extend(found);
    }
    out
}

/// The text a pending tracked merge copied from `cell` (merged away) taken out of the
/// merge's first cell `origin` again (`_drop_copies`): each copy goes; where it was the
/// cell's last paragraph, the marks around it are set as a merge without `cell` writes
/// them (the cell's last mark inserted only while a copy follows it).
pub(crate) fn drop_copies(work: &mut Work, origin: Option<Id>, cell: Id, stamp: &mut Stamp, orders: &Orders) {
    let Some(holder) = origin else { return };
    for found in copies_of(work, origin, cell) {
        let paragraphs = find_all(work, holder, "p");
        let last = paragraphs.last() == Some(&found);
        let position = paragraphs.iter().position(|p| *p == found).unwrap_or(0);
        let old = find_path(work, found, &["pPr", "pPrChange", "pPr"]).map(|p| work.deep_copy(p));
        work.remove(found);
        if !last {
            continue;
        }
        let Some(before) = position.checked_sub(1).and_then(|k| paragraphs.get(k)).copied() else {
            continue;
        };
        if let Some(record) = mark_record(work, before).filter(|r| work.is_any(*r, W, &INSERTED)) {
            let run_properties = work.parent(record);
            work.remove(record);
            if let Some(emptied) = run_properties.filter(|h| work.children(*h).is_empty()) {
                work.remove(emptied);
            }
        }
        if let Some(old) = old.filter(|_| all_inserted(work, before)) {
            record_paragraph_change(work, before, old, stamp, orders);
        }
    }
}

/// Each pending tracked merge of the table as the rectangle it makes: (top row, bottom
/// row, first grid column, last grid column) (`_pending_boxes`).
pub(crate) fn pending_boxes(work: &Work, rows: &[Id]) -> Vec<(Position, Position)> {
    let mut place: HashMap<Id, (usize, usize, usize)> = HashMap::new();
    for (r, row) in rows.iter().enumerate() {
        for p in row_cells(work, *row) {
            place.insert(p.cell, (r, p.start, p.span));
        }
    }
    let mut boxes = Vec::new();
    for group in merge_groups(work, work.root()) {
        let spots: Vec<(usize, usize, usize)> = group.iter().filter_map(|(_, c)| place.get(c).copied()).collect();
        let (Some(top), Some(bottom), Some(left), Some(right)) = (
            spots.iter().map(|s| s.0).min(),
            spots.iter().map(|s| s.0).max(),
            spots.iter().map(|s| s.1).min(),
            spots.iter().map(|s| s.1 + s.2 - 1).max(),
        ) else {
            continue;
        };
        boxes.push(((top, left), (bottom, right)));
    }
    boxes
}

/// The records that make `cell` part of a column change still pending
/// (`_pending_column`, round 53, R107): its `w:cellDel` or `w:cellIns`, or -- Word's form
/// of a column inserted, tracked -- the inserted mark of its one empty paragraph (not in a
/// row inserted, tracked, whose marks are the row's).
pub(crate) fn pending_column(work: &Work, cell: Id) -> Vec<Id> {
    let mut records: Vec<Id> = work
        .find(cell, W, "tcPr")
        .map(|p| {
            work.children(p)
                .iter()
                .copied()
                .filter(|c| work.is_any(*c, W, &["cellDel", "cellIns"]))
                .collect()
        })
        .unwrap_or_default();
    let row = ancestor(work, cell, "tr");
    if row.is_some_and(|r| find_path(work, r, &["trPr", "ins"]).is_some()) {
        return records;
    }
    let blocks: Vec<Id> = work
        .children(cell)
        .iter()
        .copied()
        .filter(|c| work.is_any(*c, W, &["p", "tbl", "sdt"]))
        .collect();
    if let [only] = blocks.as_slice() {
        if work.is(*only, W, "p") && !has_runs(work, *only) {
            records.extend(mark_record(work, *only).filter(|r| work.is_any(*r, W, &INSERTED)));
        }
    }
    records
}

/// A new cell in a column whose change is pending: the cell, its paragraph and the
/// column's records.
pub(crate) struct Following {
    pub(crate) cell: Id,
    pub(crate) paragraph: Id,
    pub(crate) records: Vec<Id>,
}

/// The paragraph's mark properties (`w:pPr/w:rPr`), made where missing.
fn mark_properties(work: &mut Work, paragraph: Id, orders: &Orders) -> Id {
    let properties = match work.find(paragraph, W, "pPr") {
        Some(p) => p,
        None => {
            let p = make(work, "pPr", &[]);
            work.insert(paragraph, 0, p);
            p
        }
    };
    match work.find(properties, W, "rPr") {
        Some(r) => r,
        None => {
            let r = make(work, "rPr", &[]);
            insert_in_order(work, properties, r, orders);
            r
        }
    }
}

/// A new row's cells in columns whose tracked deletion or insertion is pending go with
/// that change (`_follow_pending_columns`, round 53, R107): each a copy of its column's
/// record numbered by `next_id` -- a `w:cellDel` (its mark deleted too: a deleted cell
/// whose mark is kept is one Accept All gives to the cell before it), or its paragraph's
/// mark inserted (Word's form of a column inserted, tracked). Accepted or rejected, the
/// new row then loses or keeps the cell as the column's other rows do. The paragraphs
/// given a mark of their column's.
pub(crate) fn follow_pending_columns(
    work: &mut Work,
    pending: &[Following],
    next_id: &mut dyn FnMut() -> String,
    orders: &Orders,
) -> Vec<Id> {
    let mut marked = Vec::new();
    for following in pending {
        for record in &following.records {
            let copied = work.deep_copy(*record);
            set_attr(work, copied, "id", &next_id());
            if work.is(*record, W, "cellDel") {
                let mark = work.deep_copy(*record);
                let name = work.qname(W, "del", "w");
                work.rename(mark, name);
                set_attr(work, mark, "id", &next_id());
                let holder = mark_properties(work, following.paragraph, orders);
                insert_in_order(work, holder, mark, orders);
                marked.push(following.paragraph);
            }
            if work.is_any(*record, W, &["ins", "del", "moveFrom", "moveTo"]) {
                let holder = mark_properties(work, following.paragraph, orders);
                insert_in_order(work, holder, copied, orders);
                marked.push(following.paragraph);
            } else if let Some(properties) = work.find(following.cell, W, "tcPr") {
                insert_in_order(work, properties, copied, orders);
            }
        }
    }
    marked
}

/// What a tracked reshape of a table reaches (docx-agent's `_no_reshape_over_merge_lead`
/// and `_no_split_over_pending`, round 55, R115).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reshape {
    /// Document row `index` deleted.
    DeleteRow(usize),
    /// Grid column `index` deleted.
    DeleteColumn(usize),
    /// The cell over (document row, grid column) split.
    Split(usize, usize),
}

/// The pending tracked merges a tracked reshape of `table` would move to another cell --
/// a row or column deleted that holds a merge's first cell, a split of a cell a merge
/// makes -- as their cells, `None` where it reaches none.
pub(crate) fn merge_in_the_way(work: &Work, table: Id, reshape: Reshape) -> Option<Vec<Vec<Id>>> {
    let rows = own_rows(work, table);
    let hit: Vec<Vec<Id>> = match reshape {
        Reshape::DeleteRow(index) => {
            let row = *rows.get(index)?;
            let inside = work.iter(row, W, &["tc"]);
            merge_groups(work, table)
                .into_iter()
                .filter(|g| g.first().is_some_and(|(_, first)| inside.contains(first)))
                .map(|g| g.into_iter().map(|(_, c)| c).collect())
                .collect()
        }
        Reshape::DeleteColumn(index) => {
            let leads = merge_leads(work, table);
            rows.iter()
                .flat_map(|row| row_cells(work, *row))
                .filter(|p| p.start == index && p.span == 1)
                .filter_map(|p| leads.get(&p.cell).cloned())
                .collect()
        }
        Reshape::Split(row, column) => {
            let leads = merge_leads(work, table);
            let cell = row_cells(work, *rows.get(row)?)
                .into_iter()
                .find(|p| p.start <= column && column < p.start + p.span)?
                .cell;
            vec![leads.get(&cell)?.clone()]
        }
    };
    (!hit.is_empty()).then_some(hit)
}

/// The cells a pending tracked merge took in that a tracked reshape deletes, each with
/// the merge's first cell (where their text went): in a row deleted, every cell of a
/// merge but its first; in a column deleted, each cell merged away only in it.
pub(crate) fn taken_in(work: &Work, table: Id, reshape: Reshape) -> Vec<(Id, Id)> {
    match reshape {
        Reshape::DeleteRow(index) => merge_groups(work, table)
            .into_iter()
            .filter_map(|g| {
                let origin = g.first()?.1;
                Some(
                    g.into_iter()
                        .skip(1)
                        .filter(move |(r, _)| *r == index)
                        .map(move |(_, c)| (origin, c)),
                )
            })
            .flatten()
            .collect(),
        Reshape::DeleteColumn(index) => {
            let origins = merge_origins(work, table);
            own_rows(work, table)
                .iter()
                .flat_map(|row| row_cells(work, *row))
                .filter(|p| p.start == index && p.span == 1 && merged_away(work, p.cell))
                .filter_map(|p| origins.get(&p.cell).map(|o| (*o, p.cell)))
                .collect()
        }
        Reshape::Split(..) => Vec::new(),
    }
}

/// A tracked row or column deletion (`what`) that would move a pending tracked merge to
/// another cell, refused (`delete_row`'s and `delete_column`'s round 55 refusals); one
/// whose cells' text, held in a merge's first cell, holds a note or comment reference is
/// not ported (a reference the deletion would orphan is the host's to word).
pub(crate) fn no_reshape_over_merge(work: &Work, reshape: Reshape, what: &str) -> Result<()> {
    let table = work.root();
    if merge_in_the_way(work, table, reshape).is_some() {
        return Err(EditError::Refused(format!(
            "{what} holds the first cell of a pending tracked merge (its text and the cells merged into it); \
             settle that merge first"
        )));
    }
    let referenced = taken_in(work, table, reshape).into_iter().any(|(origin, cell)| {
        copies_of(work, Some(origin), cell)
            .into_iter()
            .any(|copy| !work.iter(copy, W, &REFERENCES).is_empty())
    });
    if referenced {
        return Err(EditError::Unsupported(format!(
            "word_edit_table deleting {what}, whose text a pending tracked merge holds with a note or comment \
             reference"
        )));
    }
    Ok(())
}

/// The places, below `element` (the table `work` was read from), of the records that keep
/// the pending tracked merges a tracked reshape would move (their `w:cellDel` and
/// `w:cellMerge`), `None` where it reaches none: for the reply naming the changes to settle
/// first by id.
#[must_use]
pub fn merges_in_the_way(element: &Arc<Element>, reshape: Reshape) -> Option<Vec<Vec<u32>>> {
    let work = Work::of(element, Vec::new());
    let hit = merge_in_the_way(&work, work.root(), reshape)?;
    let records = hit
        .iter()
        .flatten()
        .flat_map(|cell| work.iter(*cell, W, &MERGE_RECORDS))
        .filter_map(|record| path_below(&work, element, record))
        .collect();
    Some(records)
}

/// Where `node` of `work` (read from `element`, unchanged) stands below `element`: the
/// child places, text included, from it down.
pub(crate) fn path_below(work: &Work, element: &Arc<Element>, node: Id) -> Option<Vec<u32>> {
    let mut chain = Vec::new();
    let mut at = node;
    while let Some(parent) = work.parent(at) {
        chain.push(work.children(parent).iter().position(|c| *c == at)?);
        at = parent;
    }
    let mut path = Vec::new();
    let mut current = element;
    for k in chain.into_iter().rev() {
        let (place, child) = current
            .children()
            .iter()
            .enumerate()
            .filter(|(_, n)| !matches!(n, Node::Text(_)))
            .nth(k)?;
        path.push(u32::try_from(place).ok()?);
        let Node::Element(child) = child else { return None };
        current = child;
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::{merge_groups, merges_in_the_way, Reshape};
    use crate::work::Work;
    use ooxml_base::Limits;

    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

    /// A cell holding `text`, its properties `properties`.
    fn cell(properties: &str, text: &str) -> String {
        format!("<w:tc><w:tcPr>{properties}</w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>")
    }

    /// A 2x2 table: row 0's cells merged across as a pending tracked merge (the second
    /// `w:cellDel` with its text kept), column 0 merged down (`w:cellMerge` rest, cont).
    fn table() -> std::sync::Arc<ooxml_xml::Element> {
        let deleted = r#"<w:cellDel w:id="1" w:author="A" w:date="2026-01-01T00:00:00Z"/>"#;
        let rest = r#"<w:cellMerge w:id="2" w:author="A" w:date="2026-01-01T00:00:00Z" w:vMerge="rest"/>"#;
        let cont = r#"<w:cellMerge w:id="3" w:author="A" w:date="2026-01-01T00:00:00Z" w:vMerge="cont"/>"#;
        let xml = format!(
            r#"<w:tbl xmlns:w="{W}"><w:tblGrid><w:gridCol/><w:gridCol/></w:tblGrid><w:tr>{}{}</w:tr><w:tr>{}{}</w:tr></w:tbl>"#,
            cell(rest, "A"),
            cell(deleted, "B"),
            cell(cont, "C"),
            cell("", "D"),
        );
        let tree = ooxml_xml::parse(xml.as_bytes(), &Limits::default().xml).unwrap();
        std::sync::Arc::clone(tree.root())
    }

    #[test]
    fn a_merge_down_takes_in_the_cells_merged_away_beside_it() {
        let element = table();
        let work = Work::of(&element, Vec::new());
        let groups = merge_groups(&work, work.root());
        assert_eq!(groups.len(), 1);
        let rows: Vec<usize> = groups[0].iter().map(|(r, _)| *r).collect();
        assert_eq!(rows, [0, 0, 1]);
    }

    #[test]
    fn reshapes_moving_a_merge_name_its_records() {
        let element = table();
        // row 0 holds the merge's first cell; row 1 holds only a continuation
        assert_eq!(
            merges_in_the_way(&element, Reshape::DeleteRow(0)).map(|r| r.len()),
            Some(3)
        );
        assert_eq!(merges_in_the_way(&element, Reshape::DeleteRow(1)), None);
        // column 0's cells lead the merge (each not deleted); column 1's second row is no merge's
        assert!(merges_in_the_way(&element, Reshape::DeleteColumn(0)).is_some());
        assert_eq!(merges_in_the_way(&element, Reshape::Split(1, 1)), None);
        assert!(merges_in_the_way(&element, Reshape::Split(1, 0)).is_some());
        // a record's place below the table: row 0, its first cell, its properties' record
        let places = merges_in_the_way(&element, Reshape::DeleteRow(0)).unwrap();
        assert_eq!(places[0], [1, 0, 0, 0]);
    }
}
