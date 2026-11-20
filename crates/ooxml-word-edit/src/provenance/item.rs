//! Owns: the records' custom XML item as an edit changes it ([`Draft`]): found and read
//! through the sink (the main part's relationships and the item are the keys read), its
//! entries kept, rewritten, dropped (with the text after them, as lxml's `remove` takes
//! an element's tail) or added at the end, then written back -- or, when no entry is left,
//! released with its relationship and properties (docx-agent's `_remove_provenance_item`).
//!
//! Must not: decide which entries go (the follow-ups do).

use crate::doc::Key;
use crate::sink::EditSink;
use crate::Result;
use ooxml_opc::name::rels_for;
use ooxml_word_model::docx::provenance::{self as records, NS, REL_CUSTOM_XML};
use ooxml_word_model::docx::Docx;
use ooxml_xml::{Attribute, Element, Node, QName, Tree};
use std::sync::Arc;

/// What becomes of one entry.
pub(super) enum Fate {
    /// It stays as it is.
    Kept,
    /// It goes.
    Dropped,
    /// Its key and its record now.
    Written(String, String),
}

/// The item being changed: its part, its tree as read and its root's children now.
pub(super) struct Draft {
    name: String,
    tree: Tree,
    nodes: Vec<Node>,
    changed: bool,
}

impl Draft {
    /// The item of the sink's working version, read (`None`: the document has none).
    ///
    /// # Errors
    /// When the main part's relationships are not XML.
    pub(super) fn open(sink: &mut EditSink<'_, '_>, now: &Docx) -> Result<Option<Draft>> {
        sink.read(Key::Part(Arc::from(rels_for(&now.main))));
        let Some(name) = records::item(now)? else {
            return Ok(None);
        };
        sink.read(Key::Part(Arc::from(name.as_str())));
        let Some(tree) = sink.doc().package().tree(&name)? else {
            return Ok(None);
        };
        let nodes = tree.root().children().to_vec();
        Ok(Some(Draft {
            name,
            tree,
            nodes,
            changed: false,
        }))
    }

    /// Each entry's fate, in order.
    pub(super) fn settle(&mut self, mut fate: impl FnMut(&Element) -> Fate) {
        let mut out = Vec::with_capacity(self.nodes.len());
        let mut tail = false;
        for node in std::mem::take(&mut self.nodes) {
            match &node {
                Node::Element(entry) if is_entry(entry) => {
                    tail = false;
                    match fate(entry) {
                        Fate::Kept => out.push(node),
                        Fate::Dropped => (self.changed, tail) = (true, true),
                        Fate::Written(key, text) => {
                            out.push(Node::Element(Arc::new(written(entry, &key, &text))));
                            self.changed = true;
                        }
                    }
                }
                Node::Text(_) if tail => tail = false,
                _ => {
                    tail = false;
                    out.push(node);
                }
            }
        }
        self.nodes = out;
    }

    /// The entries' keys, in order.
    pub(super) fn keys(&self) -> Vec<String> {
        self.entries()
            .map(|e| e.attribute(None, "key").unwrap_or_default().to_owned())
            .collect()
    }

    /// The entries now.
    pub(super) fn entries(&self) -> impl Iterator<Item = &Arc<Element>> {
        self.nodes.iter().filter_map(|n| n.as_element().filter(|e| is_entry(e)))
    }

    /// The record `key`'s entry holds now: its text replaced where there is one, else a new
    /// entry of `kind` at the end (docx-agent's `store_provenance` with a record).
    pub(super) fn store(&mut self, kind: &str, key: &str, text: &str) {
        let mut found = false;
        self.settle(|entry| {
            if !found && entry.attribute(None, "key") == Some(key) {
                found = true;
                Fate::Written(key.to_owned(), text.to_owned())
            } else {
                Fate::Kept
            }
        });
        if !found {
            let attribute = |local: &str, value: &str| Attribute {
                name: Arc::new(QName::new(None, local, None)),
                value: Arc::from(value),
            };
            let prefix = self.tree.root().name().prefix().map(str::to_owned);
            let entry = Element::new(
                Arc::new(QName::new(prefix.as_deref(), "source", Some(NS))),
                Vec::new(),
                vec![attribute("kind", kind), attribute("key", key)],
                vec![Node::Text(Arc::from(text))],
            );
            self.nodes.push(Node::Element(Arc::new(entry)));
            self.changed = true;
        }
    }

    /// The item written back through `sink`, or released with its relationship and
    /// properties when no entry is left.
    ///
    /// # Errors
    /// When the main part's relationships are not XML.
    pub(super) fn finish(self, sink: &mut EditSink<'_, '_>, now: &Docx) -> Result<()> {
        if self.entries().next().is_none() {
            let ids: Vec<String> = now
                .package
                .relationships(&now.main)?
                .iter()
                .filter(|r| {
                    r.rel_type == REL_CUSTOM_XML && !r.external && r.target_part.as_deref() == Some(self.name.as_str())
                })
                .map(|r| r.id.clone())
                .collect();
            return sink.release(&now.main, &ids);
        }
        if self.changed {
            let root = self.tree.root();
            let root = Element::from_head(Arc::clone(root.head()), ooxml_xml::Children::from_vec(self.nodes));
            sink.put(&self.name, self.tree.with_root(Arc::new(root)))?;
        }
        Ok(())
    }
}

fn is_entry(element: &Element) -> bool {
    element.name().is(Some(NS), "source")
}

/// `entry` keyed `key`, its text `text` (its attributes in their order, its other children
/// kept).
fn written(entry: &Element, key: &str, text: &str) -> Element {
    let attributes = entry
        .attributes()
        .iter()
        .map(|a| {
            if a.name.namespace().is_none() && a.name.local() == "key" {
                Attribute {
                    name: a.name.clone(),
                    value: Arc::from(key),
                }
            } else {
                a.clone()
            }
        })
        .collect();
    let mut children = vec![Node::Text(Arc::from(text))];
    children.extend(
        entry
            .children()
            .iter()
            .skip_while(|n| !matches!(n, Node::Element(_)))
            .cloned(),
    );
    Element::new(entry.name().clone(), entry.namespaces().to_vec(), attributes, children)
}
