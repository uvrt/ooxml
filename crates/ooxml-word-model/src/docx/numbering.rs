//! Owns: which list a paragraph is in, as docx-agent's `edit/numbering.py` reads it: its
//! own `w:numPr`, else its style chain's, where the numbering part has that instance
//! ([`ListMembers`], the one rule the tools' `membership` and the layout's list labels both
//! ask); the level that applies (an override's, else its definition's, the first definition
//! sharing a `w:nsid`, through a `w:numStyleLink`).
//!
//! Must not: count list items or draw labels (the layout model's labels do, as docx2svg
//! counts them), or change numbering.

use crate::docx::{find_path, Docx};
use crate::ids::W;
use crate::xmlread::{attr_in, attr_int_in, child_in, children_in, py_int, Ns};
use ooxml_xml::Element;
use std::collections::{HashMap, HashSet};
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

    /// Whether `level` of instance `num_id` is one its definition lacks and Word fills: Word
    /// 16.106 gives a multi-level definition (not `w:multiLevelType="singleLevel"`) all nine
    /// levels when it opens the file, a missing one a decimal level with no text, so no label
    /// (measured: `style-document.docx`'s one-level bullet list, a paragraph set to level 1).
    #[must_use]
    pub fn fills(&self, docx: &Docx, num_id: i64, level: i64) -> bool {
        (0..9).contains(&level) && self.definition_of(docx, num_id).is_some() && !self.single_level(docx, num_id)
    }

    /// Whether instance `num_id` numbers with a single-level definition
    /// (`w:multiLevelType="singleLevel"`), whose paragraphs Word 16.106 sets to no level but
    /// the first (measured).
    #[must_use]
    pub fn single_level(&self, docx: &Docx, num_id: i64) -> bool {
        self.definition_of(docx, num_id)
            .and_then(|definition| definition.find(Some(W), "multiLevelType"))
            .is_some_and(|t| t.attribute(Some(W), "val") == Some("singleLevel"))
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
    docx.list_members().style(style_id)
}

/// One style's own `w:numPr` and the style it is based on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct StyleNumbering {
    num_id: Option<i64>,
    level: Option<i64>,
    based_on: Option<String>,
}

/// Which list a paragraph's own properties put it in, read once from the styles and the
/// numbering part (WordprocessingML, transitional or strict -- [`Ns::W`]; the tools refuse a
/// strict document, the layout draws one): each paragraph style's `w:numPr` and `w:basedOn` (the
/// last style of an id), the default paragraph style (the first of type `paragraph` marked
/// `w:default` `1` or `true`) and the instances the numbering part has. The one rule for a
/// paragraph's list: the tools ask it of a paragraph's element ([`membership`]), the layout
/// of a paragraph's properties in the view it reads (a joined paragraph's are its surviving
/// mark's).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListMembers {
    styles: HashMap<String, StyleNumbering>,
    default: Option<String>,
    nums: HashSet<i64>,
}

impl ListMembers {
    /// The rule read from `styles` (the styles part's `w:style` elements, in document order)
    /// and the numbering part's root, where there is one.
    #[must_use]
    pub fn read(styles: &[Arc<Element>], numbering: Option<&Element>) -> ListMembers {
        let w = Some(Ns::W);
        let mut out = ListMembers::default();
        for style in styles {
            let own = child_in(style, Ns::W, "pPr").and_then(|p| child_in(p, Ns::W, "numPr"));
            let read = |local: &str| {
                own.and_then(|n| child_in(n, Ns::W, local))
                    .and_then(|r| attr_int_in(r, w, "val"))
            };
            out.styles.insert(
                attr_in(style, w, "styleId").unwrap_or("").to_owned(),
                StyleNumbering {
                    num_id: read("numId"),
                    level: read("ilvl"),
                    based_on: child_in(style, Ns::W, "basedOn")
                        .and_then(|b| attr_in(b, w, "val"))
                        .map(str::to_owned),
                },
            );
            let default = attr_in(style, w, "type") == Some("paragraph")
                && matches!(attr_in(style, w, "default"), Some("1" | "true"));
            if default && out.default.is_none() {
                out.default = attr_in(style, w, "styleId").map(str::to_owned);
            }
        }
        if let Some(root) = numbering {
            out.nums = children_in(root, Ns::W, "num")
                .filter_map(|node| attr_int_in(node, w, "numId"))
                .collect();
        }
        out
    }

