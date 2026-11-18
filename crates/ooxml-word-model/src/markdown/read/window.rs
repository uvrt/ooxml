//! Owns: reading part of a story (design section 7: a read of a span reads only what it
//! returns): the story's top-level blocks grouped as the view joins them -- cheap, no
//! record made -- the blocks before the part wanted only counted for their list items
//! (the one thing a block's reading takes from the blocks before it, as docx-agent's
//! `Reader.block` counts them), the part wanted read into records, the blocks after it not
//! read at all.
//!
//! What a window read finds is kept with the version ([`Kept`], `docx::Derived`): the
//! grouping, and the list counters before every [`MARK_EVERY`]th group counted, so the
//! next window read of the version counts from the mark before it. A new version takes the
//! version before's marks for as long as its groups are the very same blocks (and its
//! styles and numbering the same): an edit costs the counting from it on, not from the
//! start (design section 7).
//!
//! Must not: render, or read records of blocks outside the window.

use super::{Item2, Placed, Reader, Record};
use crate::docx::StoryPart;
use crate::ids::W;
use crate::labels::Counters;
use ooxml_xml::{Element, Node};
use std::ops::RangeInclusive;
use std::sync::Arc;

/// Groups counted between two marks.
pub const MARK_EVERY: usize = 64;

/// The list counters of a part (by whether in a text box) before a group.
type Mark = (usize, Vec<(bool, Counters)>);

/// What window reads keep of a story in a view, for a version.
#[derive(Debug)]
pub struct Kept {
    groups: Groups,
    /// The paragraphs the view joins into the first cell of a table after them, by table.
    into: Vec<(Box<[u32]>, Vec<Placed>)>,
    /// The styles and numbering the counting read (their trees' roots).
    definitions: Vec<Arc<Element>>,
    /// The counters before groups along the story, in order.
    marks: Vec<Mark>,
}

/// A story's top-level blocks as the view groups them, and what each holds.
#[derive(Debug, Clone)]
pub struct Groups {
    part: String,
    groups: Vec<Item2>,
    /// Per group: the places of the elements it holds (a table's own place covers what is
    /// inside it), or the control id of a marker.
    holds: Vec<Holds>,
}

#[derive(Debug, Clone)]
enum Holds {
    Paths(Vec<Box<[u32]>>),
    Marker(String, bool),
}

impl Groups {
    /// The group holding the element at `path` (or the control `control`'s start marker).
    #[must_use]
    pub fn position(&self, path: Option<&[u32]>, control: Option<&str>) -> Option<usize> {
        self.holds.iter().position(|h| match h {
            Holds::Paths(paths) => path.is_some_and(|path| paths.iter().any(|p| path.starts_with(p))),
            Holds::Marker(id, begin) => *begin && control == Some(id.as_str()),
        })
    }

    /// The group closing the one at `k`: a control's end for its start.
    #[must_use]
    pub fn through(&self, k: usize) -> usize {
        let Some(Holds::Marker(id, true)) = self.holds.get(k) else {
            return k;
        };
        self.holds
            .iter()
            .enumerate()
            .skip(k + 1)
            .find(|(_, h)| matches!(h, Holds::Marker(e, false) if e == id))
            .map_or(k, |(j, _)| j)
    }

    /// How many groups there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }
}

