//! Owns: a table's rows deleted as tracked changes, as docx-agent deletes them
//! (`TableOps._track_delete_rows`, measured on Word there): a row wholly the author's own
//! pending insertion removed outright (what another author inserted in it moved to the
//! paragraph after it, deleted), a row the author inserted holding what was there before
//! kept with its content and marks deleted, any other row given `w:trPr/w:del` with its
//! content and marks deleted; the table with its last row; and the empty paragraph an
//! inserted table came with (`_separator_of`, `_drop_separator`).
//!
//! Must not: allocate revision ids other than through its [`Stamp`], or read outside the
//! work it is given and the table's siblings.

use crate::across::rehome_markers;
use crate::inline::relationship_ids;
use crate::marks::{insert_in_order, mark, mark_record};
use crate::names::W;
use crate::policy::Stamp;
use crate::validate::Orders;
use crate::work::{Id, Work};
use ooxml_xml::{Element, Node};

/// How a tracked deletion treats a row (docx-agent's `_row_fate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fate {
    /// Wholly one's own insertion: removed outright.
    Remove,
    /// The same, holding another author's insertion: removed, that moved on.
    Relocate,
    /// One's own row holding what was there before: content deleted, insertion kept.
    Keep,
    /// `w:trPr/w:del`, content deleted.
    Mark,
}

/// The rows whose nearest table is `table` (docx-agent's `own_rows`).
pub(crate) fn own_rows(work: &Work, table: Id) -> Vec<Id> {
    work.iter(table, W, &["tr"])
        .into_iter()
        .filter(|row| {
            let mut at = work.parent(*row);
            while let Some(node) = at.filter(|n| !work.is(*n, W, "tbl")) {
                at = work.parent(node);
            }
            at == Some(table)
        })
        .collect()
}

/// The child `w:{a}/w:{b}` of `node`.
fn find2(work: &Work, node: Id, a: &str, b: &str) -> Option<Id> {
    work.find(node, W, a).and_then(|n| work.find(n, W, b))
}

/// docx-agent's `_row_fate` for a deletion by `author`.
pub(crate) fn row_fate(work: &Work, row: Id, author: &str) -> Fate {
    let mine = |record: Id| work.get(record, Some(W), "author") == Some(author);
    let inserted = find2(work, row, "trPr", "ins");
    if !inserted.is_some_and(mine) || find2(work, row, "trPr", "del").is_some() {
        return Fate::Mark;
    }
    if !work.iter(row, W, &["tbl"]).is_empty() {
        return Fate::Keep;
    }
    let mut others = false;
    for paragraph in work.iter(row, W, &["p"]) {
        let record = mark_record(work, paragraph);
        if !record.is_some_and(|r| work.is(r, W, "ins") && mine(r)) {
            return Fate::Keep;
        }
        for run in work.iter(paragraph, W, &["r"]) {
            let mut at = work.parent(run);
            let mut owner = None;
            while let Some(node) = at.filter(|n| *n != paragraph) {
                if work.is(node, W, "ins") {
                    owner = Some(node);
                    break;
                }
                at = work.parent(node);
            }
            let Some(owner) = owner else { return Fate::Keep };
            others = others || !mine(owner);
        }
    }
    if others {
        Fate::Relocate
    } else {
        Fate::Remove
    }
}

/// Whether a table is wholly the author's own insertion: every row of its own is.
pub(crate) fn own_table(work: &Work, table: Id, author: &str) -> bool {
    let rows = own_rows(work, table);
    !rows.is_empty() && rows.iter().all(|r| row_fate(work, *r, author) == Fate::Remove)
}

/// Whether `node` is inside one of `rows`.
fn in_rows(work: &Work, node: Id, rows: &[Id]) -> bool {
    let mut at = work.parent(node);
    while let Some(n) = at {
        if rows.contains(&n) {
            return true;
        }
        at = work.parent(n);
    }
    false
}

/// The first paragraph after `row` in document order not in a row of `going`
/// (docx-agent's `_paragraph_after`).
fn paragraph_after(work: &Work, row: Id, going: &[Id]) -> Option<Id> {
    let all = work.iter(work.root(), W, &["p"]);
    let inside = work.iter(row, W, &["p"]);
    let last = all.iter().rposition(|p| inside.contains(p))?;
    all.get(last + 1..)?
        .iter()
        .copied()
        .find(|p| !inside.contains(p) && !in_rows(work, *p, going))
}

