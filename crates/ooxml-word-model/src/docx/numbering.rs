//! Owns: which list a paragraph is in, as docx-agent's `edit/numbering.py` reads it
//! (`membership`): its own `w:numPr`, else its style chain's; the level that applies (an
//! override's, else its definition's, the first definition sharing a `w:nsid`, through a
//! `w:numStyleLink`).
//!
//! Must not: count list items or draw labels (the layout model's labels do, as docx2svg
//! counts them), or change numbering.

use crate::docx::{find_path, Docx};
use crate::ids::W;
use crate::xmlread::py_int;
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::Arc;

/// A paragraph's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membership {
    /// The instance (`w:numId`).
    pub num_id: i64,
    /// The level, from 0.
    pub level: i64,
    /// Its definition (`w:abstractNumId`).
    pub abstract_id: Option<i64>,
    /// The level's `w:numFmt`.
    pub format: Option<String>,
    /// The level's `w:lvlText`.
    pub text: Option<String>,
    /// Whether the paragraph's style gives it (no `w:numId` of its own).
    pub from_style: bool,
}

/// The numbering part read once for a call.
#[derive(Debug, Clone, Default)]
pub struct Lists {
    nums: HashMap<i64, Arc<Element>>,
    abstracts: Vec<(i64, Arc<Element>)>,
}

fn int_attr(element: &Element, local: &str) -> Option<i64> {
    element.attribute(Some(W), local).and_then(py_int)
}

impl Lists {
    /// The numbering part of `docx`'s main part.
    #[must_use]
    pub fn read(docx: &Docx) -> Lists {
        let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";
        let Ok(Some(part)) = crate::parts::related(&docx.package, &docx.main, rel) else {
            return Lists::default();
        };
        let Ok(Some(tree)) = docx.package.tree(&part) else {
            return Lists::default();
        };
        let mut lists = Lists::default();
        for node in tree.root().child_elements() {
            if node.name().is(Some(W), "num") {
                if let Some(n) = int_attr(node, "numId") {
                    lists.nums.insert(n, Arc::clone(node));
                }
            } else if node.name().is(Some(W), "abstractNum") {
                if let Some(n) = int_attr(node, "abstractNumId") {
                    match lists.abstracts.iter_mut().find(|(k, _)| *k == n) {
                        Some(slot) => slot.1 = Arc::clone(node),
                        None => lists.abstracts.push((n, Arc::clone(node))),
                    }
                }
            }
        }
        lists
    }

    fn abstract_node(&self, id: Option<i64>) -> Option<&Arc<Element>> {
        let id = id?;
        self.abstracts.iter().find(|(k, _)| *k == id).map(|(_, e)| e)
    }

    /// The abstract definition `id` (the last of that id in the part).
    #[must_use]
    pub fn abstract_element(&self, id: i64) -> Option<&Arc<Element>> {
        self.abstract_node(Some(id))
    }

    /// The abstract definitions' ids, in the part's order.
    #[must_use]
    pub fn abstract_ids(&self) -> Vec<i64> {
        self.abstracts.iter().map(|(k, _)| *k).collect()
    }

    /// An instance's definition.
    #[must_use]
    pub fn abstract_of(&self, num_id: i64) -> Option<i64> {
        self.nums
            .get(&num_id)?
            .find(Some(W), "abstractNumId")
            .and_then(|r| int_attr(r, "val"))
    }

    /// The definition whose levels `abstract_id` uses: the first with its `w:nsid`.
    #[must_use]
    pub fn canonical(&self, abstract_id: Option<i64>) -> Option<i64> {
        let nsid = |e: &Element| {
            e.find(Some(W), "nsid")
                .map(|n| n.attribute(Some(W), "val").unwrap_or("").to_uppercase())
        };
        let value = self
            .abstract_node(abstract_id)
            .and_then(|e| nsid(e))
            .unwrap_or_default();
        if value.is_empty() {
            return abstract_id;
        }
        self.abstracts
            .iter()
            .find(|(_, e)| nsid(e).as_deref() == Some(value.as_str()))
            .map_or(abstract_id, |(k, _)| Some(*k))
    }

    /// The `w:lvl` that applies to `level` of instance `num_id`.
    #[must_use]
    pub fn level(&self, docx: &Docx, num_id: i64, level: i64) -> Option<Arc<Element>> {
        self.level_at(docx, num_id, level, 0)
    }

    fn level_at(&self, docx: &Docx, num_id: i64, level: i64, depth: usize) -> Option<Arc<Element>> {
        let num = self.nums.get(&num_id)?;
        for o in num.child_elements().filter(|e| e.name().is(Some(W), "lvlOverride")) {
            if int_attr(o, "ilvl") == Some(level) {
                if let Some(lvl) = o.find(Some(W), "lvl") {
                    return Some(Arc::clone(lvl));
                }
            }
        }
        let node = self.abstract_node(self.canonical(self.abstract_of(num_id)))?;
        if let Some(link) = node.find(Some(W), "numStyleLink") {
            let linked = style_num(docx, link.attribute(Some(W), "val"));
            if let Some(linked) = linked.filter(|l| *l != num_id) {
                if depth < 16 {
                    return self.level_at(docx, linked, level, depth + 1);
                }
                return None;
            }
        }
        node.child_elements()
            .find(|l| l.name().is(Some(W), "lvl") && int_attr(l, "ilvl") == Some(level))
            .cloned()
    }

