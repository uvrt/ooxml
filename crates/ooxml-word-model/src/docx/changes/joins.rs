//! Owns: the joins of [`super::Grouping`] across paragraphs (touching records, the marks
//! between paragraphs, Word's property change on paragraphs added at a story's end) and in
//! tables (each structure edit with what it made: rows, columns, merges, splits, a grid
//! change with its table's change) -- docx-agent's `_part_changes` and `_table_joins`.
//!
//! Must not: group anything outside one part.

use super::{container, is_mark, Grouping};
use crate::docx::tables::{CellSpec, RowSpec, VMerge};
use crate::ids::W;
use std::collections::{BTreeMap, HashMap};

/// What a paragraph's walk passes through to its runs.
const TRANSPARENT: [&str; 8] = [
    "hyperlink",
    "smartTag",
    "customXml",
    "fldSimple",
    "sdt",
    "sdtContent",
    "dir",
    "bdo",
];

/// Run content a reader sees.
const CONTENT: [&str; 16] = [
    "t",
    "delText",
    "tab",
    "br",
    "cr",
    "drawing",
    "pict",
    "object",
    "sym",
    "noBreakHyphen",
    "softHyphen",
    "footnoteReference",
    "endnoteReference",
    "ptab",
    "instrText",
    "fldChar",
];

const PROPERTY_KINDS: [&str; 3] = ["row-properties", "cell-properties", "table-properties"];

/// A table's rows, records by cell and rows' own records.
struct Shape<'a> {
    rows: &'a [usize],
    by_cell: &'a HashMap<usize, Vec<usize>>,
    row_record: &'a HashMap<usize, usize>,
}

/// Where a walk over one row's cells is.
struct RowWalk {
    k: usize,
    row: usize,
    previous_kept: Option<usize>,
    merged_away: Option<usize>,
    chain: HashMap<i64, usize>,
    chain_now: HashMap<i64, usize>,
}

/// The deleted and new cells, by grid column.
#[derive(Default)]
struct Columns {
    deleted: BTreeMap<i64, Vec<usize>>,
    new: BTreeMap<i64, Vec<usize>>,
}

/// A paragraph's content in order: `true` for a revision container, `false` for a run a
/// reader sees.
type Tokens = Vec<(bool, usize)>;

