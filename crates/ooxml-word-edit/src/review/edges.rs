//! Owns: an accepted deleted mark at a table's edge, as Word's Accept All leaves it (round
//! 54, measured; docx-agent's `_Review._into_table`, `_cell_end` and `_out`): before a
//! table or block-level control, the paragraph joins the first paragraph inside it; at a
//! cell's end, the cell takes the next cell of its row in, or -- the row's last cell --
//! goes, what it held moving after the row in paragraphs of its own, the table split there.
//!
//! Must not: tidy the grids it leaves (`super::grid` does, after every record).

use super::part::PartReview;
use super::xml::{ancestor, find_path, has_content, make, next_block, BLOCKS};
use crate::names::W;
use crate::tables::shape::row_cells;
use crate::work::{Id, Work};
use std::collections::HashSet;

/// Whether `cell` is deleted (`w:cellDel`).
pub(crate) fn cell_deleted(work: &Work, cell: Id) -> bool {
    find_path(work, cell, &["tcPr", "cellDel"]).is_some()
}

/// A row's cells (row-level controls seen through) with their first grid column and span
/// (`edit/tables.py`'s `_cells`, placed by [`row_cells`]).
pub(crate) fn cells_of(work: &Work, row: Id) -> Vec<(Id, i64, i64)> {
    let signed = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
    row_cells(work, row)
        .into_iter()
        .map(|p| (p.cell, signed(p.start), signed(p.span)))
        .collect()
}

/// The first paragraph among `nodes` as accepting leaves them (`_first_in`).
fn first_in(work: &Work, nodes: &[Id]) -> Option<Id> {
    for &node in nodes {
        if !work.is_any(node, W, &BLOCKS) {
            continue;
        }
        if work.is(node, W, "p") {
            return Some(node);
        }
        if work.is(node, W, "tbl") {
            return first_paragraph(work, node);
        }
        let content = if work.is(node, W, "sdt") {
            work.find(node, W, "sdtContent")
        } else {
            Some(node)
        };
        return content.and_then(|c| first_in(work, work.children(c)));
    }
    None
}

/// The first paragraph of `table`'s first cell as accepting leaves the table.
fn first_paragraph(work: &Work, table: Id) -> Option<Id> {
    for row in work.iter(table, W, &["tr"]) {
        if ancestor(work, row, "tbl") != Some(table) || find_path(work, row, &["trPr", "del"]).is_some() {
            continue;
        }
        let shown: Vec<Id> = cells_of(work, row)
            .into_iter()
            .map(|(c, _, _)| c)
            .filter(|c| !cell_deleted(work, *c))
            .collect();
        if let Some(first) = shown.first() {
            return first_in(work, work.children(*first));
        }
    }
    None
}

/// A cell's last block when it is a paragraph (controls seen through).
fn last_paragraph(work: &Work, container: Id) -> Option<Id> {
    let elements: Vec<Id> = work
        .children(container)
        .iter()
        .copied()
        .filter(|n| work.is_element(*n))
        .collect();
    for &node in elements.iter().rev() {
        if work.is(node, W, "p") {
            return Some(node);
        }
        if work.is(node, W, "tbl") {
            return None;
        }
        if work.is_any(node, W, &["sdt", "customXml"]) {
            let content = if work.is(node, W, "sdt") {
                work.find(node, W, "sdtContent")
            } else {
                Some(node)
            };
            if let Some(content) = content {
                if work.children(content).iter().any(|n| work.is_any(*n, W, &BLOCKS)) {
                    return last_paragraph(work, content);
                }
            }
        }
    }
    None
}

impl PartReview<'_, '_> {
    /// A paragraph whose mark goes just before a table (or block-level control) joins the
    /// first paragraph inside it; one ending a section stays.
    pub(super) fn join_into_table(&mut self, paragraph: Id) -> bool {
        if find_path(self.work, paragraph, &["pPr", "sectPr"]).is_some() {
            return false;
        }
        let Some(block) = next_block(self.work, paragraph) else {
            return false;
        };
        if !self.work.is_any(block, W, &["tbl", "sdt", "customXml"]) {
            return false;
        }
        let Some(target) = first_in(self.work, &[block]) else {
            return false;
        };
        self.join(paragraph, target);
        true
    }

