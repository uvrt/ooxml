//! Owns: a paragraph made to look like a sample, as docx-agent's `edit/like.py` does it
//! (round 42): the sample's own `w:pPr` children that make its look ([`ParagraphLook`]:
//! style, list, indents, keeps, page break, widow control; each without its change
//! records), which of them a paragraph differs in, and each one it differs in taken whole
//! ([`take`]: the paragraph's own removed, the sample's copied in schema order); the look
//! of an item a level deeper in the sample's list, as Word demotes one
//! ([`ParagraphLook::deeper`]).
//!
//! Must not: write but on the work it is given, or take anything else of the sample (run
//! formatting, spacing, alignment and tabs stay the paragraph's own).

use crate::marks::insert_in_order;
use crate::names::W;
use crate::validate::Orders;
use crate::work::{Id, Work};
use ooxml_word_model::docx::numbering::{membership, Lists, Membership};
use ooxml_word_model::docx::Docx;
use ooxml_xml::{Attribute, Element, Name, Node, QName};
use std::sync::Arc;

/// The list levels a definition has (`w:ilvl` 0 to 8).
const LEVELS: i64 = 9;

/// What a paragraph takes from its sample: the part's name, as results say it, and the
/// `w:pPr` child it is.
pub const PARTS: [(&str, &str); 7] = [
    ("style", "pStyle"),
    ("list", "numPr"),
    ("indent", "ind"),
    ("keep_with_next", "keepNext"),
    ("keep_lines", "keepLines"),
    ("page_break_before", "pageBreakBefore"),
    ("widow_control", "widowControl"),
];

/// What the result says a paragraph does not take.
pub const NOT_TAKEN: &str = "run formatting, spacing, alignment and tabs stay each paragraph's own";

/// A sample paragraph's own properties that make its look (copies: a look outlives the
/// sample).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParagraphLook {
    /// The sample's id.
    pub sample: String,
    /// Per part of [`PARTS`], in order: its `w:pPr` child, or `None` where it has none.
    pub parts: Vec<Option<Arc<Element>>>,
}

impl ParagraphLook {
    /// The look of the paragraph `id`, whose element is `paragraph`.
    #[must_use]
    pub fn of(id: &str, paragraph: &Element) -> ParagraphLook {
        let properties = paragraph.find(Some(W), "pPr");
        let parts = PARTS
            .iter()
            .map(|(_, local)| properties.and_then(|p| p.find(Some(W), local)).map(clean))
            .collect();
        ParagraphLook {
            sample: id.to_owned(),
            parts,
        }
    }

    /// The parts this look gives a paragraph that has none of its own: those the sample has.
    #[must_use]
    pub fn stated(&self) -> Vec<&'static str> {
        PARTS
            .iter()
            .zip(&self.parts)
            .filter(|(_, part)| part.is_some())
            .map(|((name, _), _)| *name)
            .collect()
    }

    /// The list a paragraph of this look is in (its style's, or its own `w:numPr`'s), or
    /// none.
    #[must_use]
    pub fn membership(&self, docx: &Docx, lists: &Lists) -> Option<Membership> {
        // a stand-in paragraph: what the list is read from, its style and its own numbering
        let stated: Vec<Node> = ["style", "list"]
            .iter()
            .filter_map(|name| self.part(name))
            .map(|e| Node::Element(Arc::clone(e)))
            .collect();
        let properties = Element::new(word("pPr"), Vec::new(), Vec::new(), stated);
        let paragraph = Element::new(
            word("p"),
            Vec::new(),
            Vec::new(),
            vec![Node::Element(Arc::new(properties))],
        );
        membership(docx, lists, &paragraph)
    }

    /// The look of an item `levels` deeper in the sample's list, as Word demotes an item
    /// (Tab): the same style, list and page settings, the level `levels` lower and that
    /// level's own indent (no direct one). `None` where the sample is in no list, or the
    /// list has no such level.
    #[must_use]
    pub fn deeper(&self, docx: &Docx, lists: &Lists, levels: i64) -> Option<ParagraphLook> {
        let found = self.membership(docx, lists)?;
        let level = found.level + levels;
        if level >= LEVELS || lists.level(docx, found.num_id, level).is_none() {
            return None;
        }
        let value = |local: &str, value: i64| {
            let attribute = Attribute {
                name: word("val"),
                value: Arc::from(value.to_string()),
            };
            Node::Element(Arc::new(Element::new(
                word(local),
                Vec::new(),
                vec![attribute],
                Vec::new(),
            )))
        };
        let numbering = Element::new(
            word("numPr"),
            Vec::new(),
            Vec::new(),
            vec![value("ilvl", level), value("numId", found.num_id)],
        );
        let mut deeper = self.clone();
        for ((name, _), part) in PARTS.iter().zip(&mut deeper.parts) {
            match *name {
                "list" => *part = Some(Arc::new(numbering.clone())),
                "indent" => *part = None,
                _ => {}
            }
        }
        Some(deeper)
    }

    /// The part `name` of [`PARTS`], where the sample has it.
    fn part(&self, name: &str) -> Option<&Arc<Element>> {
        PARTS
            .iter()
            .zip(&self.parts)
            .find(|((n, _), _)| *n == name)
            .and_then(|(_, part)| part.as_ref())
    }
}