/// A deleted row's cell paragraph mark (docx-agent's `_delete_cell_mark`).
fn delete_cell_mark(work: &mut Work, paragraph: Id, own: bool, stamp: &mut Stamp, orders: &Orders) {
    let Some(record) = mark_record(work, paragraph) else {
        mark(work, paragraph, "del", stamp, orders);
        return;
    };
    let deleted = work
        .find(paragraph, W, "pPr")
        .and_then(|p| work.find(p, W, "rPr"))
        .and_then(|r| work.find(r, W, "del"))
        .is_some();
    if !work.is(record, W, "ins") || deleted {
        return;
    }
    if !stamp.same(work, record) {
        mark(work, paragraph, "del", stamp, orders);
    } else if own {
        work.remove(record);
        mark(work, paragraph, "del", stamp, orders);
    }
}

/// Track the deletion of a paragraph's whole content; the relationship ids released.
pub(crate) fn delete_content(work: &mut Work, paragraph: Id, stamp: &mut Stamp) -> Vec<String> {
    let runs = crate::text::runs(work, paragraph);
    crate::track::delete_runs(work, &runs, stamp)
}

/// Give `row` (and with `nested`, the rows of its nested tables) `w:trPr/w:del`.
fn mark_row(work: &mut Work, row: Id, nested: bool, stamp: &mut Stamp, orders: &Orders) {
    let mut targets = vec![row];
    if nested {
        targets.extend(work.iter(row, W, &["tr"]).into_iter().filter(|r| *r != row));
    }
    for target in targets {
        let properties = match work.find(target, W, "trPr") {
            Some(p) => p,
            None => {
                let p = work.new_element(work.element_name(target, W, "trPr", "w"));
                insert_in_order(work, target, p, orders);
                p
            }
        };
        if work.find(properties, W, "del").is_none() {
            let record = stamp.make(work, "del");
            insert_in_order(work, properties, record, orders);
        }
    }
}

/// Delete `rows` (of one table, in document order) as tracked changes; the rows removed
/// outright and the relationship ids released.
pub(crate) fn track_delete_rows(
    work: &mut Work,
    rows: &[Id],
    nested: bool,
    stamp: &mut Stamp,
    orders: &Orders,
) -> (Vec<Id>, Vec<String>) {
    let author = stamp.author().to_owned();
    let mut kinds: Vec<Fate> = rows.iter().map(|r| row_fate(work, *r, &author)).collect();
    let mut going: Vec<Id> = rows
        .iter()
        .zip(&kinds)
        .filter(|(_, k)| matches!(k, Fate::Remove | Fate::Relocate))
        .map(|(r, _)| *r)
        .collect();
    let mut targets: Vec<Option<Id>> = vec![None; rows.len()];
    for (k, row) in rows.iter().enumerate() {
        if kinds.get(k) == Some(&Fate::Relocate) {
            let target = paragraph_after(work, *row, &going);
            if target.is_none() {
                if let Some(kind) = kinds.get_mut(k) {
                    *kind = Fate::Keep;
                }
                going.retain(|r| r != row);
            }
            if let Some(slot) = targets.get_mut(k) {
                *slot = target;
            }
        }
    }
    let mut released = Vec::new();
    for (row, kind) in rows.iter().zip(&kinds) {
        if matches!(kind, Fate::Remove | Fate::Relocate) {
            continue;
        }
        if *kind == Fate::Mark {
            mark_row(work, *row, nested, stamp, orders);
        }
        for paragraph in work.iter(*row, W, &["p"]) {
            released.extend(delete_content(work, paragraph, stamp));
            delete_cell_mark(work, paragraph, *kind == Fate::Keep, stamp, orders);
        }
    }
    let mut removed = Vec::new();
    for (k, row) in rows.iter().enumerate().rev() {
        if !matches!(kinds.get(k), Some(Fate::Remove | Fate::Relocate)) {
            continue;
        }
        let mut moved = Vec::new();
        for paragraph in work.iter(*row, W, &["p"]) {
            released.extend(delete_content(work, paragraph, stamp));
            moved.extend(
                work.children(paragraph)
                    .iter()
                    .copied()
                    .filter(|c| work.is_element(*c) && !work.is(*c, W, "pPr") && !work.iter(*c, W, &["r"]).is_empty()),
            );
        }
        if let Some(target) = targets.get(k).copied().flatten().filter(|_| !moved.is_empty()) {
            let at = usize::from(work.find(target, W, "pPr").is_some());
            for (n, node) in moved.into_iter().enumerate() {
                work.insert(target, at + n, node);
            }
        }
        rehome_markers(work, *row);
        released.extend(relationship_ids(work, *row));
        work.remove(*row);
        removed.push(*row);
    }
    removed.reverse();
    (removed, released)
}

