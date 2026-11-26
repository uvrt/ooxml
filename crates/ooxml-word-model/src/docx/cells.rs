//! Owns: what the layout model reads of one top-level table ([`TableFacts`]), as
//! docx-agent's `effective.table_model` asks docx2svg for a table alone: each of its
//! paragraphs' cell (table style and regions), each cell's resolved properties (its
//! shading as drawn) and the tables as the model reads them; and the layout model's
//! structural path of an element of the table ([`model_path`]), by which the two are
//! matched -- or of a story part ([`story_model_path`]: a field's run, as the layout names
//! what it draws).
//!
//! A table's reading is kept across a document's versions ([`TableMemo`], in the document's
//! [`super::kept::Kept`]) by the table element and the parts it was read with, both by
//! identity: a version that shares the element and the parts reads the table as before.
//!
//! Must not: read anything outside its table, or keep a reading alive past the elements it
//! was read from (they are held weakly; a reading no version can ask for is swept).

use super::formatting::{Cell, Formatting};
use crate::ids::W;
use crate::model::{Block, Story, Table};
use crate::read::{Fields, Reader};
use crate::resolve::resolve_table;
use crate::value::{text, Props, Text};
use ooxml_derive::{AddressHash, CacheBudget, Limit, Memo, MemoStats, Weigh};
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// The prefix the table's own path is given; every path below it starts with it.
const ROOT: &str = "T";

/// What the layout model reads of one top-level table.
#[derive(Debug, Default)]
pub struct TableFacts {
    /// Each paragraph's cell, by its model path.
    paragraphs: HashMap<Text, Cell>,
    /// Each cell's resolved properties, by its model path.
    cells: HashMap<Text, Props>,
    /// The table and the tables nested in its cells, in document order.
    tables: Vec<Arc<Table>>,
}

impl TableFacts {
    /// The facts of the `w:tbl` `table`.
    #[must_use]
    pub fn read(formatting: &Formatting, table: &Element) -> TableFacts {
        let context = formatting.context();
        let reader = Reader::new(&context.colours);
        let mut fields = Fields::new(true, true);
        let blocks = reader.table_blocks(table, &text(ROOT), &mut fields);
        let mut out = TableFacts::default();
        out.blocks(formatting, &blocks);
        out
    }

    fn blocks(&mut self, formatting: &Formatting, blocks: &[Block]) {
        for block in blocks {
            match block {
                Block::Paragraph(p) => {
                    if let Some(style) = &p.table_style_id {
                        self.paragraphs.insert(
                            p.path.clone(),
                            Cell {
                                style: style.clone(),
                                conditions: Arc::clone(&p.table_conditions),
                            },
                        );
                    }
                }
                Block::Table(t) => self.table(formatting, t),
            }
        }
    }

    fn table(&mut self, formatting: &Formatting, table: &Arc<Table>) {
        self.tables.push(Arc::clone(table));
        let resolved = resolve_table(&formatting.sources(), table);
        for row in resolved.rows {
            for cell in row.cells {
                let cell = Arc::unwrap_or_clone(cell);
                let path = table.cell_paths.get(cell.row).and_then(|r| r.get(cell.index)).cloned();
                let path = path.unwrap_or_else(|| {
                    text(&format!(
                        "{}/w:tr[{}]/w:tc[{}]",
                        table.path,
                        cell.row + 1,
                        cell.index + 1
                    ))
                });
                self.cells.insert(path, cell.properties);
            }
        }
        for row in table.rows.iter() {
            for story in row.iter() {
                self.story(formatting, story);
            }
        }
    }

    fn story(&mut self, formatting: &Formatting, story: &Story) {
        self.blocks(formatting, story);
    }

    /// The cell of the paragraph whose model path is `path`.
    #[must_use]
    pub fn cell(&self, path: &str) -> Option<&Cell> {
        self.paragraphs.get(path)
    }

    /// The table and the tables nested in its cells as the layout model reads them, in
    /// document order (a table before those in its cells).
    #[must_use]
    pub fn tables(&self) -> &[Arc<Table>] {
        &self.tables
    }