    /// A cell's last paragraph whose mark goes: the cell takes the next cell of its row in,
    /// or -- with none -- goes, what it holds moving after the row.
    pub(super) fn cell_end(&mut self, paragraph: Id, record: Id) -> bool {
        let Some(cell) = ancestor(self.work, paragraph, "tc") else {
            return false;
        };
        if cell_deleted(self.work, cell) || last_paragraph(self.work, cell) != Some(paragraph) {
            return false;
        }
        let Some(row) = self.work.parent(cell).filter(|r| self.work.is(*r, W, "tr")) else {
            return false;
        };
        let Some(table) = self.work.parent(row).filter(|t| self.work.is(*t, W, "tbl")) else {
            return false;
        };
        if find_path(self.work, row, &["trPr", "del"]).is_some() {
            return false; // a row being deleted
        }
        let shown: Vec<Id> = cells_of(self.work, row)
            .into_iter()
            .map(|(c, _, _)| c)
            .filter(|c| !cell_deleted(self.work, *c))
            .collect();
        let Some(k) = shown.iter().position(|c| *c == cell) else {
            return false;
        };
        let origin = self.origin.get(&table).copied().unwrap_or(table);
        match shown.get(k + 1) {
            Some(&taken) => self.take_next(paragraph, record, (cell, row, table, origin), taken, k == 0),
            None => {
                self.give_up(cell, record, row, table, origin);
                true
            }
        }
    }

    /// The cell takes the next cell `taken` in: its blocks after the paragraph, the first
    /// joined to it.
    fn take_next(&mut self, paragraph: Id, record: Id, at: (Id, Id, Id, Id), taken: Id, first: bool) -> bool {
        let (cell, row, table, origin) = at;
        if self.work.parent(taken) != Some(row) {
            return false;
        }
        let mut leaving: Vec<Id> = Vec::new();
        if first && self.work.parent(paragraph) == Some(cell) {
            let siblings = self.work.children(cell).to_vec();
            let here = siblings.iter().position(|c| *c == paragraph).unwrap_or(0);
            leaving = siblings
                .iter()
                .take(here)
                .copied()
                .filter(|c| !self.work.is(*c, W, "tcPr"))
                .collect();
            if !leaving.iter().any(|c| self.work.is_any(*c, W, &BLOCKS)) {
                leaving.clear();
            }
        }
        let blocks: Vec<Id> = self
            .work
            .children(taken)
            .iter()
            .copied()
            .filter(|c| !self.work.is(*c, W, "tcPr"))
            .collect();
        let head = blocks
            .iter()
            .copied()
            .find(|c| self.work.is_any(*c, W, &["p", "tbl", "sdt"]));
        let mut anchor = paragraph;
        for child in blocks {
            self.work.add_next(anchor, child);
            anchor = child;
        }
        match head.filter(|h| self.work.is(*h, W, "p")) {
            Some(head) => self.join(paragraph, head),
            None => self.drop_record(record),
        }
        self.removed_from(origin).insert(taken);
        self.work.remove(taken);
        self.edged_rows.insert(row);
        if !leaving.is_empty() {
            self.out(table, row, &leaving, true);
        }
        true
    }

