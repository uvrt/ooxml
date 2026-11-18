//! Owns: a paragraph made to look like a sample, as docx-agent's `edit/like.py` does it
//! (round 42): the sample's own `w:pPr` children that make its look ([`ParagraphLook`]:
//! style, list, indents, keeps, page break, widow control; each without its change
//! records), which of them a paragraph differs in, and each one it differs in taken whole
//! ([`take`]: the paragraph's own removed, the sample's copied in schema order).
//!
//! Must not: write but on the work it is given, or take anything else of the sample (run
//! formatting, spacing, alignment and tabs stay the paragraph's own).

use crate::marks::insert_in_order;
use crate::names::W;
use crate::validate::Orders;
use crate::work::{Id, Work};
use ooxml_xml::{Element, Node};
use std::sync::Arc;

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
#[derive(Debug, Clone, PartialEq)]
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

/// Give `paragraph` (the work's root) the parts of `look` named in `changed`: its own
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