    /// The resolved properties of the cell whose model path is `path`.
    #[must_use]
    pub fn cell_properties(&self, path: &str) -> Option<&Props> {
        self.cells.get(path)
    }
}

/// The theme's, settings', styles' and numbering's root elements a reading was made with,
/// held weakly.
pub(crate) type HeldRoots = [Option<Weak<Element>>; 4];

/// `formatting`'s root elements, held weakly.
pub(crate) fn held_roots(formatting: &Formatting) -> HeldRoots {
    formatting.roots().clone().map(|r| r.as_ref().map(Arc::downgrade))
}

/// Whether `roots` are `formatting`'s root elements, by identity.
pub(crate) fn same_roots(roots: &HeldRoots, formatting: &Formatting) -> bool {
    roots.iter().zip(formatting.roots()).all(|pair| match pair {
        (Some(kept), Some(now)) => std::ptr::eq(kept.as_ptr(), Arc::as_ptr(now)),
        (None, None) => true,
        _ => false,
    })
}

/// One table's reading, and the elements it was read from, held weakly.
#[derive(Debug, Clone)]
struct KeptTable {
    table: Weak<Element>,
    roots: HeldRoots,
    facts: Arc<TableFacts>,
}

impl KeptTable {
    /// Whether it was read from `table` with `formatting`'s parts.
    fn of(&self, table: &Arc<Element>, formatting: &Formatting) -> bool {
        std::ptr::eq(self.table.as_ptr(), Arc::as_ptr(table)) && same_roots(&self.roots, formatting)
    }

    /// Whether a version can still ask for it: its table element is held.
    fn alive(&self) -> bool {
        self.table.strong_count() > 0
    }
}

impl Weigh for KeptTable {
    fn weight(&self) -> usize {
        let facts = &self.facts;
        let paragraphs: usize = facts.paragraphs.keys().map(|k| k.len() + 96).sum();
        let cells: usize = facts.cells.iter().map(|(k, p)| k.len() + 48 + 64 * p.len()).sum();
        std::mem::size_of::<KeptTable>() + paragraphs + cells + 256 * facts.tables.len()
    }
}

/// What the tables' readings hold at most where no budget is given, bytes.
pub const TABLE_MEMO_BYTES: usize = 32 << 20;

/// Each table's reading kept across a document's versions, by the table element's address
/// (a weak reference keeps the address its own while it is kept), within a budget: the
/// least recently used let go first, and those of tables no version holds swept.
pub(crate) struct TableMemo {
    /// The readings, and how many were held after the last sweep.
    memo: Mutex<(Memo<usize, KeptTable, AddressHash>, usize)>,
}

impl std::fmt::Debug for TableMemo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let memo = self.memo.lock().unwrap_or_else(PoisonError::into_inner);
        f.debug_struct("TableMemo").field("stats", &memo.0.stats()).finish()
    }
}

impl Default for TableMemo {
    fn default() -> TableMemo {
        TableMemo::new(Arc::new(Limit::new(TABLE_MEMO_BYTES)))
    }
}

impl TableMemo {
    /// An empty memo counted on `budget`.
    pub(crate) fn new(budget: Arc<dyn CacheBudget>) -> TableMemo {
        TableMemo {
            memo: Mutex::new((Memo::with_hasher(budget), 0)),
        }
    }

    /// What it holds.
    pub(crate) fn stats(&self) -> MemoStats {
        self.memo.lock().unwrap_or_else(PoisonError::into_inner).0.stats()
    }

    /// The reading of `table` with `formatting`: as kept, else read and kept.
    fn read(&self, formatting: &Formatting, table: &Arc<Element>) -> Arc<TableFacts> {
        let key = Arc::as_ptr(table) as usize;
        let found = {
            let mut memo = self.memo.lock().unwrap_or_else(PoisonError::into_inner);
            memo.0
                .get(&key)
                .filter(|k| k.of(table, formatting))
                .map(|k| Arc::clone(&k.facts))
        };
        if let Some(found) = found {
            return found;
        }
        // Read outside the lock: another reader of another table is not held up.
        let facts = Arc::new(TableFacts::read(formatting, table));
        let kept = KeptTable {
            table: Arc::downgrade(table),
            roots: held_roots(formatting),
            facts: Arc::clone(&facts),
        };
        let mut memo = self.memo.lock().unwrap_or_else(PoisonError::into_inner);
        let (kept_now, held) = &mut *memo;
        kept_now.insert(key, kept);
        // Readings of tables no version holds: swept once the memo has doubled since.
        if kept_now.stats().entries > 2 * *held + 64 {
            kept_now.sweep(KeptTable::alive);
            *held = kept_now.stats().entries;
        }
        facts
    }
}