/// `w:<local>`, as docx-agent's `make` names it.
fn word(local: &str) -> Name {
    Arc::new(QName::new(Some("w"), local, Some(W)))
}

/// A copy of a property without its change records (`w:ins`, `w:del`,
/// `w:numberingChange`): the sample's history, not its look.
fn clean(node: &Arc<Element>) -> Arc<Element> {
    let mut work = Work::of(node, Vec::new());
    let root = work.root();
    let records: Vec<Id> = work
        .children(root)
        .iter()
        .copied()
        .filter(|c| work.is_any(*c, W, &["ins", "del", "numberingChange"]))
        .collect();
    if records.is_empty() {
        return Arc::clone(node);
    }
    for record in records {
        work.remove(record);
    }
    work.build().unwrap_or_else(|| Arc::clone(node))
}

/// docx-agent's `_key`: an element's name, its attributes as a set, and its element
/// children's keys in order.
fn same(a: &Element, b: &Element) -> bool {
    if a.name().namespace() != b.name().namespace() || a.name().local() != b.name().local() {
        return false;
    }
    let attrs = |e: &Element| {
        let mut out: Vec<(Option<String>, String, String)> = e
            .attributes()
            .iter()
            .map(|x| {
                (
                    x.name.namespace().map(str::to_owned),
                    x.name.local().to_owned(),
                    x.value.to_string(),
                )
            })
            .collect();
        out.sort();
        out
    };
    if attrs(a) != attrs(b) {
        return false;
    }
    let elements = |e: &Element| -> Vec<Arc<Element>> {
        e.children()
            .iter()
            .filter_map(|n| match n {
                Node::Element(c) => Some(Arc::clone(c)),
                _ => None,
            })
            .collect()
    };
    let (left, right) = (elements(a), elements(b));
    left.len() == right.len() && left.iter().zip(&right).all(|(x, y)| same(x, y))
}

/// The parts of [`PARTS`] (less `skip`) in which `paragraph` differs from `look`.
#[must_use]
pub fn differs(paragraph: &Element, look: &ParagraphLook, skip: &[&str]) -> Vec<&'static str> {
    let properties = paragraph.find(Some(W), "pPr");
    let mut out = Vec::new();
    for ((name, local), wanted) in PARTS.iter().zip(&look.parts) {
        if skip.contains(name) {
            continue;
        }
        let own = properties.and_then(|p| p.find(Some(W), local)).map(clean);
        let alike = match (&own, wanted) {
            (None, None) => true,
            (Some(a), Some(b)) => same(a, b),
            _ => false,
        };
        if !alike {
            out.push(*name);
        }
    }
    out
}

/// Give `paragraph` (in `work`) the parts of `look` named in `changed`: its own
/// removed, the sample's copied in schema order; its `w:pPr` made, and removed when left
/// empty.
pub(crate) fn take(work: &mut Work, paragraph: Id, look: &ParagraphLook, changed: &[&str], orders: &Orders) {
    if changed.is_empty() {
        return;
    }
    let properties = work.find(paragraph, W, "pPr").unwrap_or_else(|| {
        let made = work.new_element(work.element_name(paragraph, W, "pPr", "w"));
        work.insert(paragraph, 0, made);
        made
    });
    for ((name, local), wanted) in PARTS.iter().zip(&look.parts) {
        if !changed.contains(name) {
            continue;
        }
        if let Some(own) = work.find(properties, W, local) {
            work.remove(own);
        }
        if let Some(copy) = wanted.as_ref().and_then(|w| work.adopt(w)) {
            insert_in_order(work, properties, copy, orders);
        }
    }
    if work.children(properties).is_empty() {
        work.remove(properties);
    }
}
