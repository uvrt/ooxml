//! Owns: the review's property changes and paragraph marks (docx-agent's
//! `_Review._property`, `_mark` and `_join`, and `edit/sections.py`'s `restore_kind` for a
//! rejected section change): a property change accepted drops its record, rejected puts
//! the old properties back; a mark that goes joins its paragraph to the next, or removes
//! an empty one, or leaves its cell to [`super::edges`].
//!
//! Must not: apply rows, cells or containers (`super::part`), or tidy grids.

use super::part::PartReview;
use super::xml::{
    ancestor, attr, find_all, find_path, has_content, insert_in_order, keep_markers, next_paragraph, remove_if_empty,
    CELL, CONTAINERS, DELETED, INSERTED,
};
use crate::names::W;
use crate::validate::Orders;
use crate::work::{Id, Work};

/// How each section kind that was absent before a tracked change is recorded: the
/// element in its "off" form (`w:<local> w:<attribute>="<value>"`), which rejecting removes.
const ABSENT: [(&str, Option<(&str, &str)>); 8] = [
    ("titlePg", Some(("val", "0"))),
    ("vAlign", Some(("val", "top"))),
    ("type", Some(("val", "nextPage"))),
    ("pgNumType", Some(("fmt", "decimal"))),
    ("lnNumType", Some(("countBy", "0"))),
    ("footnotePr", None),
    ("endnotePr", None),
    ("cols", Some(("num", "1"))),
];

/// Attributes whose absence means a value: dropped again where rejecting leaves that value.
const DEFAULTS: [(&str, &str, &str); 6] = [
    ("cols", "num", "1"),
    ("cols", "sep", "0"),
    ("cols", "equalWidth", "1"),
    ("pgSz", "orient", "portrait"),
    ("pgNumType", "fmt", "decimal"),
    ("lnNumType", "countBy", "0"),
];

/// Section kinds whose meaning is their children.
const PARENTS: [&str; 3] = ["cols", "footnotePr", "endnotePr"];

impl PartReview<'_, '_> {
    pub(super) fn property(&mut self, record: Id) {
        let Some(owner) = self.work.parent(record) else { return };
        if self.accept {
            self.work.remove(record);
            if self.work.is_any(owner, W, &["trPr", "tcPr"]) && self.work.children(owner).is_empty() {
                self.work.remove(owner);
                return;
            }
            remove_if_empty(self.work, owner);
            return;
        }
        let old = self
            .work
            .children(record)
            .iter()
            .copied()
            .find(|c| self.work.is_element(*c));
        let local = self.work.name(record).map(|n| n.local().to_owned()).unwrap_or_default();
        let keep: &[&str] = match local.as_str() {
            "pPrChange" => &["rPr", "sectPr"],
            "rPrChange" => &CONTAINERS,
            "trPrChange" => &["ins", "del"],
            "tcPrChange" => {
                self.respan(owner, old);
                &CELL
            }
            "sectPrChange" => {
                self.work.remove(record);
                let recorded: Vec<Id> = old
                    .map(|o| self.work.children(o).to_vec())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|c| self.work.is_element(*c))
                    .collect();
                for child in recorded {
                    restore_kind(self.work, owner, child, self.orders);
                }
                return;
            }
            "numberingChange" => {
                self.work.remove(record);
                return;
            }
            _ => &[],
        };
        self.work.remove(record);
        for child in self.work.children(owner).to_vec() {
            if self.work.is_element(child) && !self.work.is_any(child, W, keep) {
                self.work.remove(child);
            }
        }
        for child in old.map(|o| self.work.children(o).to_vec()).unwrap_or_default() {
            if self.work.is_element(child) {
                let copy = self.work.deep_copy(child);
                insert_in_order(self.work, owner, copy, self.orders);
            }
        }
        if local == "trPrChange" {
            let mut skips = find_all(self.work, owner, "gridAfter");
            skips.extend(find_all(self.work, owner, "gridBefore"));
            for node in skips {
                if attr(self.work, node, "val").unwrap_or("0") == "0" {
                    self.work.remove(node);
                }
            }
        }
        if self.work.is_any(owner, W, &["trPr", "tcPr"]) && self.work.children(owner).is_empty() {
            self.work.remove(owner);
            return;
        }
        remove_if_empty(self.work, owner);
    }

    /// A rejected cell change whose span differs from the cell's: its table's grid is
    /// rebuilt from the cells afterwards.
    fn respan(&mut self, owner: Id, old: Option<Id>) {
        let span = |work: &Work, of: Option<Id>, default: Option<&'static str>| -> Option<String> {
            match of.and_then(|o| work.find(o, W, "gridSpan")) {
                Some(node) => attr(work, node, "val").map(str::to_owned),
                None => default.map(str::to_owned),
            }
        };
        let current = span(self.work, Some(owner), Some("1"));
        let before = if old.is_some() {
            span(self.work, old, Some("1"))
        } else {
            Some("1".to_owned())
        };
        if current != before {
            if let Some(table) = ancestor(self.work, owner, "tbl") {
                if !self.respanned.contains(&table) {
                    self.respanned.push(table);
                }
            }
        }
    }

    pub(super) fn mark(&mut self, record: Id) {
        if self.work.parent(record).is_none() {
            return;
        }
        let paragraph = ancestor(self.work, record, "p");
        let deleted = self.work.is_any(record, W, &DELETED);
        let goes = deleted == self.accept;
        let Some(paragraph) = paragraph.filter(|_| goes) else {
            self.drop_record(record);
            return;
        };
        let Some(cell) = self.work.parent(paragraph) else {
            return; // gone with what held it (a note whose reference the review removed)
        };
        let following = next_paragraph(self.work, paragraph);
        if following.is_none()
            && self.accept
            && deleted
            && (self.join_into_table(paragraph) || self.cell_end(paragraph, record))
        {
            return;
        }
        let Some(following) = following else {
            self.mark_at_end(paragraph, cell, record);
            return;
        };
        self.join(paragraph, following);
    }

    /// A mark that goes with no paragraph after it in its container.
    fn mark_at_end(&mut self, paragraph: Id, cell: Id, record: Id) {
        let empty = !has_content(self.work, paragraph);
        let blocks: Vec<Id> = self
            .work
            .children(cell)
            .iter()
            .copied()
            .filter(|c| self.work.is_any(*c, W, &["p", "tbl", "sdt"]))
            .collect();
        let alone = blocks == [paragraph];
        let in_cell = self.work.is(cell, W, "tc");
        let table = ancestor(self.work, cell, "tbl");
        if empty && in_cell && alone && self.work.is_any(record, W, &INSERTED) {
            if let Some(table) = table {
                // Word's own form of an inserted column: rejecting the cell's inserted mark
                // removes the cell, and the grid column it leaves
                self.removed_from(table).insert(cell);
                self.tables.insert(table);
                self.released.extend(crate::inline::relationship_ids(self.work, cell));
                self.work.remove(cell);
                self.removed_paragraphs.push(paragraph);
                return;
            }
        }
        let breaks = find_path(self.work, paragraph, &["pPr", "sectPr"]).is_some();
        if empty && (blocks.len() > 1 || !in_cell) && (!breaks || self.work.is(cell, W, "body")) {
            self.removed_paragraphs.push(paragraph);
            self.released
                .extend(crate::inline::relationship_ids(self.work, paragraph));
            self.removing(paragraph);
            keep_markers(self.work, paragraph);
            self.work.remove(paragraph);
            return;
        }
        if empty && in_cell && alone && self.work.is_any(record, W, &DELETED) {
            if let Some(row) = ancestor(self.work, cell, "tr") {
                match self.emptied.iter_mut().find(|(r, _)| *r == row) {
                    Some((_, cells)) => {
                        cells.insert(cell);
                    }
                    None => self.emptied.push((row, std::iter::once(cell).collect())),
                }
            }
        }
        self.drop_record(record);
    }

    /// `paragraph` joined to `following`: its content moves to the start of `following`,
    /// which keeps its element, id and properties (Word's accept).
    pub(super) fn join(&mut self, paragraph: Id, following: Id) {
        let mut at = usize::from(self.work.find(following, W, "pPr").is_some());
        for child in self.work.children(paragraph).to_vec() {
            if self.work.is(child, W, "pPr") {
                continue;
            }
            self.work.insert(following, at, child);
            at += 1;
        }
        self.joined.push((paragraph, following));
        self.removed_paragraphs.push(paragraph);
        self.work.remove(paragraph);
    }
}