/// The cells of one part's paragraphs as the layout model reads them: the outermost table
/// holding a paragraph read once ([`TableFacts`]), each paragraph's cell found by its place.
pub struct PartCells<'a> {
    formatting: &'a Formatting,
    root: &'a Arc<Element>,
    tables: HashMap<Vec<u32>, Arc<TableFacts>>,
    /// The document's tables' readings kept across its versions, where the part is a
    /// story part of one.
    kept: Option<&'a TableMemo>,
    /// The rows or cells of each table or row asked about, found once.
    levels: Levels,
}

impl<'a> PartCells<'a> {
    /// The cells of the part whose root is `root`, read with `formatting`.
    #[must_use]
    pub fn new(formatting: &'a Formatting, root: &'a Arc<Element>) -> PartCells<'a> {
        PartCells {
            formatting,
            root,
            tables: HashMap::new(),
            kept: None,
            levels: Levels::default(),
        }
    }

    /// The cells of `story`'s paragraphs, read with `formatting`: each table's reading
    /// kept across the document's versions.
    #[must_use]
    pub fn of(formatting: &'a Formatting, story: &'a super::StoryPart) -> PartCells<'a> {
        PartCells {
            kept: Some(story.kept.tables()),
            ..PartCells::new(formatting, story.root())
        }
    }

    /// The cell of the paragraph at `place` (child indexes from the root), in the outermost
    /// table holding it; `None` outside tables.
    pub fn cell(&mut self, place: &[u32]) -> Option<Cell> {
        let depth = (1..place.len()).find(|d| {
            place
                .get(..*d)
                .and_then(|prefix| super::at_path(self.root, prefix))
                .is_some_and(|e| e.name().is(Some(W), "tbl"))
        })?;
        let (table_at, below) = place.split_at(depth);
        let table = super::at_path(self.root, table_at)?;
        let (formatting, kept) = (self.formatting, self.kept);
        let facts = self.tables.entry(table_at.to_vec()).or_insert_with(|| match kept {
            Some(kept) => kept.read(formatting, table),
            None => Arc::new(TableFacts::read(formatting, table)),
        });
        let path = path_from(table, String::from(ROOT), below, &mut self.levels)?;
        facts.cell(&path).cloned()
    }
}

