//! Owns: [`PartReview`], one accept or reject over a set of records in one story part, as
//! docx-agent's `_Review` makes it (`revisions/review.py`): the records that go with those
//! asked for ([`PartReview::expand`]), then each kind applied in docx-agent's order --
//! rows, cells, content containers, property changes, grids, other records, move markers,
//! then paragraph marks in document order -- and the tidying after ([`super::grid`]).
//!
//! Must not: read or write outside its part's tree, or decide which records a call
//! selects (the command does).

use super::xml::{
    ancestor, attr, find_all, find_path, insert_in_order, is_record, kind_of, made_with, make, set_attr, RecordKind,
    CELL, DELETED, INSERTED,
};
use crate::names::W;
use crate::validate::Orders;
use crate::work::{Id, Work};
use std::collections::{BTreeSet, HashMap, HashSet};

/// A table's rows as the review found them: each row with its cells' first grid column
/// and span.
pub(crate) type GridMap = Vec<GridRow>;

/// A row with its cells' first grid column and span.
pub(crate) type GridRow = (Id, Vec<(Id, i64, i64)>);

/// What an inline control shows when it holds nothing and names no placeholder text: five
/// spaces (Word 16.106's Reject All of `controls.docx`'s inserted-only control 309).
const PLACEHOLDER: &str = "     ";

/// What a control's content shows: text, a tab or break, an object, a field, a deleted
/// text still pending.
const SHOWN: [&str; 17] = [
    "t",
    "delText",
    "tab",
    "ptab",
    "br",
    "cr",
    "sym",
    "noBreakHyphen",
    "softHyphen",
    "drawing",
    "pict",
    "object",
    "footnoteReference",
    "endnoteReference",
    "fldChar",
    "fldSimple",
    "instrText",
];

/// An element of another vocabulary (an equation, a choice of content): shown.
fn foreign(work: &Work, node: Id) -> bool {
    work.is_element(node) && work.name(node).is_some_and(|n| n.namespace() != Some(W))
}

/// The inline content control (`w:sdt` in a paragraph) `node` stands directly in.
fn inline_control(work: &Work, node: Id) -> Option<Id> {
    let content = work.parent(node).filter(|c| work.is(*c, W, "sdtContent"))?;
    let sdt = work.parent(content).filter(|s| work.is(*s, W, "sdt"))?;
    work.parent(sdt).filter(|p| work.is(*p, W, "p")).map(|_| sdt)
}

/// One accept or reject over records of one part.
pub(crate) struct PartReview<'w, 'o> {
    pub(crate) work: &'w mut Work,
    pub(crate) accept: bool,
    pub(crate) orders: &'o Orders,
    /// Relationship ids of what the review removed.
    pub(crate) released: Vec<String>,
    /// Paragraphs the review removed (joined ones too).
    pub(crate) removed_paragraphs: Vec<Id>,
    /// Paragraphs joined: (the one that went, the one it went into).
    pub(crate) joined: Vec<(Id, Id)>,
    /// Tables whose cells' records it applied.
    pub(crate) tables: HashSet<Id>,
    /// Every table's grid as the review found it.
    pub(crate) maps: HashMap<Id, GridMap>,
    /// Tables holding a record this review settles, outer ones too.
    pub(crate) touched_tables: Vec<Id>,
    /// Comments (`w:id`) whose reference the review removed.
    pub(crate) dropped_comments: BTreeSet<String>,
    /// Notes (`footnote`/`endnote`, `w:id`) whose reference the review removed.
    pub(crate) dropped_notes: BTreeSet<(String, String)>,
    /// Cells removed, by the table they were removed from (as the review found it).
    pub(crate) removed_cells: HashMap<Id, HashSet<Id>>,
    /// The tables of `removed_cells`, in the order first named.
    pub(crate) removed_order: Vec<Id>,
    /// Tables whose grid a rejected grid change set back whole.
    pub(crate) restored_grids: HashSet<Id>,
    /// Tables whose cells' spans a rejected cell change set back.
    pub(crate) respanned: Vec<Id>,
    /// Rows whose cells' only paragraph's deleted mark was accepted, with those cells.
    pub(crate) emptied: Vec<(Id, HashSet<Id>)>,
    /// The table each part of a split table came from.
    pub(crate) origin: HashMap<Id, Id>,
    /// Which parts of a split table share a grid.
    pub(crate) grid_group: HashMap<Id, Id>,
    /// Tables a split made, in the order made.
    pub(crate) split_parts: Vec<Id>,
    /// Text a row's last cell gave up, moved out after the row, with its paragraphs.
    pub(crate) moved_out: Vec<(String, Vec<Id>)>,
    /// Rows that gave up a cell to a deleted mark at a cell's end.
    pub(crate) edged_rows: HashSet<Id>,
    /// The record groups of the part as they are listed (its logical changes).
    pub(crate) groups: HashMap<Id, Vec<Id>>,
    /// Inline content controls the review took content out of, in the order met.
    pub(crate) emptied_controls: Vec<Id>,
}