    /// The definition whose levels instance `num_id` numbers with (docx-agent's
    /// `definition_of`): its own (the first with its `w:nsid`), or, for a definition that
    /// only names a list style (`w:numStyleLink`), the definition that style's list has.
    #[must_use]
    pub fn definition_of(&self, docx: &Docx, num_id: i64) -> Option<&Arc<Element>> {
        let mut seen = Vec::new();
        let mut num_id = num_id;
        while !seen.contains(&num_id) {
            seen.push(num_id);
            let node = self.abstract_node(self.canonical(self.abstract_of(num_id)))?;
            let linked = node
                .find(Some(W), "numStyleLink")
                .and_then(|link| style_num(docx, link.attribute(Some(W), "val")))
                .filter(|n| *n != 0);
            match linked {
                Some(next) => num_id = next,
                None => return Some(node),
            }
        }
        None
    }

    /// The instance `num_id` (the last of that number in the part).
    #[must_use]
    pub fn num_element(&self, num_id: i64) -> Option<&Arc<Element>> {
        self.nums.get(&num_id)
    }

    /// Whether the part has instance `num_id`.
    #[must_use]
    pub fn has(&self, num_id: i64) -> bool {
        self.nums.contains_key(&num_id)
    }
}

/// The `w:numId` the style `style_id` states itself.
fn style_num(docx: &Docx, style_id: Option<&str>) -> Option<i64> {
    let style_id = style_id?;
    let style = docx
        .styles()
        .iter()
        .find(|s| s.attribute(Some(W), "styleId") == Some(style_id))?;
    find_path(style, &["pPr", "numPr", "numId"]).and_then(|r| int_attr(r, "val"))
}

/// `(numId, ilvl)` a paragraph style gives through its `w:basedOn` chain.
#[must_use]
pub fn style_numbering(docx: &Docx, style_id: Option<&str>) -> (Option<i64>, Option<i64>) {
    let mut current = match style_id {
        Some(s) => Some(s.to_owned()),
        None => docx
            .styles()
            .iter()
            .find(|s| {
                s.attribute(Some(W), "type") == Some("paragraph")
                    && matches!(s.attribute(Some(W), "default"), Some("1" | "true"))
            })
            .and_then(|s| s.attribute(Some(W), "styleId").map(str::to_owned)),
    };
    let mut seen = Vec::new();
    let (mut num_id, mut level) = (None, None);
    while let Some(id) = current.take() {
        if seen.contains(&id) {
            break;
        }
        let Some(node) = docx.style(&id) else { break };
        seen.push(id);
        if let Some(numbering) = find_path(node, &["pPr", "numPr"]) {
            if num_id.is_none() {
                if let Some(r) = numbering.find(Some(W), "numId") {
                    num_id = int_attr(r, "val");
                }
            }
            if level.is_none() {
                if let Some(r) = numbering.find(Some(W), "ilvl") {
                    level = int_attr(r, "val");
                }
            }
        }
        current = node
            .find(Some(W), "basedOn")
            .and_then(|b| b.attribute(Some(W), "val"))
            .map(str::to_owned);
    }
    (num_id, level)
}

/// The list `paragraph` is in, or `None`.
#[must_use]
pub fn membership(docx: &Docx, lists: &Lists, paragraph: &Element) -> Option<Membership> {
    let properties = paragraph.find(Some(W), "pPr");
    let direct = properties.and_then(|p| p.find(Some(W), "numPr"));
    let style = properties
        .and_then(|p| p.find(Some(W), "pStyle"))
        .and_then(|s| s.attribute(Some(W), "val"));
    let (style_num, style_level) = style_numbering(docx, style);
    let (mut num_id, mut level, mut from_style) = (None, None, true);
    if let Some(direct) = direct {
        if let Some(r) = direct.find(Some(W), "numId") {
            num_id = int_attr(r, "val");
            from_style = false;
        }
        if let Some(r) = direct.find(Some(W), "ilvl") {
            level = int_attr(r, "val");
        }
    }
    let num_id = num_id.or(style_num).filter(|n| *n != 0)?;
    let level = level.or(style_level).unwrap_or(0);
    if !lists.has(num_id) {
        return None;
    }
    let lvl = lists.level(docx, num_id, level);
    let val = |local: &str| {
        lvl.as_ref()
            .and_then(|l| l.find(Some(W), local))
            .and_then(|n| n.attribute(Some(W), "val"))
            .map(str::to_owned)
    };
    Some(Membership {
        num_id,
        level,
        abstract_id: lists.abstract_of(num_id),
        format: val("numFmt"),
        text: val("lvlText"),
        from_style,
    })
}