impl Reader<'_> {
    /// The groups of a story part whose blocks are a container's (the body, a header or
    /// footer); `None` for a notes or comments part, read whole. Kept with the version
    /// (the paragraphs joined into tables put back each time).
    pub fn groups_of(&mut self, part: &StoryPart) -> Option<Groups> {
        let key = (part.part.clone(), self.view_name);
        if let Some((kept, true)) = self.docx.memo().window(&key) {
            for (table, joined) in &kept.into {
                self.into.insert((part.part.clone(), table.clone()), joined.clone());
            }
            return Some(kept.groups.clone());
        }
        let groups = self.group_part(part)?;
        let into: Vec<(Box<[u32]>, Vec<Placed>)> = self
            .into
            .iter()
            .filter(|((p, _), _)| *p == part.part)
            .map(|((_, t), joined)| (t.clone(), joined.clone()))
            .collect();
        let definitions = definitions(self.docx);
        let marks = match self.docx.memo().window(&key) {
            Some((earlier, false)) if same_roots(&earlier.definitions, &definitions) => {
                let same = shared_prefix((&earlier.groups, &earlier.into), (&groups, &into));
                earlier.marks.iter().filter(|(k, _)| *k <= same).cloned().collect()
            }
            _ => Vec::new(),
        };
        let kept = Kept {
            groups: groups.clone(),
            into,
            definitions,
            marks,
        };
        self.docx.memo().keep_window(key, Arc::new(kept));
        Some(groups)
    }

    fn group_part(&mut self, part: &StoryPart) -> Option<Groups> {
        let root = part.root();
        let notes = ["footnotes", "endnotes", "comments"]
            .iter()
            .any(|n| root.name().is(Some(W), n));
        if notes {
            return None;
        }
        let container = if root.name().is(Some(W), "document") {
            root.children().iter().enumerate().find_map(|(k, n)| match n {
                Node::Element(e) if e.name().is(Some(W), "body") => Some(Placed {
                    element: Arc::clone(e),
                    path: vec![super::index(k)].into_boxed_slice(),
                }),
                _ => None,
            })?
        } else {
            Placed {
                element: Arc::clone(root),
                path: Box::default(),
            }
        };
        let items = self.items(&part.part, &container);
        let groups = self.groups(&part.part, items);
        let holds = groups
            .iter()
            .map(|g| match g {
                Item2::P(elements) => Holds::Paths(elements.iter().map(|p| p.path.clone()).collect()),
                Item2::Tbl(t) => {
                    let mut paths = vec![t.path.clone()];
                    if let Some(joined) = self.into.get(&(part.part.clone(), t.path.clone())) {
                        paths.extend(joined.iter().map(|p| p.path.clone()));
                    }
                    Holds::Paths(paths)
                }
                Item2::Marker(m) => Holds::Marker(m.id.clone(), m.begin),
            })
            .collect();
        Some(Groups {
            part: part.part.clone(),
            groups,
            holds,
        })
    }

    /// The records of the groups `wanted`, the groups before them counted for their list
    /// items only and those after not read.
    pub fn read_window(&mut self, groups: Groups, wanted: RangeInclusive<usize>) -> Vec<Record> {
        let (start, end) = (*wanted.start(), *wanted.end());
        let key = (groups.part.clone(), self.view_name);
        let kept = self.docx.memo().window(&key).filter(|(_, own)| *own).map(|(k, _)| k);
        let mut marks: Vec<Mark> = kept.as_ref().map(|k| k.marks.clone()).unwrap_or_default();
        let from = marks.iter().rev().find(|(k, _)| *k <= start).cloned();
        if let Some((_, counters)) = &from {
            self.counters.retain(|(p, _), _| *p != groups.part);
            for (in_box, c) in counters {
                self.counters.insert((groups.part.clone(), *in_box), c.clone());
            }
        }
        let first = from.map_or(0, |(k, _)| k);
        let known = marks.last().map_or(0, |(k, _)| *k);
        let Groups { part, groups, .. } = groups;
        let mut out = Vec::new();
        for (k, group) in groups.into_iter().enumerate().skip(first) {
            if k > end {
                break;
            }
            let counting = k < start;
            if counting && k > known && k % MARK_EVERY == 0 {
                marks.push((k, self.counters_of(&part)));
            }
            self.counting = counting;
            match group {
                Item2::P(elements) => {
                    let record = self.paragraph(&part, elements, None);
                    if !counting {
                        out.push(Record::Paragraph(Box::new(record)));
                    }
                }
                Item2::Tbl(t) => {
                    let records = self.table_with(&part, &t, None);
                    if !counting {
                        out.extend(records);
                    }
                }
                Item2::Marker(m) => {
                    if !counting {
                        out.push(Record::Marker(m));
                    }
                }
            }
        }
        self.counting = false;
        if let Some(kept) = kept {
            if marks.len() > kept.marks.len() {
                let more = Kept {
                    groups: kept.groups.clone(),
                    into: kept.into.clone(),
                    definitions: kept.definitions.clone(),
                    marks,
                };
                self.docx.memo().keep_window(key, Arc::new(more));
            }
        }
        out
    }

    /// The list counters of `part` now, by whether in a text box.
    fn counters_of(&self, part: &str) -> Vec<(bool, Counters)> {
        self.counters
            .iter()
            .filter(|((p, _), _)| p == part)
            .map(|((_, in_box), c)| (*in_box, c.clone()))
            .collect()
    }
}

/// The roots of the styles and numbering trees `docx` reads (what list counting depends on
/// besides the blocks).
fn definitions(docx: &crate::docx::Docx) -> Vec<Arc<Element>> {
    let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";
    ["styles", "numbering"]
        .iter()
        .filter_map(|kind| {
            let part = crate::parts::related(&docx.package, &docx.main, &format!("{rel}{kind}")).ok()??;
            docx.package.tree(&part).ok()?.map(|t| Arc::clone(t.root()))
        })
        .collect()
}

fn same_roots(a: &[Arc<Element>], b: &[Arc<Element>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| Arc::ptr_eq(x, y))
}

/// The paragraphs joined into tables, by table.
type Joined = [(Box<[u32]>, Vec<Placed>)];

/// How many groups from the start `a` and `b` share: the very same blocks, grouped alike
/// (a table with the very same paragraphs joined into it).
fn shared_prefix(a: (&Groups, &Joined), b: (&Groups, &Joined)) -> usize {
    let joined = |into: &Joined, table: &Placed| -> Option<Vec<Placed>> {
        into.iter().find(|(t, _)| *t == table.path).map(|(_, j)| j.clone())
    };
    a.0.groups
        .iter()
        .zip(&b.0.groups)
        .take_while(|(x, y)| match (x, y) {
            (Item2::Tbl(t), Item2::Tbl(u)) => {
                same_group(x, y) && same_list(&joined(a.1, t).unwrap_or_default(), &joined(b.1, u).unwrap_or_default())
            }
            _ => same_group(x, y),
        })
        .count()
}

fn same_placed(x: &Placed, y: &Placed) -> bool {
    Arc::ptr_eq(&x.element, &y.element) && x.path == y.path
}

fn same_list(x: &[Placed], y: &[Placed]) -> bool {
    x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_placed(p, q))
}

fn same_group(a: &Item2, b: &Item2) -> bool {
    match (a, b) {
        (Item2::P(x), Item2::P(y)) => same_list(x, y),
        (Item2::Tbl(x), Item2::Tbl(y)) => same_placed(x, y),
        (Item2::Marker(x), Item2::Marker(y)) => x.id == y.id && x.begin == y.begin,
        _ => false,
    }
}