impl<'w, 'o> PartReview<'w, 'o> {
    /// A review of `work` (a story part's root), the part's record groups `groups`.
    pub(crate) fn new(work: &'w mut Work, accept: bool, orders: &'o Orders, groups: HashMap<Id, Vec<Id>>) -> Self {
        let mut maps = HashMap::new();
        for table in work.iter(work.root(), W, &["tbl"]) {
            maps.insert(table, super::grid::grid_map(work, table));
        }
        PartReview {
            work,
            accept,
            orders,
            released: Vec::new(),
            removed_paragraphs: Vec::new(),
            joined: Vec::new(),
            tables: HashSet::new(),
            maps,
            touched_tables: Vec::new(),
            dropped_comments: BTreeSet::new(),
            dropped_notes: BTreeSet::new(),
            removed_cells: HashMap::new(),
            removed_order: Vec::new(),
            restored_grids: HashSet::new(),
            respanned: Vec::new(),
            emptied: Vec::new(),
            origin: HashMap::new(),
            grid_group: HashMap::new(),
            split_parts: Vec::new(),
            moved_out: Vec::new(),
            edged_rows: HashSet::new(),
            groups,
            emptied_controls: Vec::new(),
        }
    }

    /// The cells removed from `table`, made on first use.
    pub(crate) fn removed_from(&mut self, table: Id) -> &mut HashSet<Id> {
        if !self.removed_cells.contains_key(&table) {
            self.removed_order.push(table);
        }
        self.removed_cells.entry(table).or_default()
    }

    // -- grouping --------------------------------------------------------------------------

    /// The records that go with `elements`: a move's both ends and markers, a row's, cell's
    /// or table's records made with it, a paragraph's content with its mark.
    pub(crate) fn expand(&mut self, elements: &[Id]) -> Vec<Id> {
        let mut out: Vec<Id> = Vec::new();
        let mut seen: HashSet<Id> = HashSet::new();
        let mut add = |node: Id, out: &mut Vec<Id>| {
            if seen.insert(node) {
                out.push(node);
            }
        };
        let order: HashMap<Id, usize> = self
            .work
            .descendants(self.work.root())
            .into_iter()
            .enumerate()
            .map(|(k, n)| (n, k))
            .collect();
        for &element in elements {
            add(element, &mut out);
            let kind = kind_of(self.work, element);
            let mut table = ancestor(self.work, element, "tbl");
            while let Some(t) = table {
                if !self.touched_tables.contains(&t) {
                    self.touched_tables.push(t);
                }
                table = self.work.parent(t).and_then(|outer| ancestor(self.work, outer, "tbl"));
            }
            let table = ancestor(self.work, element, "tbl");
            let grid_change = |work: &Work, table: Id| find_path(work, table, &["tblGrid", "tblGridChange"]);
            if let Some(t) = table {
                if !self.work.is(element, W, "tblGridChange")
                    && matches!(
                        kind,
                        RecordKind::Cell | RecordKind::Row | RecordKind::Mark | RecordKind::Property
                    )
                {
                    if let Some(grid) = grid_change(self.work, t) {
                        add(grid, &mut out);
                    }
                }
            }
            if self.work.is_any(element, W, &["moveFrom", "moveTo"]) {
                for node in self.move_of(element, &order) {
                    add(node, &mut out);
                }
            } else if kind == RecordKind::Row {
                let row = self.work.parent(element).and_then(|p| self.work.parent(p));
                for node in row.map(|r| self.records(r)).unwrap_or_default() {
                    if made_with(self.work, node, element) {
                        add(node, &mut out);
                    }
                }
            } else if matches!(kind, RecordKind::Cell | RecordKind::Property)
                && (self.work.is(element, W, "tcPrChange") || self.work.is_any(element, W, &CELL))
            {
                if let Some(t) = table {
                    for node in self.groups.get(&element).cloned().unwrap_or_else(|| vec![element]) {
                        add(node, &mut out);
                    }
                    if let Some(grid) = grid_change(self.work, t) {
                        add(grid, &mut out);
                    }
                }
            } else if kind == RecordKind::Mark {
                let paragraph = ancestor(self.work, element, "p");
                for node in paragraph.map(|p| self.records(p)).unwrap_or_default() {
                    if kind_of(self.work, node) == RecordKind::Run && made_with(self.work, node, element) {
                        add(node, &mut out);
                    }
                }
            }
        }
        out
    }

