//! Owns: stand-ins, the values an edit drew while it was prepared ([`crate::Change::Draw`])
//! and what they were drawn as where the change was applied ([`Drawn`], kept by the version
//! the transaction made): new paragraph and row ids drawn from a seed in commit order, so
//! writers that each make new blocks from one version do not conflict (design section 5:
//! ids drawn where the change lands, as revision ids are numbered). Also which attributes
//! hold such ids ([`id_attribute`]), what a splice's inserted nodes need drawn into them
//! ([`scan`]) and a block key named by a stand-in ([`block_hex`]).
//!
//! On the version the edit is prepared on a stand-in is its value (the sink takes the
//! values from that very draw), so everything the edit reads or returns while it prepares
//! sees real ids; they differ only when the commit is replayed on a newer head.
//!
//! Must not: write anything; a stand-in found where no draw can reach it is pinned by the
//! sink (its sequence read), never left to stand for itself on a newer head.

use crate::doc::{Draw, DrawFrom};
use crate::names::{W14, W15};
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::Arc;

/// `w16cid` (comment durable ids; its `paraId` names a comment's last paragraph).
const W16CID: &str = "http://schemas.microsoft.com/office/word/2016/wordml/cid";

/// The values a version's transaction drew for its stand-ins, by stand-in (upper-case hex),
/// each with the seed it was drawn from. Cloning shares.
#[derive(Debug, Clone, Default)]
pub struct Drawn(Arc<HashMap<Arc<str>, ValueAndSeed>>);

/// A stand-in's value drawn and the seed it was drawn from.
type ValueAndSeed = (Arc<str>, Arc<str>);

impl Drawn {
    /// Nothing drawn.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// What `stand_in` was drawn as.
    #[must_use]
    pub fn get(&self, stand_in: &str) -> Option<&str> {
        self.0.get(stand_in).map(|(v, _)| &**v)
    }

    /// The stand-in `value` is (any case, surrounding space ignored) and the seed it was
    /// drawn from, if it is one.
    pub(crate) fn stand_in(&self, value: &str) -> Option<(Arc<str>, &Arc<str>)> {
        let value = value.trim();
        if value.len() != 8 {
            return None;
        }
        let (key, (_, seed)) = self.0.get_key_value(value.to_uppercase().as_str())?;
        Some((Arc::clone(key), seed))
    }

    /// Every stand-in, with its seed.
    pub(crate) fn stand_ins(&self) -> impl Iterator<Item = (&Arc<str>, &Arc<str>)> {
        self.0.iter().map(|(s, (_, seed))| (s, seed))
    }

    /// Record that `stand_in`, from `seed`, was drawn as `value`.
    pub(crate) fn insert(&mut self, stand_in: &Arc<str>, seed: &Arc<str>, value: String) {
        Arc::make_mut(&mut self.0).insert(Arc::clone(stand_in), (Arc::from(value), Arc::clone(seed)));
    }

    /// The stand-ins drawn as another value (none when the commit was not replayed on a
    /// newer head, or drew the same there).
    pub fn moved(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .filter(|(s, (v, _))| **s != *v)
            .map(|(s, (v, _))| (&**s, &**v))
    }

    /// `id` as drawn: a block id (`p:…`, `t:…`, a story's `…/p:…`) or a bare value whose
    /// hex is a stand-in, named by its value; any other id as it is.
    #[must_use]
    pub fn resolve(&self, id: &str) -> String {
        let (head, hex) = block_hex(id);
        match self.0.get(hex.to_uppercase().as_str()) {
            Some((value, _)) => format!("{head}{value}"),
            None => id.to_owned(),
        }
    }
}

/// A block id split before its hex (`p:1A2B3C4D` as `p:` and `1A2B3C4D`); a bare value
/// as an empty head and itself.
#[must_use]
pub fn block_hex(id: &str) -> (&str, &str) {
    match id.rfind(':') {
        Some(k) => id.split_at(k + 1),
        None => ("", id),
    }
}