/// A table's tracked deletion: its rows as Word deletes them; the table too once none of
/// its own rows is left. The rows removed outright and the relationship ids released.
pub(crate) fn track_delete_table(
    work: &mut Work,
    table: Id,
    stamp: &mut Stamp,
    orders: &Orders,
) -> (Vec<Id>, Vec<String>) {
    let rows = own_rows(work, table);
    let (gone, mut released) = track_delete_rows(work, &rows, true, stamp, orders);
    if !gone.is_empty() && own_rows(work, table).is_empty() {
        rehome_markers(work, table);
        released.extend(relationship_ids(work, table));
        work.remove(table);
    }
    (gone, released)
}

/// Whether a tree node is a block (`w:p`, `w:tbl`, `w:sdt`, `w:customXml`).
fn is_block(element: &Element) -> bool {
    element.name().namespace() == Some(W) && matches!(element.name().local(), "p" | "tbl" | "sdt" | "customXml")
}

/// The place of the block beside child `index` of `parent` (before it for `step` -1),
/// a section break or the end stopping the search (docx-agent's `_block_sibling`).
fn block_sibling(parent: &Element, index: usize, step: isize) -> Option<usize> {
    let children = parent.children().to_vec();
    let mut k = index.checked_add_signed(step)?;
    loop {
        if let Some(Node::Element(e)) = children.get(k) {
            if is_block(e) {
                return Some(k);
            }
            if e.name().is(Some(W), "sectPr") {
                return None;
            }
        }
        children.get(k)?;
        k = k.checked_add_signed(step)?;
    }
}

/// The empty paragraph, `author`'s own pending insertion, between the table at child
/// `index` of `parent` and a table beside it (docx-agent's `_separator_of`): its index.
pub(crate) fn separator_of(parent: &Element, index: usize, author: &str) -> Option<usize> {
    let children = parent.children().to_vec();
    for step in [-1isize, 1] {
        let Some(k) = block_sibling(parent, index, step) else {
            continue;
        };
        let Some(Node::Element(paragraph)) = children.get(k) else {
            continue;
        };
        if !paragraph.name().is(Some(W), "p") {
            continue;
        }
        let work = Work::of(paragraph, Vec::new());
        let record = mark_record(&work, work.root());
        if !record.is_some_and(|r| work.is(r, W, "ins") && work.get(r, Some(W), "author") == Some(author)) {
            continue;
        }
        if !work
            .iter(work.root(), W, &["r", "fldSimple", "tbl", "sectPr"])
            .is_empty()
        {
            continue;
        }
        let beyond = block_sibling(parent, k, step).and_then(|b| children.get(b));
        if matches!(beyond, Some(Node::Element(e)) if e.name().is(Some(W), "tbl")) {
            return Some(k);
        }
    }
    None
}

/// The block beside `node` in the work (a section break or the end stopping it).
fn work_sibling(work: &Work, node: Id, forward: bool) -> Option<Id> {
    let mut at = if forward { work.next(node) } else { work.previous(node) };
    while let Some(n) = at {
        if work.is_any(n, W, &["p", "tbl", "sdt", "customXml"]) {
            return Some(n);
        }
        if work.is(n, W, "sectPr") {
            return None;
        }
        at = if forward { work.next(n) } else { work.previous(n) };
    }
    None
}

/// Drop the separator paragraph once nothing needs it (docx-agent's `_drop_separator`):
/// not between two tables, not its container's last block. Whether it went.
pub(crate) fn drop_separator(work: &mut Work, paragraph: Id) -> bool {
    if work.parent(paragraph).is_none() {
        return false;
    }
    let before = work_sibling(work, paragraph, false);
    let after = work_sibling(work, paragraph, true);
    let between_tables = before.is_some_and(|b| work.is(b, W, "tbl")) && after.is_some_and(|a| work.is(a, W, "tbl"));
    if after.is_none() || between_tables {
        return false;
    }
    rehome_markers(work, paragraph);
    work.remove(paragraph);
    true
}