    /// Every record at or below `scope`, in document order.
    fn records(&self, scope: Id) -> Vec<Id> {
        self.work
            .descendants(scope)
            .into_iter()
            .filter(|n| is_record(self.work, *n))
            .collect()
    }

    /// Every record and range marker of the move `element` belongs to.
    fn move_of(&self, element: Id, order: &HashMap<Id, usize>) -> Vec<Id> {
        let Some(name) = self.move_name(element, order) else {
            return Vec::new();
        };
        let all = self.work.descendants(self.work.root());
        let mut out = Vec::new();
        for (start_tag, end_tag) in [
            ("moveFromRangeStart", "moveFromRangeEnd"),
            ("moveToRangeStart", "moveToRangeEnd"),
        ] {
            for start in all.iter().copied().filter(|n| self.work.is(*n, W, start_tag)) {
                if attr(self.work, start, "name") != Some(name.as_str()) {
                    continue;
                }
                let start_id = attr(self.work, start, "id");
                let end = all
                    .iter()
                    .copied()
                    .find(|n| self.work.is(*n, W, end_tag) && attr(self.work, *n, "id") == start_id);
                out.push(start);
                if let Some(end) = end {
                    out.push(end);
                }
                let low = order.get(&start).copied().unwrap_or(0);
                let high = end.and_then(|e| order.get(&e).copied()).unwrap_or(low);
                let inside: Vec<Id> = all
                    .iter()
                    .copied()
                    .filter(|n| order.get(n).is_some_and(|k| (low..=high).contains(k)))
                    .collect();
                out.extend(
                    inside
                        .iter()
                        .copied()
                        .filter(|n| self.work.is_any(*n, W, &["moveFrom", "moveTo"])),
                );
                // the marks of the paragraphs the range runs through move with it
                let mut paragraphs: Vec<Id> = Vec::new();
                for n in &inside {
                    let p = if self.work.is(*n, W, "p") {
                        Some(*n)
                    } else {
                        ancestor(self.work, *n, "p")
                    };
                    if let Some(p) = p.filter(|p| !paragraphs.contains(p)) {
                        paragraphs.push(p);
                    }
                }
                for p in paragraphs {
                    if let Some(record) = super::xml::mark_record(self.work, p) {
                        if self.work.is_any(record, W, &["moveFrom", "moveTo"]) {
                            out.push(record);
                        }
                    }
                }
            }
        }
        out
    }

    /// The name of the move range `element` is in.
    fn move_name(&self, element: Id, order: &HashMap<Id, usize>) -> Option<String> {
        let position = *order.get(&element)?;
        let from = self.work.is(element, W, "moveFrom");
        let (start_tag, end_tag) = if from {
            ("moveFromRangeStart", "moveFromRangeEnd")
        } else {
            ("moveToRangeStart", "moveToRangeEnd")
        };
        let all = self.work.descendants(self.work.root());
        let mut best = None;
        for start in all.iter().copied().filter(|n| self.work.is(*n, W, start_tag)) {
            if order.get(&start).copied().unwrap_or(usize::MAX) > position {
                continue;
            }
            let start_id = attr(self.work, start, "id");
            let end = all
                .iter()
                .copied()
                .find(|n| self.work.is(*n, W, end_tag) && attr(self.work, *n, "id") == start_id);
            if end.is_none_or(|e| order.get(&e).is_some_and(|k| *k >= position)) {
                best = Some(attr(self.work, start, "name").unwrap_or("").to_owned());
            }
        }
        best
    }

    // -- applying ----------------------------------------------------------------------------

