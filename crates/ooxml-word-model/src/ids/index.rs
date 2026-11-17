//! Owns: what every paragraph, table and row of one story part is called -- docx-agent's
//! `PartIndex` (`docx_agent/edit/ids.py`): `p:<paraId>` (prefixed with the story outside
//! the body), `p@<story>/<n>` for a paragraph without a paraId, `p:<paraId>#<k>` for the
//! k-th repeat of one, `t:<first row's paraId>` or `t@<story>/<n>` for a table, `tr:` or
//! `<table>/r<k>` for a row.
//!
//! Elements are found by their place in the part (a path of child indexes from the root),
//! never by pointer: the tree shares identical subtrees, so two empty paragraphs may be one
//! element in memory.
//!
//! Must not: stamp, generate or remember ids (the ledger does), or resolve an address.

use crate::ids::{valid_long_hex, MC, W, W14};
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::Arc;

/// A place in a part: the child indexes from the root (every node counted, as cursors do).
pub type Path = Box<[u32]>;

/// How a paragraph's id stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdState {
    /// A valid paraId, the first of its value.
    Ok,
    /// No paraId: a positional id, volatile.
    Missing,
    /// A repeat of an earlier paraId (`#k`), volatile.
    Repeat,
    /// A paraId out of range (not `0 < id < 0x80000000`).
    Invalid,
}

impl IdState {
    /// Whether the id can change without the paragraph being edited.
    #[must_use]
    pub fn volatile(self) -> bool {
        matches!(self, IdState::Missing | IdState::Repeat)
    }
}

/// A paragraph's id.
#[derive(Debug, Clone)]
pub struct ParagraphId {
    /// The paragraph.
    pub element: Arc<Element>,
    /// Its place in the part.
    pub path: Path,
    /// Its id.
    pub id: String,
    /// The `w14:paraId` as written.
    pub raw: Option<String>,
    /// How the id stands.
    pub state: IdState,
    /// Its 0-based place among the story's paragraphs (live ones, document order).
    pub index: usize,
}

/// A table's id and its rows'.
#[derive(Debug, Clone)]
pub struct TableId {
    /// The table.
    pub element: Arc<Element>,
    /// Its place in the part.
    pub path: Path,
    /// Its id.
    pub id: String,
    /// Its own rows (not a nested table's), each with its place and id.
    pub rows: Vec<(Path, String)>,
}

/// One entry of the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// The paragraph at this place in [`PartIds::paragraphs`].
    Paragraph(usize),
    /// The table at this place in [`PartIds::tables`].
    Table(usize),
}

/// Every paragraph and table of one story part, by id and by place.
#[derive(Debug, Clone, Default)]
pub struct PartIds {
    /// The story's name (`body`, `header1`, `footnotes`...).
    pub story: String,
    /// The paragraphs in document order (`mc:Fallback` content left out).
    pub paragraphs: Vec<ParagraphId>,
    /// The tables in document order, nested ones included.
    pub tables: Vec<TableId>,
    by_id: HashMap<String, Entry>,
    by_path: HashMap<Path, Entry>,
}

/// A row as its table's ids read it: its raw paraId, how often that value came before, its
/// place.
type RowFound<'a> = (Option<&'a str>, usize, &'a Path);

/// A live `w:p`, `w:tr` or `w:tbl` as the walk meets it.
struct Found {
    element: Arc<Element>,
    path: Path,
    kind: Kind,
    /// The place of the nearest enclosing table, for a row.
    owner: Option<Path>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    P,
    Tr,
    Tbl,
}

impl PartIds {
    /// The index of the part whose root is `root`, for the story `story`.
    #[must_use]
    pub fn new(story: &str, root: &Element) -> PartIds {
        let found = live_blocks(root);
        let raws: Vec<Option<&str>> = found
            .iter()
            .map(|f| match f.kind {
                Kind::Tbl => None,
                Kind::P | Kind::Tr => f.element.attribute(Some(W14), "paraId"),
            })
            .collect();
        let repeats = repeat_counts(&raws);
        let mut out = PartIds {
            story: story.to_owned(),
            ..PartIds::default()
        };
        out.index_paragraphs(&found, &raws, &repeats);
        out.index_tables(&found, &raws, &repeats);
        out
    }

    /// The index of `new_root`, a later version of the part `self` indexes (whose root was
    /// `old_root`): equal to [`PartIds::new`]'s, with every subtree the new version shares
    /// with the old one (the same allocation at the same place) taken from this index
    /// rather than walked again -- an edit that path-copies one block re-reads that block.
    #[must_use]
    pub fn after(&self, old_root: &Element, new_root: &Element) -> PartIds {
        let old = self.found(old_root);
        let mut found = Vec::with_capacity(old.len());
        let mut tables: Vec<Path> = Vec::new();
        walk_after(old_root, new_root, &old, &mut Vec::new(), &mut tables, &mut found);
        let raws: Vec<Option<&str>> = found
            .iter()
            .map(|f| match f.kind {
                Kind::Tbl => None,
                Kind::P | Kind::Tr => f.element.attribute(Some(W14), "paraId"),
            })
            .collect();
        let repeats = repeat_counts(&raws);
        let mut out = PartIds {
            story: self.story.clone(),
            ..PartIds::default()
        };
        out.index_paragraphs(&found, &raws, &repeats);
        out.index_tables(&found, &raws, &repeats);
        out
    }

