//! Owns: a field's cached result written as docx-agent's `_write_result` writes it, in a
//! copy of the paragraph holding the field: the old result's first run takes the text (its
//! formatting, and any revision holding it, kept) and the others go; with no result yet, a
//! run in the begin run's formatting (`w:noProof` for page numbers, as Word writes them);
//! a separator added where there was none; `w:dirty` taken off.
//!
//! Must not: write but into the copy it is given, or compute a value.

use crate::links::new_w;
use crate::marks::insert_in_order;
use crate::names::W;
use crate::text::write::spell;
use crate::validate::Orders;
use crate::work::{Id, Work};
use ooxml_xml::{Element, Node};

/// The page-number fields, whose new result run is `w:noProof`.
const PAGES: [&str; 4] = ["PAGE", "PAGEREF", "NUMPAGES", "SECTIONPAGES"];

/// The node of `work` (a copy of `green`) at `path` below `green` (green child indexes:
/// text between children counts).
pub(crate) fn locate(work: &Work, green: &Element, path: &[u32]) -> Option<Id> {
    let mut node = work.root();
    let mut element = green;
    for step in path {
        let k = *step as usize;
        let children: Vec<&Node> = element.children().iter().collect();
        let before = children
            .get(..k)?
            .iter()
            .filter(|n| !matches!(n, Node::Text(_)))
            .count();
        node = *work.children(node).get(before)?;
        element = element.children().get(k)?.as_element()?;
    }
    Some(node)
}

/// The run holding `node` (its parent when that is a `w:r`), else the node.
pub(crate) fn run_of(work: &Work, node: Id) -> Id {
    work.parent(node).filter(|p| work.is(*p, W, "r")).unwrap_or(node)
}

/// Every element of `work` after `first` (not below it) and before `last`, in document
/// order (`_between` within one paragraph).
pub(crate) fn between(work: &Work, first: Id, last: Id) -> Vec<Id> {
    let mut order = Vec::new();
    preorder(work, work.root(), &mut order);
    let below_first = work.descendants(first);
    let Some(start) = order.iter().position(|n| *n == first) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for node in order.iter().skip(start + 1) {
        if *node == last {
            return out;
        }
        if work.is_element(*node) && !below_first.contains(node) {
            out.push(*node);
        }
    }
    Vec::new()
}

fn preorder(work: &Work, node: Id, out: &mut Vec<Id>) {
    out.push(node);
    for child in work.children(node).to_vec() {
        preorder(work, child, out);
    }
}

/// `value` into the first of `runs` (its properties and the revision holding it kept);
/// the other runs go, and a revision container left empty with them (`_retext`).
pub(crate) fn retext(work: &mut Work, runs: &[Id], value: &str) {
    let Some((&first, rest)) = runs.split_first() else {
        return;
    };
    for child in work.children(first).to_vec() {
        if work.is_element(child) && !work.is(child, W, "rPr") {
            work.remove(child);
        }
    }
    for child in spell(work, value, None) {
        work.append(first, child);
    }
    for &run in rest {
        let mut parent = work.parent(run);
        work.remove(run);
        while let Some(container) = parent.filter(|p| work.is_any(*p, W, &["ins", "del"])) {
            if work.children(container).iter().any(|c| work.is_element(*c)) {
                break;
            }
            parent = work.parent(container);
            work.remove(container);
        }
    }
}

/// A run with a copy of `properties` and `value`'s text.
pub(crate) fn result_run(work: &mut Work, parent: Id, value: &str, properties: Option<Id>) -> Id {
    let run = new_w(work, parent, "r", &[]);
    if let Some(properties) = properties {
        let copy = work.deep_copy(properties);
        work.append(run, copy);
    }
    for child in spell(work, value, None) {
        work.append(run, child);
    }
    run
}

/// A new `w:rPr` holding `w:noProof`.
pub(crate) fn no_proof_properties(work: &mut Work, parent: Id) -> Id {
    let properties = new_w(work, parent, "rPr", &[]);
    let proof = new_w(work, properties, "noProof", &[]);
    work.append(properties, proof);
    properties
}

