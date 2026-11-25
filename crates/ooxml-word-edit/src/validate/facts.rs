//! Owns: [`ElementFacts`], what docx-agent's validation (`validate.py`, `check`) reads of one
//! element -- a top-level block of a story part, or a whole other part -- found in one walk
//! of it: the problems it holds alone (child order, paragraph ids, revisions, tables) and
//! the marks the whole-document rules count (paraIds, bookmarks, comment and note marks,
//! relationship ids, list and style references, drawing numbers, revision records, field
//! characters, annotation ids).
//!
//! Must not: read anything outside the element, or decide a whole-document rule (the
//! rules module folds the facts).

use crate::names::{MC, R, W, W14, WP, WP14};
use crate::validate::order::Orders;
use ooxml_word_model::ids::valid_long_hex;
use ooxml_xml::{Element, Node};

/// A revision record kind docx-agent's validation counts (`_RECORDS`).
const RECORDS: &[&str] = &[
    "ins",
    "del",
    "moveFrom",
    "moveTo",
    "rPrChange",
    "pPrChange",
    "sectPrChange",
    "tblPrChange",
    "trPrChange",
    "tcPrChange",
    "numberingChange",
    "cellIns",
    "cellDel",
    "cellMerge",
];
/// Elements whose `w:id` is an annotation id (docx-agent's `ANNOTATIONS`).
const ANNOTATIONS: &[&str] = &[
    "bookmarkStart",
    "bookmarkEnd",
    "commentRangeStart",
    "commentRangeEnd",
    "commentReference",
    "comment",
    "ins",
    "del",
    "moveFrom",
    "moveTo",
    "moveFromRangeStart",
    "moveFromRangeEnd",
    "moveToRangeStart",
    "moveToRangeEnd",
    "rPrChange",
    "pPrChange",
    "sectPrChange",
    "tblPrChange",
    "trPrChange",
    "tcPrChange",
    "tblGridChange",
    "numberingChange",
    "cellIns",
    "cellDel",
    "cellMerge",
    "customXmlInsRangeStart",
    "customXmlInsRangeEnd",
    "customXmlDelRangeStart",
    "customXmlDelRangeEnd",
    "customXmlMoveFromRangeStart",
    "customXmlMoveFromRangeEnd",
    "customXmlMoveToRangeStart",
    "customXmlMoveToRangeEnd",
    "permStart",
    "permEnd",
];

/// A problem's code and detail, its part given where the facts are folded.
pub(crate) type Local = (&'static str, String);

/// A `w:numPr`'s `w:numId` and `w:ilvl` values: `None` without the element,
/// `Some(None)` with it but without a `w:val`.
pub(crate) type NumRef = (Option<Option<String>>, Option<Option<String>>);

/// A move range mark: from or to, start or end, its id, its name (starts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MoveMark {
    pub(crate) from: bool,
    pub(crate) start: bool,
    pub(crate) id: String,
    pub(crate) name: String,
}