impl Grouping<'_> {
    fn tokens(&self, paragraph: usize) -> Tokens {
        let mut out = Vec::new();
        self.walk_tokens(paragraph, &mut out);
        out
    }

    fn walk_tokens(&self, node: usize, out: &mut Tokens) {
        for child in self.arena.children(node) {
            let e = self.arena.element(child);
            if e.name().namespace() != Some(W) || e.name().local() == "pPr" {
                if e.name().namespace() != Some(W) {
                    continue;
                }
                continue;
            }
            let local = e.name().local();
            if container(e).is_some() {
                out.push((true, child));
            } else if local == "r" {
                let content = self
                    .arena
                    .children(child)
                    .iter()
                    .any(|c| CONTENT.iter().any(|t| self.arena.is(*c, t)));
                if content {
                    out.push((false, child));
                }
            } else if TRANSPARENT.contains(&local) {
                self.walk_tokens(child, out);
            }
        }
    }

    /// Paragraphs: touching records, and the marks between paragraphs.
    pub(super) fn paragraph_joins(&mut self) {
        let paragraphs: Vec<usize> = (0..self.arena.nodes.len()).filter(|k| self.arena.is(*k, "p")).collect();
        let mut marks: HashMap<usize, usize> = HashMap::new();
        let mut changes: HashMap<usize, usize> = HashMap::new();
        for &p in &paragraphs {
            let Some(properties) = self.arena.find(p, "pPr") else {
                continue;
            };
            if let Some(mark) = self.arena.find(properties, "rPr") {
                for child in self.arena.children(mark) {
                    if self.key.contains_key(&child) && is_mark(&self.arena, child) {
                        marks.insert(p, child);
                    }
                }
            }
            if let Some(change) = self
                .arena
                .find(properties, "pPrChange")
                .filter(|c| self.key.contains_key(c))
            {
                changes.insert(p, change);
            }
        }
        let mut first: HashMap<usize, Option<usize>> = HashMap::new();
        let mut empty: HashMap<usize, bool> = HashMap::new();
        for &p in &paragraphs {
            let tokens = self.tokens(p);
            empty.insert(p, tokens.is_empty());
            let first_rec = tokens.first().filter(|t| t.0).map(|t| t.1);
            let last_rec = tokens.last().filter(|t| t.0).map(|t| t.1);
            first.insert(p, first_rec);
            for pair in tokens.windows(2) {
                if let [(true, a), (true, b)] = pair {
                    self.join(*a, *b, false);
                }
            }
            let mark = marks.get(&p).copied();
            if let (Some(mark), Some(last)) = (mark, last_rec) {
                if !self.claimed.contains(&mark) {
                    self.join(last, mark, true);
                }
            }
            if let Some(&change) = changes.get(&p) {
                for candidate in [mark, first_rec, last_rec].into_iter().flatten() {
                    if self.who(candidate) == self.who(change)
                        && container(self.arena.element(candidate)) == Some("ins")
                    {
                        self.join(candidate, change, false);
                        break;
                    }
                }
            }
        }
        for pair in paragraphs.windows(2) {
            let [previous, paragraph] = pair else { continue };
            let Some(&mark) = marks.get(previous) else { continue };
            let (before, after) = (
                self.arena.nearest(*previous, "tc"),
                self.arena.nearest(*paragraph, "tc"),
            );
            if before.is_some() && after.is_some() && before != after {
                continue;
            }
            let is_empty = empty.get(paragraph).copied().unwrap_or(false);
            if let Some(start) = first.get(paragraph).copied().flatten() {
                self.join(mark, start, true);
            } else if is_empty && marks.contains_key(paragraph) {
                if let Some(&next) = marks.get(paragraph) {
                    self.join(mark, next, true);
                }
            } else if is_empty {
                if let Some(&change) = changes.get(paragraph) {
                    self.join(mark, change, false);
                }
            }
        }
    }

    fn nearest_is(&self, k: usize, local: &str, owner: usize) -> bool {
        self.arena.nearest(k, local) == Some(owner)
    }

    fn sign(&self, k: usize) -> Option<&'static str> {
        let e = self.arena.element(k);
        if e.name().is(Some(W), "cellIns") {
            return Some("ins");
        }
        if e.name().is(Some(W), "cellDel") {
            return Some("del");
        }
        if e.name().is(Some(W), "moveFrom") || e.name().is(Some(W), "moveTo") {
            return None;
        }
        container(e)
    }

    fn property(&self, k: usize) -> bool {
        let kind = self.kind(k);
        PROPERTY_KINDS.iter().any(|p| kind.starts_with(p))
    }

    fn label(&mut self, k: usize, what: &'static str) {
        self.labels.entry(k).or_insert(what);
    }

    /// A row's own cells with the grid column each starts at (the grid's value rule:
    /// [`CellSpec`], [`RowSpec`]).
    fn row_cells(&self, row: usize) -> Vec<(usize, i64)> {
        let raw = |at: usize, holder: &str, name: &str| {
            self.arena
                .find(at, holder)
                .and_then(|p| self.arena.find(p, name))
                .and_then(|g| self.arena.attr(g, "val"))
        };
        let before = RowSpec::new(raw(row, "trPr", "gridBefore"), None, Vec::new()).before;
        let mut at = i64::try_from(before).unwrap_or(i64::MAX);
        let mut out = Vec::new();
        for cell in self.arena.subtree(row).into_iter().skip(1) {
            if !self.arena.is(cell, "tc") || !self.nearest_is(cell, "tr", row) {
                continue;
            }
            out.push((cell, at));
            let span = CellSpec::new(raw(cell, "tcPr", "gridSpan"), VMerge::None).span;
            at = at.saturating_add(i64::try_from(span).unwrap_or(i64::MAX));
        }
        out
    }

    /// The text of the containers `w:t` and `w:delText` below `k`.
    pub(super) fn texts(&self, k: usize) -> String {
        self.arena
            .subtree(k)
            .into_iter()
            .filter(|j| self.arena.is(*j, "t") || self.arena.is(*j, "delText"))
            .map(|j| crate::docx::text::leading_text(self.arena.element(j)))
            .collect()
    }

    /// The cell's paragraphs' text as it stands (deleted text left out).
    fn visible_texts(&self, cell: usize) -> Vec<String> {
        let mut out = Vec::new();
        for p in self.arena.subtree(cell) {
            if !self.arena.is(p, "p") || !self.nearest_is(p, "tc", cell) {
                continue;
            }
            let text: String = self
                .arena
                .subtree(p)
                .into_iter()
                .filter(|t| self.arena.is(*t, "t") && !self.deleted_above(*t))
                .map(|t| crate::docx::text::leading_text(self.arena.element(t)))
                .collect();
            out.push(text);
        }
        out
    }

    fn deleted_above(&self, k: usize) -> bool {
        let mut at = self.arena.parent(k);
        while let Some(p) = at {
            if self.arena.is(p, "del") || self.arena.is(p, "moveFrom") {
                return true;
            }
            at = self.arena.parent(p);
        }
        false
    }

    fn mark_record(&self, paragraph: usize) -> Option<usize> {
        let mark = self
            .arena
            .find(paragraph, "pPr")
            .and_then(|p| self.arena.find(p, "rPr"))?;
        self.arena
            .children(mark)
            .into_iter()
            .find(|c| self.arena.is(*c, "ins") || self.arena.is(*c, "del"))
    }

    fn join_copies(&mut self, record: usize, target: &[usize], moved: &[String]) {
        for &element in target {
            if self.sign(element) != Some("ins") {
                continue;
            }
            if self.kind(element) == "paragraph-mark-insertion" {
                self.claimed.insert(element);
                self.join(record, element, false);
            } else if !moved.is_empty() && moved.contains(&self.texts(element)) {
                self.join(record, element, false);
            }
        }
    }

    /// A table's structure records joined with what each edit made with them.
    pub(super) fn table_joins(&mut self, table: usize) {
        let inside: Vec<usize> = self
            .arena
            .subtree(table)
            .into_iter()
            .filter(|e| self.key.contains_key(e) && self.nearest_is(*e, "tbl", table))
            .collect();
        if inside.is_empty() {
            return;
        }
        let rows: Vec<usize> = self
            .arena
            .subtree(table)
            .into_iter()
            .filter(|r| self.arena.is(*r, "tr") && self.nearest_is(*r, "tbl", table))
            .collect();
        let mut by_cell: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut by_row: HashMap<usize, Vec<usize>> = HashMap::new();
        for &element in &inside {
            if let Some(cell) = self
                .arena
                .nearest(element, "tc")
                .filter(|c| self.nearest_is(*c, "tbl", table))
            {
                by_cell.entry(cell).or_default().push(element);
            }
            if let Some(row) = self.arena.nearest(element, "tr").filter(|r| rows.contains(r)) {
                by_row.entry(row).or_default().push(element);
            }
        }
        let properties: Vec<usize> = inside.iter().copied().filter(|e| self.property(*e)).collect();
        for pair in properties.windows(2) {
            if let [a, b] = pair {
                self.join(*a, *b, false);
            }
        }
        let row_record = self.row_records(&rows, &by_row);
        self.cells(&rows, &by_cell, &row_record);
        self.grid_changes(&inside);
    }

    fn row_records(&mut self, rows: &[usize], by_row: &HashMap<usize, Vec<usize>>) -> HashMap<usize, usize> {
        let mut row_record = HashMap::new();
        for &row in rows {
            let record = by_row.get(&row).and_then(|list| {
                list.iter().copied().find(|e| {
                    let kind = self.kind(*e);
                    (kind == "row-insertion" || kind == "row-deletion")
                        && self.arena.parent(*e).and_then(|p| self.arena.parent(p)) == Some(row)
                })
            });
            let Some(record) = record else { continue };
            row_record.insert(row, record);
            let what = if self.sign(record) == Some("ins") {
                "row insertion"
            } else {
                "row deletion"
            };
            self.label(record, what);
            for element in by_row.get(&row).cloned().unwrap_or_default() {
                if element != record && self.sign(element) == self.sign(record) {
                    self.join(record, element, false);
                }
            }
        }
        for pair in rows.windows(2) {
            let [upper, lower] = pair else { continue };
            if let (Some(&a), Some(&b)) = (row_record.get(upper), row_record.get(lower)) {
                if self.sign(a) == self.sign(b) {
                    self.join(a, b, false);
                }
            }
        }
        row_record
    }

    fn cells(&mut self, rows: &[usize], by_cell: &HashMap<usize, Vec<usize>>, row_record: &HashMap<usize, usize>) {
        let shape = Shape {
            rows,
            by_cell,
            row_record,
        };
        let mut columns = Columns::default();
        let mut merge_chain: HashMap<i64, usize> = HashMap::new();
        for (k, &row) in rows.iter().enumerate() {
            let mut walk = RowWalk {
                k,
                row,
                previous_kept: None,
                merged_away: None,
                chain: merge_chain,
                chain_now: HashMap::new(),
            };
            for (cell, column) in self.row_cells(row) {
                self.cell(&shape, &mut walk, &mut columns, cell, column);
            }
            merge_chain = walk.chain_now;
        }
        for group in columns.deleted.values().chain(columns.new.values()) {
            for pair in group.windows(2) {
                if let [a, b] = pair {
                    self.join(*a, *b, false);
                }
            }
        }
    }

    /// One cell of a row: a deleted column's, a merged-away one, a vertical merge's, a new
    /// one (a column's or a split's).
    fn cell(&mut self, shape: &Shape<'_>, walk: &mut RowWalk, columns: &mut Columns, cell: usize, column: i64) {
        let records = shape.by_cell.get(&cell).cloned().unwrap_or_default();
        let own: Vec<usize> = records
            .iter()
            .copied()
            .filter(|e| {
                let p = self.arena.parent(*e);
                p.is_some_and(|p| self.arena.is(p, "tcPr") && self.arena.parent(p) == Some(cell))
            })
            .collect();
        let mut deletions: Vec<usize> = own.iter().copied().filter(|e| self.arena.is(*e, "cellDel")).collect();
        let insertion = own.iter().copied().find(|e| self.arena.is(*e, "cellIns"));
        let vertical = own.iter().copied().find(|e| self.arena.is(*e, "cellMerge"));
        let content: Vec<usize> = records
            .iter()
            .copied()
            .filter(|e| !own.contains(e) && !self.property(*e))
            .collect();
        let last = self
            .arena
            .subtree(cell)
            .into_iter()
            .rev()
            .find(|p| self.arena.is(*p, "p") && self.nearest_is(*p, "tc", cell));
        let mark = last.and_then(|p| self.mark_record(p));
        let new_mark = mark.filter(|m| self.key.contains_key(m) && self.sign(*m) == Some("ins"));
        if !deletions.is_empty() && (content.iter().any(|e| self.sign(*e) == Some("del")) || new_mark.is_some()) {
            if let Some(deletion) = deletions.pop() {
                self.label(deletion, "column deletion");
                self.join_signed(deletion, &content, "del");
                columns.deleted.entry(column).or_default().push(deletion);
            }
        }
        if let Some(&deletion) = deletions.first() {
            self.merged_away(shape, walk, cell, deletion);
            return;
        }
        if own.iter().any(|e| self.arena.is(*e, "cellDel")) && new_mark.is_none() {
            walk.merged_away = None;
            return;
        }
        walk.merged_away = None;
        if let Some(vertical) = vertical {
            self.vertical(shape, walk, vertical, column, &content);
        }
        let anchor = insertion.or(new_mark);
        if let Some(anchor) = anchor.filter(|_| !shape.row_record.contains_key(&walk.row)) {
            self.new_cell(shape, walk, columns, anchor, column, &content);
        }
        if anchor.is_none() {
            walk.previous_kept = Some(cell);
        }
        self.split_row(shape, walk, cell, column);
    }

    fn join_signed(&mut self, record: usize, content: &[usize], sign: &str) {
        for &element in content {
            if self.sign(element) == Some(sign) {
                self.join(record, element, false);
            }
        }
    }

    /// A merged-away cell: its text kept, a copy inserted in the cell it went into.
    fn merged_away(&mut self, shape: &Shape<'_>, walk: &mut RowWalk, cell: usize, deletion: usize) {
        self.label(deletion, "merge");
        if let Some(away) = walk.merged_away {
            self.join(away, deletion, false);
        }
        walk.merged_away = Some(deletion);
        if let Some(kept) = walk.previous_kept {
            let texts: Vec<String> = self.visible_texts(cell).into_iter().filter(|t| !t.is_empty()).collect();
            let target = shape.by_cell.get(&kept).cloned().unwrap_or_default();
            self.join_copies(deletion, &target, &texts);
        }
    }

    /// A vertical merge's record down its column, with the text it moved.
    fn vertical(&mut self, shape: &Shape<'_>, walk: &mut RowWalk, vertical: usize, column: i64, content: &[usize]) {
        self.label(vertical, "merge");
        let continued = self.arena.attr(vertical, "vMerge").unwrap_or("") == "cont";
        match walk.chain.get(&column).copied() {
            Some(head) if continued => {
                self.join(head, vertical, false);
                let moved: Vec<String> = content
                    .iter()
                    .filter(|e| self.sign(**e) == Some("del"))
                    .map(|e| self.texts(*e))
                    .filter(|t| !t.is_empty())
                    .collect();
                self.join_signed(vertical, content, "del");
                if let Some(head_cell) = self.arena.nearest(head, "tc") {
                    let target = shape.by_cell.get(&head_cell).cloned().unwrap_or_default();
                    self.join_copies(head, &target, &moved);
                }
                walk.chain_now.insert(column, head);
            }
            _ => {
                walk.chain_now.insert(column, vertical);
            }
        }
    }

    /// A new cell: a column's, or a split's (next to the split cell).
    fn new_cell(
        &mut self,
        shape: &Shape<'_>,
        walk: &RowWalk,
        columns: &mut Columns,
        anchor: usize,
        column: i64,
        content: &[usize],
    ) {
        self.label(anchor, "column insertion");
        self.join_signed(anchor, content, "ins");
        let beside: Vec<usize> = walk
            .previous_kept
            .map(|c| shape.by_cell.get(&c).cloned().unwrap_or_default())
            .unwrap_or_default()
            .into_iter()
            .filter(|e| self.kind(*e) == "cell-properties")
            .collect();
        if let Some(&first) = beside.first() {
            self.labels.insert(anchor, "split");
            self.label(first, "split");
            self.join(first, anchor, false);
        } else {
            columns.new.entry(column).or_default().push(anchor);
        }
    }

    /// A row a split inserted: its cells continue a vertical merge from the row above.
    fn split_row(&mut self, shape: &Shape<'_>, walk: &RowWalk, cell: usize, column: i64) {
        let Some(&record) = shape.row_record.get(&walk.row) else {
            return;
        };
        if self.sign(record) != Some("ins") || walk.k == 0 {
            return;
        }
        let continuing = self.arena.find(cell, "tcPr").and_then(|p| self.arena.find(p, "vMerge"));
        let Some(continuing) = continuing else { return };
        let value = self
            .arena
            .attr(continuing, "val")
            .filter(|v| !v.is_empty())
            .unwrap_or("continue");
        if value != "continue" {
            return;
        }
        let Some(&upper) = shape.rows.get(walk.k - 1) else {
            return;
        };
        let above = self
            .row_cells(upper)
            .into_iter()
            .find(|(_, at)| *at == column)
            .map(|(c, _)| c);
        let changed: Vec<usize> = above
            .map(|c| shape.by_cell.get(&c).cloned().unwrap_or_default())
            .unwrap_or_default()
            .into_iter()
            .filter(|e| self.kind(*e) == "cell-properties")
            .collect();
        if let Some(&first) = changed.first() {
            self.labels.insert(record, "split");
            self.label(first, "split");
            self.join(first, record, false);
        }
    }

    /// A grid change (no author or date) with its table's change: its `w:tblPrChange`, its
    /// other property records, or the latest of its row and cell records.
    fn grid_changes(&mut self, inside: &[usize]) {
        for &record in inside {
            if !self.arena.is(record, "tblGridChange") || self.who(record) != (None, None) {
                continue;
            }
            let mine: Vec<usize> = inside
                .iter()
                .copied()
                .filter(|e| {
                    let kind = self.kind(*e);
                    *e != record
                        && self.arena.attr(*e, "author").is_some()
                        && ["row-", "cell-", "table-"].iter().any(|p| kind.starts_with(p))
                })
                .collect();
            if mine.is_empty() {
                continue;
            }
            let table_level = mine.iter().copied().rev().find(|e| self.arena.is(*e, "tblPrChange"));
            let other = mine.iter().copied().find(|e| self.property(*e));
            let latest = mine.iter().copied().fold(None::<usize>, |best, e| match best {
                Some(b) if self.arena.attr(b, "date").unwrap_or("") >= self.arena.attr(e, "date").unwrap_or("") => {
                    Some(b)
                }
                _ => Some(e),
            });
            let Some(partner) = table_level.or(other).or(latest) else {
                continue;
            };
            if let (Some(&a), Some(&b)) = (self.key.get(&partner), self.key.get(&record)) {
                self.union.union(a, b);
            }
        }
    }
}