    /// A row's last cell whose last mark goes: the cell goes, what it holds moving after
    /// the row in blocks of its own.
    fn give_up(&mut self, cell: Id, record: Id, row: Id, table: Id, origin: Id) {
        self.drop_record(record);
        let items: Vec<Id> = self
            .work
            .children(cell)
            .iter()
            .copied()
            .filter(|c| self.work.is_element(*c) && !self.work.is(*c, W, "tcPr"))
            .collect();
        let empty: Vec<Id> = items
            .iter()
            .copied()
            .filter(|c| self.work.is(*c, W, "p") && !has_content(self.work, *c))
            .collect();
        let rest: Vec<Id> = items.iter().copied().filter(|c| !empty.contains(c)).collect();
        let moving: Vec<Id> = if rest.iter().any(|c| self.work.is_any(*c, W, &BLOCKS)) {
            rest
        } else {
            Vec::new()
        };
        let kept: HashSet<Id> = moving.iter().flat_map(|c| self.work.iter(*c, W, &["p"])).collect();
        let gone: Vec<Id> = self
            .work
            .iter(cell, W, &["p"])
            .into_iter()
            .filter(|p| !kept.contains(p))
            .collect();
        self.removed_paragraphs.extend(gone);
        let inside: HashSet<Id> = moving.iter().flat_map(|c| self.work.descendants(*c)).collect();
        for node in self
            .work
            .iter(cell, W, &["commentReference", "footnoteReference", "endnoteReference"])
        {
            if !inside.contains(&node) {
                self.orphan(node);
            }
        }
        let kept_ids: HashSet<String> = moving
            .iter()
            .flat_map(|c| crate::inline::relationship_ids(self.work, *c))
            .collect();
        let released: Vec<String> = crate::inline::relationship_ids(self.work, cell)
            .into_iter()
            .filter(|rid| !kept_ids.contains(rid))
            .collect();
        self.released.extend(released);
        self.removed_from(origin).insert(cell);
        self.edged_rows.insert(row);
        for child in &moving {
            self.work.remove(*child);
        }
        self.work.remove(cell);
        if !moving.is_empty() {
            self.out(table, row, &moving, false);
            let paragraphs: Vec<Id> = moving
                .iter()
                .flat_map(|c| {
                    if self.work.is(*c, W, "p") {
                        vec![*c]
                    } else {
                        self.work.iter(*c, W, &["p"])
                    }
                })
                .collect();
            let joined = paragraphs
                .iter()
                .map(|p| crate::text::paragraph_text(self.work, *p))
                .collect::<Vec<_>>()
                .join(" ");
            let text = joined.split_whitespace().collect::<Vec<_>>().join(" ");
            self.moved_out.push((text, paragraphs));
        }
    }

    /// `blocks` out of `table`, before or after `row`: the table split there.
    fn out(&mut self, table: Id, row: Id, blocks: &[Id], before: bool) {
        let rows: Vec<Id> = self
            .work
            .children(table)
            .iter()
            .copied()
            .filter(|r| self.work.is(*r, W, "tr"))
            .collect();
        let cut = rows.iter().position(|r| *r == row).unwrap_or(0) + usize::from(!before);
        if cut == 0 {
            for &block in blocks {
                self.work.add_previous(table, block);
            }
            return;
        }
        if let Some(&first) = rows.get(cut) {
            let part = make(self.work, "tbl", &[]);
            for child in self.work.children(table).to_vec() {
                if self.work.is_any(child, W, &["tblPr", "tblGrid"]) {
                    let copied = self.work.deep_copy(child);
                    for record in self.work.iter(copied, W, &["tblPrChange", "tblGridChange"]) {
                        self.work.remove(record);
                    }
                    self.work.append(part, copied);
                }
            }
            let siblings = self.work.children(table).to_vec();
            let from = siblings.iter().position(|c| *c == first).unwrap_or(siblings.len());
            for child in siblings.into_iter().skip(from) {
                self.work.append(part, child);
            }
            self.work.add_next(table, part);
            let origin = self.origin.get(&table).copied().unwrap_or(table);
            self.origin.insert(part, origin);
            let group = if before {
                part
            } else {
                self.grid_group.get(&table).copied().unwrap_or(origin)
            };
            self.grid_group.insert(part, group);
            self.split_parts.push(part);
        }
        let mut anchor = table;
        for &block in blocks {
            self.work.add_next(anchor, block);
            anchor = block;
        }
    }
}