    /// The live blocks this index holds, in document order (as [`live_blocks`] found them).
    fn found(&self, root: &Element) -> Vec<Found> {
        let mut out: Vec<Found> = Vec::with_capacity(self.paragraphs.len() + self.tables.len());
        out.extend(self.paragraphs.iter().map(|p| Found {
            element: Arc::clone(&p.element),
            path: p.path.clone(),
            kind: Kind::P,
            owner: None,
        }));
        for table in &self.tables {
            out.push(Found {
                element: Arc::clone(&table.element),
                path: table.path.clone(),
                kind: Kind::Tbl,
                owner: None,
            });
            for (path, _) in &table.rows {
                if let Some(row) = element_at(root, path) {
                    out.push(Found {
                        element: row,
                        path: path.clone(),
                        kind: Kind::Tr,
                        owner: Some(table.path.clone()),
                    });
                }
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    fn index_paragraphs(&mut self, found: &[Found], raws: &[Option<&str>], repeats: &[usize]) {
        let prefix = self.prefix();
        for ((f, raw), repeat) in found.iter().zip(raws).zip(repeats) {
            if f.kind != Kind::P {
                continue;
            }
            let index = self.paragraphs.len();
            let (id, state) = match raw {
                None => (format!("p@{}/{index}", self.story), IdState::Missing),
                Some(raw) if *repeat > 0 => (format!("{prefix}p:{raw}#{repeat}"), IdState::Repeat),
                Some(raw) if valid_long_hex(raw) => (format!("{prefix}p:{raw}"), IdState::Ok),
                Some(raw) => (format!("{prefix}p:{raw}"), IdState::Invalid),
            };
            self.by_id.insert(id.clone(), Entry::Paragraph(index));
            self.by_path.insert(f.path.clone(), Entry::Paragraph(index));
            self.paragraphs.push(ParagraphId {
                element: Arc::clone(&f.element),
                path: f.path.clone(),
                id,
                raw: raw.map(str::to_owned),
                state,
                index,
            });
        }
    }

    fn index_tables(&mut self, found: &[Found], raws: &[Option<&str>], repeats: &[usize]) {
        let prefix = self.prefix();
        let tables: Vec<&Found> = found.iter().filter(|f| f.kind == Kind::Tbl).collect();
        // Each table's rows, in document order, gathered in one pass.
        let mut rows_of: HashMap<&Path, Vec<RowFound<'_>>> = HashMap::new();
        for ((f, raw), repeat) in found.iter().zip(raws).zip(repeats) {
            if let (Kind::Tr, Some(owner)) = (f.kind, f.owner.as_ref()) {
                rows_of.entry(owner).or_default().push((*raw, *repeat, &f.path));
            }
        }
        for (number, table) in tables.into_iter().enumerate() {
            let rows = rows_of.remove(&table.path).unwrap_or_default();
            let id = match rows.first().and_then(|(raw, repeat, _)| good(*raw, *repeat)) {
                Some(first) => format!("{prefix}t:{first}"),
                None => format!("t@{}/{number}", self.story),
            };
            let row_ids = rows
                .iter()
                .enumerate()
                .map(|(k, (raw, repeat, path))| {
                    let row = match good(*raw, *repeat) {
                        Some(raw) => format!("{prefix}tr:{raw}"),
                        None => format!("{id}/r{k}"),
                    };
                    ((*path).clone(), row)
                })
                .collect();
            let at = self.tables.len();
            self.by_id.insert(id.clone(), Entry::Table(at));
            self.by_path.insert(table.path.clone(), Entry::Table(at));
            self.tables.push(TableId {
                element: Arc::clone(&table.element),
                path: table.path.clone(),
                id,
                rows: row_ids,
            });
        }
    }

    fn prefix(&self) -> String {
        if self.story == "body" {
            String::new()
        } else {
            format!("{}/", self.story)
        }
    }

    /// The entry an id names, as written (no alias followed).
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Entry> {
        self.by_id.get(id).copied()
    }

    /// The entry of the paragraph or table at `path`.
    #[must_use]
    pub fn at(&self, path: &[u32]) -> Option<Entry> {
        self.by_path.get(path).copied()
    }

    /// The paragraph at `path`.
    #[must_use]
    pub fn paragraph_at(&self, path: &[u32]) -> Option<&ParagraphId> {
        match self.at(path)? {
            Entry::Paragraph(k) => self.paragraphs.get(k),
            Entry::Table(_) => None,
        }
    }

    /// The table at `path`.
    #[must_use]
    pub fn table_at(&self, path: &[u32]) -> Option<&TableId> {
        match self.at(path)? {
            Entry::Table(k) => self.tables.get(k),
            Entry::Paragraph(_) => None,
        }
    }

    /// The id of the entry `entry`.
    #[must_use]
    pub fn id_of(&self, entry: Entry) -> Option<&str> {
        match entry {
            Entry::Paragraph(k) => self.paragraphs.get(k).map(|p| p.id.as_str()),
            Entry::Table(k) => self.tables.get(k).map(|t| t.id.as_str()),
        }
    }
}

/// How many times each raw paraId was written before (case-insensitive), 0 for the first;
/// all 0 when no value repeats.
fn repeat_counts(raws: &[Option<&str>]) -> Vec<usize> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    raws.iter()
        .map(|raw| match raw {
            None => 0,
            Some(raw) => {
                let count = seen.entry(raw.to_uppercase()).or_insert(0);
                let before = *count;
                *count += 1;
                before
            }
        })
        .collect()
}

/// The live `w:p`, `w:tr` and `w:tbl` below `root` in document order, each with its path
/// and, for a row, its nearest table's path; `mc:Fallback` subtrees are skipped (a fallback
/// repeats its choice's content and is not what Word shows).
fn live_blocks(root: &Element) -> Vec<Found> {
    let mut out = Vec::new();
    let mut tables: Vec<Path> = Vec::new();
    walk(root, &mut Vec::new(), &mut tables, &mut out);
    out
}

fn walk(element: &Element, path: &mut Vec<u32>, tables: &mut Vec<Path>, out: &mut Vec<Found>) {
    for (k, node) in element.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        if child.name().is(Some(MC), "Fallback") {
            continue;
        }
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        let kind = if child.name().is(Some(W), "p") {
            Some(Kind::P)
        } else if child.name().is(Some(W), "tr") {
            Some(Kind::Tr)
        } else if child.name().is(Some(W), "tbl") {
            Some(Kind::Tbl)
        } else {
            None
        };
        if let Some(kind) = kind {
            let here: Path = path.clone().into_boxed_slice();
            out.push(Found {
                element: Arc::clone(child),
                path: here.clone(),
                kind,
                owner: if kind == Kind::Tr { tables.last().cloned() } else { None },
            });
            if kind == Kind::Tbl {
                tables.push(here);
            }
        }
        let table = kind == Some(Kind::Tbl);
        walk(child, path, tables, out);
        if table {
            tables.pop();
        }
        path.pop();
    }
}

/// A row's paraId where it names the row: valid and not a repeat.
fn good(raw: Option<&str>, repeat: usize) -> Option<&str> {
    raw.filter(|r| valid_long_hex(r) && repeat == 0)
}

/// The element at `path` below `root`.
fn element_at(root: &Element, path: &[u32]) -> Option<Arc<Element>> {
    let mut at: Option<&Arc<Element>> = None;
    for k in path {
        let parent: &Element = at.map_or(root, |a| a);
        at = Some(parent.children().get(*k as usize)?.as_element()?);
    }
    at.cloned()
}

/// [`walk`] of `new`, taking from `old_found` (the old version's blocks, sorted by path)
/// the blocks of each child that is the old version's own child at the same place.
fn walk_after(
    old: &Element,
    new: &Element,
    old_found: &[Found],
    path: &mut Vec<u32>,
    tables: &mut Vec<Path>,
    out: &mut Vec<Found>,
) {
    for (k, node) in new.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        if child.name().is(Some(MC), "Fallback") {
            continue;
        }
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        let before = old.children().get(k).and_then(Node::as_element);
        if before.is_some_and(|b| Arc::ptr_eq(b, child)) {
            // the same subtree at the same place: its blocks as they were
            let start = old_found.partition_point(|f| f.path.as_ref() < path.as_slice());
            for f in old_found.get(start..).unwrap_or_default() {
                if !f.path.starts_with(path) {
                    break;
                }
                out.push(Found {
                    element: Arc::clone(&f.element),
                    path: f.path.clone(),
                    kind: f.kind,
                    owner: f.owner.clone(),
                });
            }
            path.pop();
            continue;
        }
        let kind = if child.name().is(Some(W), "p") {
            Some(Kind::P)
        } else if child.name().is(Some(W), "tr") {
            Some(Kind::Tr)
        } else if child.name().is(Some(W), "tbl") {
            Some(Kind::Tbl)
        } else {
            None
        };
        if let Some(kind) = kind {
            let here: Path = path.clone().into_boxed_slice();
            out.push(Found {
                element: Arc::clone(child),
                path: here.clone(),
                kind,
                owner: if kind == Kind::Tr { tables.last().cloned() } else { None },
            });
            if kind == Kind::Tbl {
                tables.push(here);
            }
        }
        let table = kind == Some(Kind::Tbl);
        match before {
            Some(b) if b.name() == child.name() => walk_after(b, child, old_found, path, tables, out),
            _ => walk(child, path, tables, out),
        }
        if table {
            tables.pop();
        }
        path.pop();
    }
}