    /// Apply the records `elements` (expanded) in docx-agent's order, then tidy.
    pub(crate) fn run(&mut self, elements: &[Id]) {
        let elements: Vec<Id> = elements
            .iter()
            .copied()
            .filter(|e| self.work.parent(*e).is_some())
            .collect();
        let of = |kind: RecordKind| -> Vec<Id> {
            elements
                .iter()
                .copied()
                .filter(|e| kind_of(self.work, *e) == kind)
                .collect()
        };
        let rows = of(RecordKind::Row);
        let cells = of(RecordKind::Cell);
        let runs = of(RecordKind::Run);
        let marks = of(RecordKind::Mark);
        let properties: Vec<Id> = of(RecordKind::Property)
            .into_iter()
            .filter(|e| !self.work.is(*e, W, "tblGridChange"))
            .collect();
        let grids: Vec<Id> = elements
            .iter()
            .copied()
            .filter(|e| self.work.is(*e, W, "tblGridChange"))
            .collect();
        let others = of(RecordKind::Other);
        let markers: Vec<Id> = elements
            .iter()
            .copied()
            .filter(|e| {
                self.work.is_any(
                    *e,
                    W,
                    &[
                        "moveFromRangeStart",
                        "moveFromRangeEnd",
                        "moveToRangeStart",
                        "moveToRangeEnd",
                    ],
                )
            })
            .collect();
        for element in rows {
            self.row(element);
        }
        self.cells(&cells);
        for element in runs {
            self.container(element);
        }
        for element in properties {
            self.property(element);
        }
        for element in grids {
            self.grid(element);
        }
        for element in others {
            if self.work.parent(element).is_some() {
                self.drop_record(element);
            }
        }
        for element in markers {
            if self.work.parent(element).is_some() {
                self.work.remove(element);
            }
        }
        let order: HashMap<Id, usize> = self
            .work
            .descendants(self.work.root())
            .into_iter()
            .enumerate()
            .map(|(k, n)| (n, k))
            .collect();
        let mut marks = marks;
        marks.sort_by_key(|e| order.get(e).map_or(-1, |k| i64::try_from(*k).unwrap_or(i64::MAX)));
        for element in marks {
            self.mark(element);
        }
        self.show_placeholders();
        self.tidy();
    }

    /// Every inline control the review emptied shows its placeholder, as Word 16.106's
    /// Accept All and Reject All leave one: `w:showingPlcHdr` set, and -- the control
    /// naming no placeholder text of its own -- five spaces as its content.
    fn show_placeholders(&mut self) {
        for sdt in std::mem::take(&mut self.emptied_controls) {
            if self.work.parent(sdt).is_none() {
                continue;
            }
            let (Some(properties), Some(content)) =
                (self.work.find(sdt, W, "sdtPr"), self.work.find(sdt, W, "sdtContent"))
            else {
                continue;
            };
            let holds = self
                .work
                .descendants(content)
                .into_iter()
                .any(|n| self.work.is_any(n, W, &SHOWN) || foreign(self.work, n));
            if holds {
                continue;
            }
            if self.work.find(properties, W, "showingPlcHdr").is_none() {
                let shown = make(self.work, "showingPlcHdr", &[]);
                insert_in_order(self.work, properties, shown, self.orders);
            }
            let run = make(self.work, "r", &[]);
            let text = make(self.work, "t", &[]);
            crate::text::write::set_spaced_text(self.work, text, PLACEHOLDER);
            self.work.append(run, text);
            self.work.append(content, run);
        }
    }

    /// Remove a record, and the property element it leaves empty.
    pub(crate) fn drop_record(&mut self, record: Id) {
        let parent = self.work.parent(record);
        self.work.remove(record);
        let Some(parent) = parent else { return };
        if self.work.is_any(parent, W, &["rPr", "trPr"]) && self.work.children(parent).is_empty() {
            let owner = self.work.parent(parent);
            self.work.remove(parent);
            if let Some(owner) = owner.filter(|o| self.work.is(*o, W, "pPr")) {
                if self.work.children(owner).is_empty() {
                    self.work.remove(owner);
                }
            }
        }
    }

    fn row(&mut self, record: Id) {
        let Some(row) = self.work.parent(record).and_then(|p| self.work.parent(p)) else {
            return;
        };
        let goes = self.work.is_any(record, W, &DELETED) == self.accept;
        let table = ancestor(self.work, row, "tbl");
        if goes {
            self.released.extend(crate::inline::relationship_ids(self.work, row));
            self.removed_paragraphs.extend(self.work.iter(row, W, &["p"]));
            self.removing(row);
            self.work.remove(row);
            if let Some(table) = table {
                self.remove_if_rowless(table);
            }
        } else {
            self.drop_record(record);
        }
    }

    /// Remove `table` when it has no row left (and no row-level control).
    pub(crate) fn remove_if_rowless(&mut self, table: Id) {
        let rowless = self.work.find(table, W, "tr").is_none()
            && !self
                .work
                .children(table)
                .iter()
                .any(|c| self.work.is_any(*c, W, &["sdt", "customXml"]));
        if rowless {
            self.work.remove(table);
        }
    }

    fn cells(&mut self, records: &[Id]) {
        let mut by_table: Vec<(Id, Vec<Id>)> = Vec::new();
        for &record in records {
            if self.work.parent(record).is_none() {
                continue;
            }
            let Some(table) = ancestor(self.work, record, "tbl") else {
                continue;
            };
            match by_table.iter_mut().find(|(t, _)| *t == table) {
                Some((_, group)) => group.push(record),
                None => by_table.push((table, vec![record])),
            }
        }
        for (table, group) in by_table {
            self.tables.insert(table);
            self.removed_from(table);
            for record in group {
                self.cell(table, record);
            }
        }
    }

