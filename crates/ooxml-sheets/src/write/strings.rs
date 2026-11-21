//! Owns: the shared-string table as a writer keeps it (ooxml-edit's `Workbook._string_table`,
//! `add_shared_string`, and `finish`'s counts): each item's text, the first index of each
//! plain (run-less) text, a new text appended as `<si><t>` before the table's `extLst`, and
//! `count` / `uniqueCount` brought up to date where the table states them.
//!
//! Must not: write rich text, or drop an item (an index a cell holds stays valid).

use super::nodes::{element, insert_child, places, text_element, with_attr};
use crate::items::{child, item_text};
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::Arc;

/// The shared strings of a workbook being written.
#[derive(Debug)]
pub(crate) struct Strings {
    /// The part.
    pub(crate) part: String,
    /// The table's root as written so far.
    pub(crate) root: Arc<Element>,
    /// Each item's text, in table order.
    texts: Vec<String>,
    /// Whether each item is rich text (has runs).
    rich: Vec<bool>,
    /// The first index of each plain text.
    plain: HashMap<String, usize>,
    /// Whether an item was added.
    pub(crate) changed: bool,
}

impl Strings {
    /// The table whose root is `root`, indexed.
    pub(crate) fn new(part: String, root: Arc<Element>) -> Strings {
        let mut texts = Vec::new();
        let mut rich = Vec::new();
        let mut plain = HashMap::new();
        for (_, item) in places(&root, "si") {
            let text = item_text(item);
            let runs = child(item, "r").is_some();
            if !runs {
                plain.entry(text.clone()).or_insert(texts.len());
            }
            texts.push(text);
            rich.push(runs);
        }
        Strings {
            part,
            root,
            texts,
            rich,
            plain,
            changed: false,
        }
    }

    /// Item `index`'s text.
    pub(crate) fn text(&self, index: i64) -> Option<&str> {
        usize::try_from(index).ok().and_then(|i| self.texts.get(i)).map(String::as_str)
    }

    /// Whether item `index` is rich text.
    pub(crate) fn is_rich(&self, index: i64) -> bool {
        usize::try_from(index).ok().and_then(|i| self.rich.get(i)).copied().unwrap_or(false)
    }

    /// The index of `text`, appended as a new item when no plain item holds it.
    pub(crate) fn add(&mut self, text: &str) -> usize {
        if let Some(&index) = self.plain.get(text) {
            return index;
        }
        let t = Node::Element(Arc::new(text_element(&self.root, text)));
        let item = element(&self.root, "si", &[], vec![t]);
        // before the table's extLst (the first one, as the table was indexed), else last
        let at = places(&self.root, "extLst")
            .next()
            .map_or(self.root.children().len(), |(i, _)| i);
        self.root = Arc::new(insert_child(&self.root, at, Node::Element(Arc::new(item))));
        let index = self.texts.len();
        self.texts.push(text.to_owned());
        self.rich.push(false);
        self.plain.insert(text.to_owned(), index);
        self.changed = true;
        index
    }

    /// `count` (the cells using a shared string, `uses`) and `uniqueCount` (the items) where
    /// the table states them; whether either changed.
    pub(crate) fn recount(&mut self, uses: usize) -> bool {
        let mut root = (*self.root).clone();
        let mut changed = false;
        for (name, wanted) in [("count", uses), ("uniqueCount", self.texts.len())] {
            let wanted = wanted.to_string();
            if root.attribute(None, name).is_some_and(|v| v != wanted) {
                root = with_attr(&root, name, &wanted);
                changed = true;
            }
        }
        if changed {
            self.root = Arc::new(root);
            self.changed = true;
        }
        changed
    }
}
