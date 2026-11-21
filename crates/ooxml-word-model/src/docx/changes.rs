//! Owns: logical changes -- a document's revision records grouped as a reviewer reads them
//! (docx-agent's `revisions/changes.py`, `changes`): touching records one person made at
//! one time, a move's two ends, the marks between paragraphs, a table's structure edits
//! each on its own, a grid change with its table's change. Each group lists its records.
//!
//! Elements are found by their place in their part (an arena: every element of a part in
//! document order with its parent), as lxml's walk finds them: `mc:Fallback` content
//! included, as docx-agent's `iter` includes it.
//!
//! Must not: accept, reject, or word a change (only its records are grouped here).

use super::revisions::{records, Record, KINDS};
use super::{Docx, StoryPart};
use crate::ids::W;
use ooxml_xml::{Element, Node};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

/// Every element of a part in document order, with its parent.
struct Arena {
    root: Arc<Element>,
    nodes: Vec<Arc<Element>>,
    parents: Vec<Option<usize>>,
    by_path: HashMap<Box<[u32]>, usize>,
    paths: Vec<Box<[u32]>>,
}

impl Arena {
    fn new(root: &Arc<Element>) -> Arena {
        let mut arena = Arena::empty(root);
        let mut path = Vec::new();
        arena.walk(root, 0, &mut path);
        arena
    }

    /// The root alone.
    fn empty(root: &Arc<Element>) -> Arena {
        let mut arena = Arena {
            root: Arc::clone(root),
            nodes: vec![Arc::clone(root)],
            parents: vec![None],
            by_path: HashMap::new(),
            paths: vec![Box::default()],
        };
        arena.by_path.insert(Box::default(), 0);
        arena
    }

    /// `element` (a child of node `at`, at `path`) and everything below it.
    fn add(&mut self, element: &Arc<Element>, at: usize, path: &mut Vec<u32>) {
        let index = self.push(element, at, path);
        self.walk(element, index, path);
    }

    /// `element` alone (a child of node `at`, at `path`): its index.
    fn push(&mut self, element: &Arc<Element>, at: usize, path: &[u32]) -> usize {
        let index = self.nodes.len();
        self.nodes.push(Arc::clone(element));
        self.parents.push(Some(at));
        let boxed: Box<[u32]> = path.into();
        self.paths.push(boxed.clone());
        self.by_path.insert(boxed, index);
        index
    }

    fn walk(&mut self, element: &Arc<Element>, at: usize, path: &mut Vec<u32>) {
        for (k, node) in element.children().iter().enumerate() {
            let Node::Element(child) = node else { continue };
            path.push(u32::try_from(k).unwrap_or(u32::MAX));
            self.add(child, at, path);
            path.pop();
        }
    }

    fn element(&self, k: usize) -> &Element {
        self.nodes.get(k).unwrap_or(&self.root)
    }

    fn parent(&self, k: usize) -> Option<usize> {
        self.parents.get(k).copied().flatten()
    }

    fn is(&self, k: usize, local: &str) -> bool {
        self.nodes.get(k).is_some_and(|e| e.name().is(Some(W), local))
    }

    /// The nearest ancestor of `k` named `local`.
    fn nearest(&self, k: usize, local: &str) -> Option<usize> {
        let mut at = self.parent(k);
        while let Some(p) = at {
            if self.is(p, local) {
                return Some(p);
            }
            at = self.parent(p);
        }
        None
    }

    /// The element's children that are elements, by index.
    fn children(&self, k: usize) -> Vec<usize> {
        // Children follow their parent in document order: each is the next node whose
        // parent is `k`, until the subtree ends.
        let mut out = Vec::new();
        let mut j = k + 1;
        while j < self.nodes.len() {
            match self.parent(j) {
                Some(p) if p == k => out.push(j),
                _ => {
                    if !self.descends(j, k) {
                        break;
                    }
                }
            }
            j += 1;
        }
        out
    }

    fn descends(&self, j: usize, ancestor: usize) -> bool {
        let mut at = self.parent(j);
        while let Some(p) = at {
            if p == ancestor {
                return true;
            }
            at = self.parent(p);
        }
        false
    }

    /// `k` and every element below it, in document order.
    fn subtree(&self, k: usize) -> Vec<usize> {
        let mut out = vec![k];
        let mut j = k + 1;
        while j < self.nodes.len() && self.descends(j, k) {
            out.push(j);
            j += 1;
        }
        out
    }

    fn find(&self, k: usize, local: &str) -> Option<usize> {
        self.children(k).into_iter().find(|c| self.is(*c, local))
    }

    fn attr(&self, k: usize, local: &str) -> Option<&str> {
        self.nodes.get(k).and_then(|e| e.attribute(Some(W), local))
    }
}

