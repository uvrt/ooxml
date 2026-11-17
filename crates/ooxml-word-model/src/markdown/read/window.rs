//! Owns: reading part of a story (design section 7: a read of a span reads only what it
//! returns): the story's top-level blocks grouped as the view joins them -- cheap, no
//! record made -- the blocks before the part wanted only counted for their list items
//! (the one thing a block's reading takes from the blocks before it, as docx-agent's
//! `Reader.block` counts them), the part wanted read into records, the blocks after it not
//! read at all.
//!
//! Must not: render, or read records of blocks outside the window.

use super::{Item2, Placed, Reader, Record};
use crate::docx::StoryPart;
use crate::ids::W;
use ooxml_xml::Node;
use std::ops::RangeInclusive;
use std::sync::Arc;

/// A story's top-level blocks as the view groups them, and what each holds.
pub struct Groups {
    part: String,
    groups: Vec<Item2>,
    /// Per group: the places of the elements it holds (a table's own place covers what is
    /// inside it), or the control id of a marker.
    holds: Vec<Holds>,
}

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
    /// footer); `None` for a notes or comments part, read whole.
    pub fn groups_of(&mut self, part: &StoryPart) -> Option<Groups> {
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
        let Groups { part, groups, .. } = groups;
        let (start, end) = (*wanted.start(), *wanted.end());
        let mut out = Vec::new();
        for (k, group) in groups.into_iter().enumerate() {
            if k > end {
                break;
            }
            let counting = k < start;
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
        out
    }
}