/// Whether `node` is the "off" form a tracked change recorded for a section kind that was
/// absent (`is_absent_marker`).
fn is_absent_marker(work: &Work, node: Id) -> bool {
    let Some(name) = work.name(node).filter(|n| n.namespace() == Some(W)) else {
        return false;
    };
    let Some((_, attribute)) = ABSENT.iter().find(|(local, _)| *local == name.local()) else {
        return false;
    };
    if !work.children(node).is_empty() || !work.text(node).trim().is_empty() {
        return false;
    }
    let attributes = work.attributes(node);
    match attribute {
        None => attributes.is_empty(),
        Some((local, value)) => {
            attributes.len() == 1
                && attributes
                    .iter()
                    .all(|a| a.name.is(Some(W), local) && &*a.value == *value)
        }
    }
}

/// Reject one recorded section kind into `sect` as Word does (`restore_kind`).
fn restore_kind(work: &mut Work, sect: Id, recorded: Id, orders: &Orders) {
    let Some(name) = work.name(recorded).cloned() else {
        return;
    };
    let local = name.local().to_owned();
    let find_same = |work: &Work| {
        work.children(sect).iter().copied().find(|c| {
            work.name(*c)
                .is_some_and(|n| n.namespace() == name.namespace() && n.local() == name.local())
        })
    };
    let same = find_same(work);
    if is_absent_marker(work, recorded) {
        if let Some(same) = same {
            work.remove(same);
        }
        return;
    }
    let same = match same {
        None => {
            let copy = work.deep_copy(recorded);
            insert_in_order(work, sect, copy, orders);
            find_same(work)
        }
        Some(same) => {
            for attribute in work.attributes(recorded).to_vec() {
                work.set(same, attribute.name, &attribute.value);
            }
            if PARENTS.contains(&local.as_str()) || !work.children(recorded).is_empty() {
                for child in work.children(same).to_vec() {
                    work.remove(child);
                }
                for child in work.children(recorded).to_vec() {
                    if work.is_element(child) {
                        let copy = work.deep_copy(child);
                        work.append(same, copy);
                    }
                }
            }
            Some(same)
        }
    };
    let Some(same) = same else { return };
    for (kind, attribute, value) in DEFAULTS {
        if kind == local && attr(work, same, attribute) == Some(value) {
            work.remove_attr(same, Some(W), attribute);
        }
    }
}