/// What validation reads of one element.
#[derive(Debug, Clone, Default)]
pub(crate) struct ElementFacts {
    /// Problems the element holds alone, in the order the check finds them per rule.
    pub(crate) order: Vec<Local>,
    pub(crate) paraid: Vec<Local>,
    pub(crate) revision: Vec<Local>,
    pub(crate) cells: Vec<Local>,
    pub(crate) grids: Vec<Local>,
    /// Live paragraphs' and rows' paraIds, upper-cased.
    pub(crate) para_ids: Vec<String>,
    /// Field characters outside deletions: the change in depth, and the lowest depth
    /// reached from 0.
    pub(crate) field_net: i64,
    pub(crate) field_low: i64,
    pub(crate) bookmark_starts: Vec<String>,
    pub(crate) bookmark_names: Vec<String>,
    pub(crate) bookmark_ends: Vec<String>,
    pub(crate) comment_starts: Vec<Option<String>>,
    pub(crate) comment_ends: Vec<Option<String>>,
    pub(crate) comment_refs: Vec<Option<String>>,
    pub(crate) footnote_refs: Vec<Option<String>>,
    pub(crate) endnote_refs: Vec<Option<String>>,
    /// `w:comment`, `w:footnote`, `w:endnote` ids (in their parts).
    pub(crate) comments: Vec<Option<String>>,
    pub(crate) footnotes: Vec<Option<String>>,
    pub(crate) endnotes: Vec<Option<String>>,
    /// Values of attributes in the relationships namespace.
    pub(crate) r_values: Vec<String>,
    /// Each `w:numPr`: its `w:numId` value (`Some(None)`: no `w:val`) and `w:ilvl` value.
    pub(crate) num_refs: Vec<NumRef>,
    /// `pStyle`, `rStyle`, `tblStyle` references.
    pub(crate) style_refs: Vec<(&'static str, Option<String>)>,
    /// Live drawing numbers (`wp:docPr/@id`, `""` when missing).
    pub(crate) docpr_ids: Vec<String>,
    /// Live revision records: kind, whether it has an author, its id.
    pub(crate) records: Vec<(&'static str, bool, Option<String>)>,
    /// Live move range marks.
    pub(crate) moves: Vec<MoveMark>,
    /// The largest annotation id written.
    pub(crate) annotation_max: Option<i64>,
    /// Live drawings (`wp:inline`, `wp:anchor`) without a valid anchorId and editId: what
    /// stamping gives ids to.
    pub(crate) unstamped_drawings: usize,
    /// The authors of revision records and comments, hidden ones too (docx-agent's
    /// `_authors_in_use`, which `_tidy_people` keeps listed).
    pub(crate) authors: Vec<String>,
}

impl ElementFacts {
    /// The facts of `element` and everything below it (`element` itself counted, as a
    /// block is in its part's walk).
    pub(crate) fn of(element: &Element, orders: &Orders) -> ElementFacts {
        let mut facts = ElementFacts::default();
        let mut ancestors: Vec<&Element> = Vec::new();
        visit(element, &mut ancestors, false, orders, &mut facts);
        facts
    }

    /// The facts of a part's root alone: its head, its own children's order, and `body`'s
    /// (the main part's), with the children `blocks_of` names left out; the order of the
    /// blocks' container (the body's children, a story root's) only with `container_order`
    /// (a tally keeps that order in place).
    pub(crate) fn of_rest(
        root: &Element,
        body: Option<&Element>,
        orders: &Orders,
        container_order: bool,
    ) -> ElementFacts {
        let mut facts = ElementFacts::default();
        head_facts(root, &[], false, &mut facts);
        if container_order || body.is_some() {
            order_of(root, orders, &mut facts.order);
        }
        let mut ancestors = vec![root];
        for child in root.child_elements() {
            if body.is_some_and(|b| std::ptr::eq(b, &**child)) {
                head_facts(child, &ancestors, false, &mut facts);
                if container_order {
                    order_of(child, orders, &mut facts.order);
                }
            } else if body.is_none() {
                // a story part's root children are its blocks
            } else {
                visit(child, &mut ancestors, false, orders, &mut facts);
            }
        }
        ancestors.clear();
        facts
    }
}

/// `w:id`'s value.
fn w_id(element: &Element) -> Option<String> {
    element.attribute(Some(W), "id").map(str::to_owned)
}

fn is_w(element: &Element, local: &str) -> bool {
    element.name().is(Some(W), local)
}

/// A revision container that holds runs (not a paragraph mark's or row's record).
fn owns_runs(parent: Option<&Element>) -> bool {
    parent.is_none_or(|p| !(is_w(p, "rPr") || is_w(p, "trPr")))
}

/// The facts of an element's own head (attributes, its kind's marks).
fn head_facts(element: &Element, ancestors: &[&Element], hidden: bool, facts: &mut ElementFacts) {
    for attribute in element.attributes() {
        if attribute.name.namespace() == Some(R) {
            facts.r_values.push(attribute.value.to_string());
        }
    }
    let name = element.name();
    if name.namespace() == Some(W) {
        word_marks(element, name.local(), ancestors, hidden, facts);
    } else if name.is(Some(WP), "docPr") && !hidden {
        facts
            .docpr_ids
            .push(element.attribute(None, "id").unwrap_or("").to_owned());
    } else if (name.is(Some(WP), "inline") || name.is(Some(WP), "anchor"))
        && !hidden
        && !(element.attribute(Some(WP14), "anchorId").is_some_and(valid_long_hex)
            && element.attribute(Some(WP14), "editId").is_some_and(valid_long_hex))
    {
        facts.unstamped_drawings += 1;
    }
}

fn word_marks(element: &Element, local: &str, ancestors: &[&Element], hidden: bool, facts: &mut ElementFacts) {
    if (RECORDS.contains(&local) || local == "tblGridChange" || local == "comment") && !local.is_empty() {
        if let Some(author) = element.attribute(Some(W), "author") {
            facts.authors.push(author.to_owned());
        }
    }
    if ANNOTATIONS.contains(&local) {
        if let Some(value) = element.attribute(Some(W), "id").and_then(python_int) {
            facts.annotation_max = Some(facts.annotation_max.map_or(value, |m| m.max(value)));
        }
    }
    match local {
        "p" | "tr" if !hidden => paraid_marks(element, facts),
        "bookmarkStart" => {
            facts.bookmark_starts.push(w_id(element).unwrap_or_default());
            facts
                .bookmark_names
                .push(element.attribute(Some(W), "name").unwrap_or("").to_owned());
        }
        "bookmarkEnd" => facts.bookmark_ends.push(w_id(element).unwrap_or_default()),
        "commentRangeStart" => facts.comment_starts.push(w_id(element)),
        "commentRangeEnd" => facts.comment_ends.push(w_id(element)),
        "commentReference" => facts.comment_refs.push(w_id(element)),
        "footnoteReference" => facts.footnote_refs.push(w_id(element)),
        "endnoteReference" => facts.endnote_refs.push(w_id(element)),
        "comment" => facts.comments.push(w_id(element)),
        "footnote" => facts.footnotes.push(w_id(element)),
        "endnote" => facts.endnotes.push(w_id(element)),
        "tbl" if !hidden => {
            for (code, detail) in crate::validate::grid::grid_problems(element) {
                facts.grids.push((code, detail));
            }
        }
        "numPr" => {
            let value = |e: &Element| e.attribute(Some(W), "val").map(str::to_owned);
            let num = element.find(Some(W), "numId").map(|e| value(e));
            let level = element.find(Some(W), "ilvl").map(|e| value(e));
            facts.num_refs.push((num, level));
        }
        "pStyle" | "rStyle" | "tblStyle" => {
            let tag = match local {
                "pStyle" => "pStyle",
                "rStyle" => "rStyle",
                _ => "tblStyle",
            };
            facts
                .style_refs
                .push((tag, element.attribute(Some(W), "val").map(str::to_owned)));
        }
        _ => revision_marks(element, local, ancestors, facts),
    }
    if !hidden {
        if RECORDS.contains(&local) {
            let kind = RECORDS.iter().find(|r| **r == local).copied().unwrap_or("ins");
            facts
                .records
                .push((kind, element.attribute(Some(W), "author").is_some(), w_id(element)));
        } else if let Some((from, start)) = move_mark(local) {
            facts.moves.push(MoveMark {
                from,
                start,
                id: w_id(element).unwrap_or_default(),
                name: element.attribute(Some(W), "name").unwrap_or("").to_owned(),
            });
        }
    }
}

/// The marks of text in or outside deletions, revisions holding links, field characters
/// and cells.
fn revision_marks(element: &Element, local: &str, ancestors: &[&Element], facts: &mut ElementFacts) {
    let owning = |k: usize| owns_runs(k.checked_sub(1).and_then(|p| ancestors.get(p)).copied());
    match local {
        "t" if ancestors.iter().enumerate().any(|(k, a)| is_w(a, "del") && owning(k)) => {
            let text: String = element.text_content().chars().take(30).collect();
            facts.revision.push(("text-in-deletion", text));
        }
        "delText" if !ancestors.iter().any(|a| is_w(a, "del")) => {
            let text: String = element.text_content().chars().take(30).collect();
            facts.revision.push(("deleted-text-outside-deletion", text));
        }
        "hyperlink" | "fldSimple"
            if ancestors
                .iter()
                .enumerate()
                .any(|(k, a)| ["ins", "del", "moveFrom", "moveTo"].iter().any(|t| is_w(a, t)) && owning(k)) =>
        {
            facts.revision.push(("hyperlink-in-revision", local.to_owned()));
        }
        "fldChar" if !ancestors.iter().any(|a| is_w(a, "del") || is_w(a, "moveFrom")) => {
            match element.attribute(Some(W), "fldCharType") {
                Some("begin") => facts.field_net += 1,
                Some("end") => {
                    facts.field_net -= 1;
                    facts.field_low = facts.field_low.min(facts.field_net);
                }
                _ => {}
            }
        }
        "tc" => {
            let last_block = element
                .child_elements()
                .filter(|c| ["p", "tbl", "sdt", "customXml"].iter().any(|t| is_w(c, t)))
                .last();
            if last_block.is_none_or(|b| is_w(b, "tbl")) {
                facts.cells.push(("cell-not-ending-in-paragraph", String::new()));
            }
        }
        _ => {}
    }
}

fn move_mark(local: &str) -> Option<(bool, bool)> {
    Some(match local {
        "moveFromRangeStart" => (true, true),
        "moveFromRangeEnd" => (true, false),
        "moveToRangeStart" => (false, true),
        "moveToRangeEnd" => (false, false),
        _ => return None,
    })
}

fn paraid_marks(element: &Element, facts: &mut ElementFacts) {
    let raw = element.attribute(Some(W14), "paraId");
    let text = element.attribute(Some(W14), "textId");
    if let (Some(text), None) = (text, raw) {
        facts.paraid.push(("textid-without-paraid", text.to_owned()));
    }
    if let Some(text) = text.filter(|t| !valid_long_hex(t)) {
        facts.paraid.push(("textid-out-of-range", text.to_owned()));
    }
    let Some(raw) = raw else { return };
    if !valid_long_hex(raw) {
        facts.paraid.push(("paraid-out-of-range", raw.to_owned()));
    }
    facts.para_ids.push(raw.to_uppercase());
}

/// The child-order problem of `element`'s children, if any (one per parent).
pub(crate) fn order_of(element: &Element, orders: &Orders, out: &mut Vec<Local>) {
    let parent = orders.prefixed(element);
    let Some(ranks) = orders.ranks(&parent) else { return };
    let unknown = ranks.len() + 1;
    let mut last = 0usize;
    let mut first = true;
    for child in element.child_elements() {
        let rank = orders.rank(ranks, child).unwrap_or(unknown);
        if !first && rank < last {
            out.push(("child-order", format!("{} in {parent}", orders.prefixed(child))));
            return;
        }
        first = false;
        last = rank;
    }
}

/// Walk `element` (with its ancestors inside the walked element) and everything below.
fn visit<'e>(
    element: &'e Element,
    ancestors: &mut Vec<&'e Element>,
    hidden: bool,
    orders: &Orders,
    facts: &mut ElementFacts,
) {
    head_facts(element, ancestors, hidden, facts);
    order_of(element, orders, &mut facts.order);
    let hide = hidden || element.name().is(Some(MC), "Fallback");
    ancestors.push(element);
    for node in element.children().iter() {
        if let Node::Element(child) = node {
            visit(child, ancestors, hide, orders, facts);
        }
    }
    ancestors.pop();
}

/// Python's `int()` of an attribute value: optional sign, digits, surrounding white space.
pub(crate) fn python_int(text: &str) -> Option<i64> {
    let text = text.trim();
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit() || b == b'_') || digits.starts_with('_') {
        return None;
    }
    text.replace('_', "").parse().ok()
}