/// A simple field's (`field`, its `w:fldSimple`) result set to `value`: whether it changed.
pub(crate) fn write_simple(work: &mut Work, field: Id, value: &str) -> bool {
    let runs = work.iter(field, W, &["r"]);
    let now: String = work
        .iter(field, W, &["t"])
        .iter()
        .map(|t| work.text(*t).to_owned())
        .collect();
    if now == value {
        return false;
    }
    if runs.is_empty() {
        let properties = no_proof_properties(work, field);
        let run = result_run(work, field, value, Some(properties));
        work.append(field, run);
    } else {
        retext(work, &runs, value);
    }
    true
}

/// The places of a complex field in a copy of its paragraph.
pub(crate) struct Complex {
    /// Its begin `w:fldChar`.
    pub(crate) begin: Id,
    /// Its separator.
    pub(crate) separate: Option<Id>,
    /// Its end.
    pub(crate) end: Id,
    /// Its kind.
    pub(crate) keyword: String,
}

/// A complex field's result set to `value` (`now`: its result's text as it stands):
/// whether it changed.
pub(crate) fn write_complex(work: &mut Work, field: &Complex, value: &str, now: &str, orders: &Orders) -> bool {
    if field.separate.is_some() && now == value {
        return false;
    }
    let begin_run = run_of(work, field.begin);
    let end_run = run_of(work, field.end);
    let begin_properties = work.find(begin_run, W, "rPr");
    let separate = match field.separate {
        Some(separate) => separate,
        None => {
            let parent = work.parent(end_run).unwrap_or(work.root());
            let run = new_w(work, parent, "r", &[]);
            if let Some(properties) = begin_properties {
                let copy = work.deep_copy(properties);
                work.append(run, copy);
            }
            let separate = new_w(work, run, "fldChar", &[("fldCharType", "separate")]);
            work.append(run, separate);
            work.add_previous(end_run, run);
            separate
        }
    };
    let separate_run = run_of(work, separate);
    let old: Vec<Id> = between(work, separate_run, end_run)
        .into_iter()
        .filter(|n| work.is(*n, W, "r"))
        .filter(|n| !ancestors(work, *n).iter().any(|a| work.is(*a, W, "r")))
        .filter(|n| {
            !work
                .children(*n)
                .iter()
                .any(|c| work.is_any(*c, W, &["fldChar", "instrText"]))
        })
        .collect();
    if old.is_empty() {
        let parent = work.parent(separate_run).unwrap_or(work.root());
        let mut properties = begin_properties.map(|p| work.deep_copy(p));
        if PAGES.contains(&field.keyword.as_str()) {
            let owner = properties.unwrap_or_else(|| new_w(work, parent, "rPr", &[]));
            if work.find(owner, W, "noProof").is_none() {
                let proof = new_w(work, owner, "noProof", &[]);
                insert_in_order(work, owner, proof, orders);
            }
            properties = Some(owner);
        }
        let run = new_w(work, parent, "r", &[]);
        if let Some(properties) = properties {
            work.append(run, properties);
        }
        for child in spell(work, value, None) {
            work.append(run, child);
        }
        work.add_next(separate_run, run);
    } else {
        retext(work, &old, value);
    }
    true
}

fn ancestors(work: &Work, node: Id) -> Vec<Id> {
    let mut out = Vec::new();
    let mut at = work.parent(node);
    while let Some(parent) = at {
        out.push(parent);
        at = work.parent(parent);
    }
    out
}

/// The path of `id` below the root of `work` in the element the copy builds (green
/// indexes: text between children counts).
pub(crate) fn green_path(work: &Work, id: Id) -> Option<Vec<u32>> {
    let mut path = Vec::new();
    let mut at = id;
    while let Some(parent) = work.parent(at) {
        let mut k = usize::from(!work.text(parent).is_empty());
        for child in work.children(parent) {
            if *child == at {
                break;
            }
            k += 1 + usize::from(!work.tail(*child).is_empty());
        }
        path.push(u32::try_from(k).ok()?);
        at = parent;
    }
    (at == work.root()).then(|| {
        path.reverse();
        path
    })
}