/// Union-find over a part's records, smallest index the root.
struct Union(Vec<usize>);

impl Union {
    fn find(&mut self, mut k: usize) -> usize {
        while let Some(&p) = self.0.get(k) {
            if p == k {
                break;
            }
            let grand = self.0.get(p).copied().unwrap_or(p);
            if let Some(slot) = self.0.get_mut(k) {
                *slot = grand;
            }
            k = p;
        }
        k
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            if let Some(slot) = self.0.get_mut(ra.max(rb)) {
                *slot = ra.min(rb);
            }
        }
    }
}

/// `ins` or `del` for a revision container (a move's end too), else none.
fn container(e: &Element) -> Option<&'static str> {
    match e.name().local() {
        "ins" | "moveTo" if e.name().namespace() == Some(W) => Some("ins"),
        "del" | "moveFrom" if e.name().namespace() == Some(W) => Some("del"),
        _ => None,
    }
}

fn is_move(e: &Element) -> bool {
    e.name().is(Some(W), "moveFrom") || e.name().is(Some(W), "moveTo")
}

/// Every part's records grouped into logical changes, in story and document order: the ids
/// of each group's records.
#[must_use]
pub fn change_groups(docx: &Docx) -> Vec<Vec<String>> {
    grouped(docx, |_, grouping, group| {
        group
            .members
            .iter()
            .filter_map(|k| grouping.held.get(*k).map(|r| r.id.clone()))
            .collect()
    })
}

/// Every part's records grouped, each group made into a value by `make` with the part's
/// grouping, in story and document order.
fn grouped<T>(docx: &Docx, mut make: impl FnMut(&StoryPart, &Grouping<'_>, &Group) -> T) -> Vec<T> {
    let all = records(docx);
    let mut by_part: BTreeMap<usize, Vec<Record>> = BTreeMap::new();
    for record in all {
        let place = docx
            .parts
            .iter()
            .position(|p| p.part == record.part)
            .unwrap_or(usize::MAX);
        by_part.entry(place).or_default().push(record);
    }
    let mut out = Vec::new();
    for (place, held) in by_part {
        let Some(part) = docx.parts.get(place) else { continue };
        let mut grouping = Grouping::new(part.root(), &held);
        for group in grouping.groups() {
            out.push(make(part, &grouping, &group));
        }
    }
    out
}

/// One group of a part's records: their indexes in the part's record list, in document
/// order, and the table structure edit it is, where one is.
struct Group {
    members: Vec<usize>,
    label: Option<&'static str>,
}

/// The grouping of one part's records.
struct Grouping<'r> {
    arena: Arena,
    held: &'r [Record],
    /// Arena index -> record index.
    key: HashMap<usize, usize>,
    /// Record index -> arena index.
    at: Vec<usize>,
    union: Union,
    names: HashMap<usize, String>,
    labels: HashMap<usize, &'static str>,
    claimed: HashSet<usize>,
}