    fn cell(&mut self, table: Id, record: Id) {
        let cell = ancestor(self.work, record, "tc");
        if self.work.is(record, W, "cellMerge") {
            let source = if self.accept { "vMerge" } else { "vMergeOrig" };
            let value = match attr(self.work, record, source).unwrap_or("") {
                "rest" => Some("restart"),
                "cont" => Some("continue"),
                _ => None,
            };
            if let Some(properties) = self.work.parent(record) {
                if let Some(current) = self.work.find(properties, W, "vMerge") {
                    self.work.remove(current);
                }
                if let Some(value) = value {
                    let node = make(self.work, "vMerge", &[]);
                    if value == "restart" {
                        set_attr(self.work, node, "val", "restart");
                    }
                    insert_in_order(self.work, properties, node, self.orders);
                }
            }
            self.work.remove(record);
            return;
        }
        let goes = self.work.is(record, W, "cellDel") == self.accept;
        match cell.filter(|_| goes) {
            Some(cell) => {
                let before = (self.accept && super::grid::content_kept(self.work, cell))
                    .then(|| super::grid::cell_before(self.work, cell))
                    .flatten();
                match before {
                    Some(before) => super::grid::absorb(self.work, before, cell, self.orders),
                    None => {
                        self.removed_from(table).insert(cell);
                    }
                }
                self.removing(cell);
                self.released.extend(crate::inline::relationship_ids(self.work, cell));
                self.removed_paragraphs.extend(self.work.iter(cell, W, &["p"]));
                self.work.remove(cell);
            }
            None => self.work.remove(record),
        }
    }

    fn container(&mut self, record: Id) {
        if self.work.parent(record).is_none() {
            return;
        }
        let shown = self.work.is_any(record, W, &INSERTED);
        if shown == self.accept {
            if self.work.is_any(record, W, &DELETED) {
                spell_shown(self.work, record);
            }
            for child in self.work.children(record).to_vec() {
                self.work.add_previous(record, child);
            }
            self.work.remove(record);
        } else {
            self.released.extend(crate::inline::relationship_ids(self.work, record));
            self.removing(record);
            if let Some(sdt) = inline_control(self.work, record) {
                self.emptied_controls.push(sdt);
            }
            self.work.remove(record);
        }
    }

    /// The comments and notes whose references `element` holds go with it.
    pub(crate) fn removing(&mut self, element: Id) {
        for node in self.work.iter(
            element,
            W,
            &["commentReference", "footnoteReference", "endnoteReference"],
        ) {
            self.orphan(node);
        }
    }

    /// A comment or note reference the review removes with its content.
    pub(crate) fn orphan(&mut self, node: Id) {
        let Some(id) = attr(self.work, node, "id").map(str::to_owned) else {
            return;
        };
        if self.work.is(node, W, "commentReference") {
            self.dropped_comments.insert(id);
        } else if self.work.is(node, W, "footnoteReference") {
            self.dropped_notes.insert(("footnote".to_owned(), id));
        } else if self.work.is(node, W, "endnoteReference") {
            self.dropped_notes.insert(("endnote".to_owned(), id));
        }
    }

    fn grid(&mut self, record: Id) {
        let Some(grid) = self.work.parent(record) else { return };
        if self.accept {
            self.work.remove(record);
            return;
        }
        let old = self.work.find(record, W, "tblGrid");
        self.work.remove(record);
        if let Some(table) = self.work.parent(grid) {
            self.restored_grids.insert(table);
        }
        for column in find_all(self.work, grid, "gridCol") {
            self.work.remove(column);
        }
        for column in old.map(|o| find_all(self.work, o, "gridCol")).unwrap_or_default() {
            let copy = self.work.deep_copy(column);
            insert_in_order(self.work, grid, copy, self.orders);
        }
    }
}

/// Spell a deletion's content as shown text: `w:delText` back to `w:t`, `w:delInstrText`
/// to `w:instrText`.
fn spell_shown(work: &mut Work, record: Id) {
    for node in work.iter(record, W, &["delText", "delInstrText"]) {
        let local = if work.is(node, W, "delText") { "t" } else { "instrText" };
        let Some(name) = work.name(node) else { continue };
        let to = std::sync::Arc::new(ooxml_xml::QName::new(name.prefix(), local, Some(W)));
        work.rename(node, to);
    }
}