/// Whether an attribute holds a paragraph or row id a draw resolves: `w14:paraId`,
/// `w14:textId`, and the comment parts' references to a comment's paragraph
/// (`w15:paraId`, `w15:paraIdParent`, `w16cid:paraId`).
#[must_use]
pub fn id_attribute(name: &ooxml_xml::Name) -> bool {
    name.is(Some(W14), "paraId")
        || name.is(Some(W14), "textId")
        || name.is(Some(W15), "paraId")
        || name.is(Some(W15), "paraIdParent")
        || name.is(Some(W16CID), "paraId")
}

/// What a splice's inserted `nodes` hold of `stand_ins`: the draws that write each where
/// it is in an id attribute, and the stand-ins found anywhere else (in another attribute or
/// in text), which no draw reaches.
pub(crate) fn scan(nodes: &[Node], stand_ins: &Drawn) -> (Vec<Draw>, Vec<Arc<str>>) {
    let mut draws = Vec::new();
    let mut elsewhere = Vec::new();
    if stand_ins.is_empty() {
        return (draws, elsewhere);
    }
    let found = |value: &str| -> Option<Arc<str>> { stand_ins.stand_in(value).map(|(s, _)| s) };
    for (k, node) in nodes.iter().enumerate() {
        let node_index = u32::try_from(k).unwrap_or(u32::MAX);
        let Node::Element(element) = node else {
            text_stand_ins(node, stand_ins, &mut elsewhere);
            continue;
        };
        let mut stack: Vec<(&Element, Vec<u32>)> = vec![(element, Vec::new())];
        while let Some((e, path)) = stack.pop() {
            for attribute in e.attributes().iter() {
                let Some(stand_in) = found(&attribute.value) else {
                    continue;
                };
                if id_attribute(&attribute.name) {
                    draws.push(Draw {
                        from: DrawFrom::Drawn(stand_in),
                        node: node_index,
                        at: path.clone(),
                        attribute: Arc::clone(&attribute.name),
                    });
                } else {
                    elsewhere.push(stand_in);
                }
            }
            for (j, child) in e.children().iter().enumerate() {
                match child {
                    Node::Element(c) => {
                        let mut below = path.clone();
                        below.push(u32::try_from(j).unwrap_or(u32::MAX));
                        stack.push((c, below));
                    }
                    other => text_stand_ins(other, stand_ins, &mut elsewhere),
                }
            }
        }
    }
    (draws, elsewhere)
}

/// The stand-ins a text node holds: each whole run of eight hex digits that is one.
fn text_stand_ins(node: &Node, stand_ins: &Drawn, into: &mut Vec<Arc<str>>) {
    let Some(text) = node.as_text() else { return };
    let runs = text
        .split(|c: char| !c.is_ascii_hexdigit())
        .filter(|run| run.len() == 8);
    into.extend(runs.filter_map(|run| stand_ins.stand_in(run).map(|(s, _)| s)));
}

/// Whether a splice's inserted `nodes` hold one of the transaction's stand-ins (`drawn`'s)
/// in an id attribute no draw of `draws` writes: a stand-in the sink let through, which a
/// commit replayed on a newer head would leave standing for itself (a duplicate id). Checked
/// in debug builds as each splice is applied.
pub(crate) fn uncovered(nodes: &[Node], draws: &[Draw], drawn: &Drawn) -> bool {
    if drawn.is_empty() {
        return false;
    }
    for (k, node) in nodes.iter().enumerate() {
        let Node::Element(element) = node else { continue };
        let node_index = u32::try_from(k).unwrap_or(u32::MAX);
        let mut stack: Vec<(&Element, Vec<u32>)> = vec![(element, Vec::new())];
        while let Some((e, path)) = stack.pop() {
            for attribute in e.attributes().iter() {
                let is_stand_in = id_attribute(&attribute.name)
                    && drawn.get(attribute.value.trim().to_uppercase().as_str()).is_some();
                let covered = || {
                    draws.iter().any(|d| {
                        d.node == node_index && d.at == path && d.attribute.as_ref() == attribute.name.as_ref()
                    })
                };
                if is_stand_in && !covered() {
                    return true;
                }
            }
            for (j, child) in e.children().iter().enumerate() {
                if let Node::Element(c) = child {
                    let mut below = path.clone();
                    below.push(u32::try_from(j).unwrap_or(u32::MAX));
                    stack.push((c, below));
                }
            }
        }
    }
    false
}