/// The places of each parent's rows or cells (wrappers looked into), by the parent's
/// address and the level: a table's or row's found once for all the paragraphs in it.
#[derive(Default)]
struct Levels(HashMap<(usize, &'static str), HashMap<Vec<u32>, usize>>);

impl Levels {
    /// The place among `parent`'s rows or cells (`level`) of the one at `path` below it.
    fn index(&mut self, parent: &Element, level: &'static str, path: &[u32]) -> Option<usize> {
        let found = self
            .0
            .entry((std::ptr::from_ref(parent) as usize, level))
            .or_insert_with(|| {
                let mut found = Vec::new();
                level_paths(parent, level, &mut Vec::new(), &mut found);
                found.into_iter().enumerate().map(|(k, p)| (p, k)).collect()
            });
        found.get(path).copied()
    }
}

/// The layout model's path of the element at `below` under the `w:tbl` `table`: each step
/// `w:<name>[k]`, `k` counting siblings of that name from 1 -- rows among a table's rows and
/// cells among a row's cells wherever wrappers (content controls, custom XML) hold them.
#[must_use]
pub fn model_path(table: &Arc<Element>, below: &[u32]) -> Option<String> {
    path_from(table, String::from(ROOT), below, &mut Levels::default())
}

/// The layout model's path of the element at child indexes `place` in the story part whose
/// root is `root`, as the model names a story's blocks and runs: the main part's from its
/// body (`w:body/w:p[3]/w:r[2]`), any other part's from its root (`w:hdr/w:p[1]/w:r[1]`);
/// rows and cells counted as [`model_path`] counts them.
#[must_use]
pub fn story_model_path(root: &Arc<Element>, place: &[u32]) -> Option<String> {
    let (first, rest) = place.split_first()?;
    match root.children().get(*first as usize)?.as_element() {
        Some(body) if body.name().is(Some(W), "body") => {
            path_from(body, String::from("w:body"), rest, &mut Levels::default())
        }
        _ => path_from(
            root,
            format!("w:{}", root.name().local()),
            place,
            &mut Levels::default(),
        ),
    }
}

/// The layout model's path of the element at `below` under `start`, whose own path is
/// `prefix` (the part's index names a container child's step; the steps below it are
/// counted here, rows and cells as [`model_path`] counts them).
pub(crate) fn path_below(start: &Arc<Element>, prefix: String, below: &[u32]) -> Option<String> {
    path_from(start, prefix, below, &mut Levels::default())
}

/// The places below `parent` (child indexes) of its rows or cells (`level`), counted through
/// row- and cell-level wrappers as [`model_path`] counts them, in order.
pub(crate) fn level_places(parent: &Element, level: &str) -> Vec<Vec<u32>> {
    let mut out = Vec::new();
    level_paths(parent, level, &mut Vec::new(), &mut out);
    out
}

/// The steps from `start` to the element at `below`, after `prefix`.
fn path_from(start: &Arc<Element>, prefix: String, below: &[u32], levels: &mut Levels) -> Option<String> {
    let mut out = prefix;
    let mut node = Arc::clone(start);
    let mut steps = below.iter().peekable();
    while let Some(k) = steps.next() {
        let parent_name = node.name().local().to_owned();
        let child = node.children().get(*k as usize)?.as_element()?.clone();
        let name = child.name().local().to_owned();
        let level = match parent_name.as_str() {
            "tbl" => Some("tr"),
            "tr" => Some("tc"),
            _ => None,
        };
        if let Some(level) = level.filter(|l| name != *l) {
            // A wrapper at row or cell level: the row or cell inside it, counted among all.
            let mut path = vec![*k];
            let mut inner = Arc::clone(&child);
            while inner.name().local() != level {
                let step = *steps.next()?;
                path.push(step);
                inner = inner.children().get(step as usize)?.as_element()?.clone();
            }
            let index = levels.index(&node, level, &path)?;
            out.push_str(&format!("/w:{level}[{}]", index + 1));
            node = inner;
            continue;
        }
        if let Some(level) = level {
            let index = levels.index(&node, level, &[*k])?;
            out.push_str(&format!("/w:{level}[{}]", index + 1));
        } else {
            let count = node
                .children()
                .iter()
                .take(*k as usize)
                .filter(|n| matches!(n, Node::Element(e) if e.name().local() == name))
                .count();
            out.push_str(&format!("/w:{name}[{}]", count + 1));
        }
        node = child;
    }
    Some(out)
}

fn level_paths(parent: &Element, level: &str, path: &mut Vec<u32>, out: &mut Vec<Vec<u32>>) {
    for (k, node) in parent.children().iter().enumerate() {
        let Node::Element(e) = node else { continue };
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        let local = e.name().local();
        if e.name().namespace() == Some(W) {
            if local == level {
                out.push(path.clone());
            } else if local == "sdt" {
                if let Some(j) = e
                    .children()
                    .iter()
                    .position(|n| matches!(n, Node::Element(c) if c.name().is(Some(W), "sdtContent")))
                {
                    path.push(u32::try_from(j).unwrap_or(u32::MAX));
                    if let Some(content) = e.children().get(j).and_then(Node::as_element) {
                        level_paths(content, level, path, out);
                    }
                    path.pop();
                }
            } else if local == "customXml" {
                level_paths(e, level, path, out);
            }
        }
        path.pop();
    }
}
