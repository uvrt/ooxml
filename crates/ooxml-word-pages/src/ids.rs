//! Owns: a version's body blocks by the tool's ids ([`BlockIds`]): each paragraph's, table's
//! and row's docx2svg path (`w:body/w:tbl[2]/w:tr[3]/w:tc[1]/w:p[1]`, the paths the layout's
//! lines and pages name blocks by) with its id as docx-agent names it in the version's
//! bytes (`ooxml_word_model::ids::PartIds`: `p:<paraId>`, `p@body/<n>`, `t:...`, rows'
//! ids) -- so a reader asks the layout by id and maps nothing itself (lane L4b).
//!
//! Paths count an element among its siblings of the same local name, from 1, as the Word
//! model writes them (`read::Steps`). The walk goes through every element but a
//! paragraph's content and `mc:Fallback` (as the ids do).
//!
//! Must not: stamp or change an id, follow an alias (the caller holds those), or read
//! anything but the main part's tree.

use ooxml_word_model::ids::{Entry, PartIds, MC, W};
use ooxml_word_model::value::Text;
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::Arc;

/// A version's body blocks' ids by path, and their paths by id.
#[derive(Debug, Clone, Default)]
pub struct BlockIds {
    by_path: HashMap<Text, Text>,
}

impl BlockIds {
    /// The ids of the body of the part whose root is `root`, indexed as `ids`.
    pub fn of(root: &Element, ids: &PartIds) -> BlockIds {
        let rows: HashMap<&[u32], &str> = ids
            .tables
            .iter()
            .flat_map(|t| t.rows.iter().map(|(path, id)| (&**path, id.as_str())))
            .collect();
        let mut walk = Walk {
            ids,
            rows,
            out: HashMap::new(),
            place: Vec::new(),
        };
        for (k, node) in root.children().iter().enumerate() {
            let Node::Element(child) = node else { continue };
            if child.name().is(Some(W), "body") {
                walk.place.push(u32::try_from(k).unwrap_or(u32::MAX));
                walk.container(child, "w:body");
                walk.place.pop();
                break;
            }
        }
        BlockIds { by_path: walk.out }
    }

    /// The id of the paragraph, table or row at `path`.
    pub fn get(&self, path: &str) -> Option<&Text> {
        self.by_path.get(path)
    }

    /// How many blocks it names.
    pub fn len(&self) -> usize {
        self.by_path.len()
    }

    /// Whether it names none.
    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    /// What it holds, bytes, roughly.
    pub fn weight(&self) -> usize {
        self.by_path
            .iter()
            .map(|(path, id)| path.len() + id.len() + 4 * std::mem::size_of::<usize>())
            .sum()
    }
}

/// One walk of a part's tree.
struct Walk<'a> {
    ids: &'a PartIds,
    rows: HashMap<&'a [u32], &'a str>,
    out: HashMap<Text, Text>,
    /// The child indexes from the root of the element walked now.
    place: Vec<u32>,
}

impl Walk<'_> {
    /// Name every block below `element`, whose path is `path`.
    fn container(&mut self, element: &Arc<Element>, path: &str) {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for (k, node) in element.children().iter().enumerate() {
            let Node::Element(child) = node else { continue };
            if child.name().is(Some(MC), "Fallback") {
                continue;
            }
            let local = child.name().local();
            let count = counts.entry(local).or_insert(0);
            *count += 1;
            let here = format!("{path}/w:{local}[{count}]");
            self.place.push(u32::try_from(k).unwrap_or(u32::MAX));
            let named = if child.name().is(Some(W), "tr") {
                self.rows.get(self.place.as_slice()).map(|id| (*id).to_owned())
            } else {
                self.ids
                    .at(self.place.as_slice())
                    .and_then(|entry: Entry| self.ids.id_of(entry))
                    .map(str::to_owned)
            };
            if let Some(id) = named {
                self.out.insert(Text::from(here.as_str()), Text::from(id.as_str()));
            }
            if !child.name().is(Some(W), "p") {
                self.container(child, &here);
            }
            self.place.pop();
        }
    }
}