    /// `(numId, ilvl)` the paragraph style `style_id` (`None`: the default paragraph style)
    /// gives through its `w:basedOn` chain, each the nearest style's that states it.
    #[must_use]
    pub fn style(&self, style_id: Option<&str>) -> (Option<i64>, Option<i64>) {
        let mut current = style_id.map(str::to_owned).or_else(|| self.default.clone());
        let mut seen: Vec<String> = Vec::new();
        let (mut num_id, mut level) = (None, None);
        while let Some(id) = current.take() {
            if seen.contains(&id) {
                break;
            }
            let Some(style) = self.styles.get(&id) else { break };
            seen.push(id);
            num_id = num_id.or(style.num_id);
            level = level.or(style.level);
            current.clone_from(&style.based_on);
        }
        (num_id, level)
    }

    /// The list `(numId, level)` of a paragraph of style `style_id` whose own `w:numPr`
    /// states `num_id` and `level` (each where it does): its own, else its style chain's; none
    /// for `w:numId` 0 or an instance the numbering part does not have.
    #[must_use]
    pub fn of(&self, style_id: Option<&str>, num_id: Option<i64>, level: Option<i64>) -> Option<(i64, i64)> {
        let (style_num, style_level) = self.style(style_id);
        let num_id = num_id.or(style_num).filter(|n| *n != 0)?;
        let level = level.or(style_level).unwrap_or(0);
        self.nums.contains(&num_id).then_some((num_id, level))
    }
}

/// The list `paragraph` is in, or `None`.
#[must_use]
pub fn membership(docx: &Docx, lists: &Lists, paragraph: &Element) -> Option<Membership> {
    let properties = paragraph.find(Some(W), "pPr");
    let direct = properties.and_then(|p| p.find(Some(W), "numPr"));
    let style = properties
        .and_then(|p| p.find(Some(W), "pStyle"))
        .and_then(|s| s.attribute(Some(W), "val"));
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
    let (num_id, level) = docx.list_members().of(style, num_id, level)?;
    let lvl = lists.level(docx, num_id, level);
    let val = |local: &str| {
        lvl.as_ref()
            .and_then(|l| l.find(Some(W), local))
            .and_then(|n| n.attribute(Some(W), "val"))
            .map(str::to_owned)
    };
    // a level the definition lacks reads as Word fills it: decimal, no text
    let filled = lvl.is_none() && lists.fills(docx, num_id, level);
    Some(Membership {
        num_id,
        level,
        abstract_id: lists.abstract_of(num_id),
        format: val("numFmt").or_else(|| filled.then(|| "decimal".to_owned())),
        text: val("lvlText").or_else(|| filled.then(String::new)),
        from_style,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::ListMembers;
    use ooxml_xml::Element;
    use std::sync::Arc;

    const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

    fn root(xml: &str) -> Arc<Element> {
        Arc::clone(
            ooxml_xml::parse(xml.as_bytes(), &ooxml_base::XmlLimits::default())
                .unwrap()
                .root(),
        )
    }

    fn members(styles: &str, nums: &[i64]) -> ListMembers {
        let styles = root(&format!("<w:styles {W}>{styles}</w:styles>"));
        let styles: Vec<Arc<Element>> = styles.child_elements().cloned().collect();
        let nums: String = nums.iter().map(|n| format!(r#"<w:num w:numId="{n}"/>"#)).collect();
        let numbering = root(&format!("<w:numbering {W}>{nums}</w:numbering>"));
        ListMembers::read(&styles, Some(&numbering))
    }

    #[test]
    fn a_paragraph_is_in_its_own_list_else_its_style_chain_s_where_the_part_has_it() {
        let styles = r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"/>
<w:style w:type="paragraph" w:styleId="List"><w:pPr><w:numPr><w:numId w:val="3"/></w:numPr></w:pPr></w:style>
<w:style w:type="paragraph" w:styleId="Deeper"><w:basedOn w:val="List"/><w:pPr><w:numPr><w:ilvl w:val="2"/></w:numPr></w:pPr></w:style>
<w:style w:type="paragraph" w:styleId="Loop"><w:basedOn w:val="Loop"/></w:style>"#;
        let m = members(styles, &[3, 5]);
        // its own numPr; its own level over its style's
        assert_eq!(m.of(Some("Deeper"), Some(5), Some(1)), Some((5, 1)));
        // the chain gives the number and the nearest level
        assert_eq!(m.of(Some("Deeper"), None, None), Some((3, 2)));
        assert_eq!(m.of(Some("List"), None, None), Some((3, 0)));
        // no style named: the default paragraph style's (none here)
        assert_eq!(m.of(None, None, None), None);
        // numId 0 takes it out; an instance the part lacks is no list
        assert_eq!(m.of(Some("List"), Some(0), None), None);
        assert_eq!(m.of(None, Some(7), Some(0)), None);
        // a style based on itself ends the chain
        assert_eq!(m.of(Some("Loop"), None, None), None);
    }

    #[test]
    fn without_a_numbering_part_no_paragraph_is_in_a_list() {
        let m = ListMembers::read(&[], None);
        assert_eq!(m.of(None, Some(1), Some(0)), None);
    }
}