impl<'r> Grouping<'r> {
    fn new(root: &Arc<Element>, held: &'r [Record]) -> Grouping<'r> {
        Grouping::over(Arena::new(root), held)
    }

    /// The grouping of `held`, the records `arena` holds.
    fn over(arena: Arena, held: &'r [Record]) -> Grouping<'r> {
        let at: Vec<usize> = held
            .iter()
            .map(|r| arena.by_path.get(&r.path).copied().unwrap_or(usize::MAX))
            .collect();
        let key = at.iter().enumerate().map(|(k, a)| (*a, k)).collect();
        let names = move_names(&arena);
        Grouping {
            union: Union((0..held.len()).collect()),
            arena,
            held,
            key,
            at,
            names,
            labels: HashMap::new(),
            claimed: HashSet::new(),
        }
    }

    fn who(&self, a: usize) -> (Option<&str>, Option<&str>) {
        (self.arena.attr(a, "author"), self.arena.attr(a, "date"))
    }

    fn kind(&self, a: usize) -> String {
        revision_kind(&self.arena, a)
    }

    /// One change, when one person made both at once -- a move only with its own records,
    /// a paragraph mark only with text of its own sign.
    fn join(&mut self, a: usize, b: usize, same_sign: bool) {
        let (Some(&ka), Some(&kb)) = (self.key.get(&a), self.key.get(&b)) else {
            return;
        };
        if self.who(a) != self.who(b) {
            return;
        }
        let (ea, eb) = (self.arena.element(a), self.arena.element(b));
        if same_sign {
            if container(ea) != container(eb) {
                return;
            }
        } else if is_move(ea) != is_move(eb) || self.names.get(&a) != self.names.get(&b) {
            return;
        }
        self.union.union(ka, kb);
    }

    fn groups(&mut self) -> Vec<Group> {
        self.moves_and_nesting();
        let tables: Vec<usize> = (0..self.arena.nodes.len())
            .filter(|k| self.arena.is(*k, "tbl"))
            .collect();
        for table in tables {
            self.table_joins(table);
        }
        self.paragraph_joins();
        let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for k in 0..self.held.len() {
            let root = self.union.find(k);
            groups.entry(root).or_default().push(k);
        }
        let mut out: Vec<Vec<usize>> = groups.into_values().collect();
        out.sort_by_key(|m| m.first().copied().unwrap_or(0));
        out.into_iter()
            .map(|members| {
                let label = members
                    .iter()
                    .find_map(|k| self.at.get(*k).and_then(|a| self.labels.get(a)).copied());
                Group { members, label }
            })
            .collect()
    }

    fn moves_and_nesting(&mut self) {
        let mut first_of_name: HashMap<String, usize> = HashMap::new();
        let at = self.at.clone();
        for (k, a) in at.iter().enumerate() {
            if let Some(name) = self.names.get(a).filter(|n| !n.is_empty()).cloned() {
                match first_of_name.get(&name) {
                    Some(first) => self.union.union(*first, k),
                    None => {
                        first_of_name.insert(name, k);
                    }
                }
            }
        }
        for a in at {
            let mut parent = self.arena.parent(a);
            while let Some(p) = parent {
                if self.key.contains_key(&p) {
                    self.join(p, a, false);
                    break;
                }
                parent = self.arena.parent(p);
            }
        }
    }
}

/// A revision record's kind, a mark's or a row's own named so (docx-agent's `revision_kind`).
fn revision_kind(arena: &Arena, a: usize) -> String {
    let element = arena.element(a);
    let local = element.name().local();
    let base = KINDS.iter().find(|(l, _)| *l == local).map_or("", |(_, k)| *k);
    if !["ins", "del", "moveFrom", "moveTo"].contains(&local) {
        return base.to_owned();
    }
    match arena.parent(a) {
        Some(p) if arena.is(p, "rPr") => {
            if arena.parent(p).is_some_and(|g| arena.is(g, "pPr")) {
                format!("paragraph-mark-{base}")
            } else {
                format!("run-mark-{base}")
            }
        }
        Some(p) if arena.is(p, "trPr") => format!("row-{base}"),
        _ => base.to_owned(),
    }
}

/// Every move record (by arena index) -> the name of the move range it is in.
fn move_names(arena: &Arena) -> HashMap<usize, String> {
    let mut out = HashMap::new();
    let mut open: [Vec<(String, String)>; 2] = [Vec::new(), Vec::new()];
    for k in 0..arena.nodes.len() {
        let e = arena.element(k);
        if e.name().namespace() != Some(W) {
            continue;
        }
        let local = e.name().local();
        let side = usize::from(local.starts_with("moveTo"));
        match local {
            "moveFromRangeStart" | "moveToRangeStart" => {
                let id = arena.attr(k, "id").unwrap_or("").to_owned();
                let name = arena.attr(k, "name").unwrap_or("").to_owned();
                if let Some(list) = open.get_mut(side) {
                    list.push((id, name));
                }
            }
            "moveFromRangeEnd" | "moveToRangeEnd" => {
                let wanted = arena.attr(k, "id");
                if let Some(list) = open.get_mut(side) {
                    list.retain(|(id, _)| Some(id.as_str()) != wanted);
                }
            }
            "moveFrom" | "moveTo" => {
                if let Some((_, name)) = open.get(side).and_then(|l| l.last()) {
                    out.insert(k, name.clone());
                } else if is_mark(arena, k) {
                    let paragraph = arena
                        .parent(k)
                        .and_then(|p| arena.parent(p))
                        .and_then(|p| arena.parent(p));
                    let start = if side == 0 {
                        "moveFromRangeStart"
                    } else {
                        "moveToRangeStart"
                    };
                    if let Some(found) =
                        paragraph.and_then(|p| arena.subtree(p).into_iter().find(|j| arena.is(*j, start)))
                    {
                        out.insert(k, arena.attr(found, "name").unwrap_or("").to_owned());
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Whether `k` is a paragraph mark's record (`w:pPr/w:rPr/*`).
fn is_mark(arena: &Arena, k: usize) -> bool {
    arena
        .parent(k)
        .filter(|p| arena.is(*p, "rPr"))
        .and_then(|p| arena.parent(p))
        .is_some_and(|g| arena.is(g, "pPr"))
}

mod describe;
mod joins;
mod window;

pub use describe::{changes, LogicalChange, CHANGE_KINDS};
pub use window::{authored_changes_holding, changes_holding};
