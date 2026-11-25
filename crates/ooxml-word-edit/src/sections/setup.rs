//! Owns: a section's page setup written into its `w:sectPr` as docx-agent's
//! `edit/sections.py` writes it ([`apply`]: the start, page size and orientation, margins
//! and columns, attributes in the order Word writes them), and, tracked, the old value of
//! each kind that changed recorded in a `w:sectPrChange` ([`record_change`], merged into a
//! change already there; a kind that was absent recorded in its "off" form).
//!
//! Must not: read outside the `w:sectPr`, or allocate revision ids but through its stamp.

use crate::names::W;
use crate::policy::Stamp;
use crate::review::xml::{insert_in_order, make};
use crate::units::Twips;
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result};

/// The page setup a call sets; `None` leaves a property as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SectionValues {
    /// How the section starts (`nextPage`, `continuous`...).
    pub start: Option<String>,
    /// `portrait` or `landscape`.
    pub orientation: Option<String>,
    /// The page's width.
    pub page_width: Option<Twips>,
    /// The page's height.
    pub page_height: Option<Twips>,
    /// The margins: top, right, bottom, left.
    pub margins: [Option<Twips>; 4],
    /// Text columns.
    pub columns: Option<i64>,
}

impl SectionValues {
    /// The names docx-agent's `set_section` takes for the values given, in the order the
    /// tool gives them.
    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        let given = [
            ("start", self.start.is_some()),
            ("orientation", self.orientation.is_some()),
            ("page_width", self.page_width.is_some()),
            ("page_height", self.page_height.is_some()),
        ];
        out.extend(given.iter().filter(|(_, g)| *g).map(|(n, _)| *n));
        for (name, value) in ["margin_top", "margin_right", "margin_bottom", "margin_left"]
            .into_iter()
            .zip(self.margins)
        {
            if value.is_some() {
                out.push(name);
            }
        }
        if self.columns.is_some() {
            out.push("columns");
        }
        out
    }
}

const PGMAR_ORDER: [&str; 7] = ["top", "right", "bottom", "left", "header", "footer", "gutter"];
const COLS_ORDER: [&str; 4] = ["num", "sep", "space", "equalWidth"];

/// The child `w:{local}` of `sect`, made in schema order when missing.
fn child(work: &mut Work, sect: Id, local: &str, orders: &Orders) -> Id {
    if let Some(found) = work.find(sect, W, local) {
        return found;
    }
    let node = make(work, local, &[]);
    insert_in_order(work, sect, node, orders);
    node
}

fn drop_all(work: &mut Work, sect: Id, local: &str) {
    for node in work.children(sect).to_vec() {
        if work.is(node, W, local) {
            work.remove(node);
        }
    }
}

fn int_attr(work: &Work, node: Id, local: &str, default: i64) -> i64 {
    work.get(node, Some(W), local)
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(default)
}

/// Set (`None`: drop) `w:` attributes of `node`, rewriting them in `order` -- the order
/// Word writes them in (docx-agent's `_set_attributes`).
fn set_attributes(work: &mut Work, node: Id, values: &[(&str, Option<String>)], order: &[&str]) {
    let attributes = work.attributes(node).to_vec();
    let mut current: Vec<(String, Option<String>)> = Vec::new();
    let mut others = Vec::new();
    for a in &attributes {
        if a.name.namespace() == Some(W) {
            current.push((a.name.local().to_owned(), Some(a.value.to_string())));
        } else {
            others.push(a.clone());
        }
    }
    for (name, value) in values {
        match current.iter_mut().find(|(n, _)| n == name) {
            Some(found) => found.1.clone_from(value),
            None => current.push(((*name).to_owned(), value.clone())),
        }
    }
    for a in &attributes {
        work.remove_attr(node, a.name.namespace(), a.name.local());
    }
    for name in order {
        if let Some(k) = current.iter().position(|(n, v)| n == name && v.is_some()) {
            let (n, v) = current.remove(k);
            let qname = work.qname(W, &n, "w");
            work.set(node, qname, &v.unwrap_or_default());
        }
    }
    for (n, v) in current {
        if let Some(v) = v {
            let qname = work.qname(W, &n, "w");
            work.set(node, qname, &v);
        }
    }
    for a in others {
        work.set(node, a.name, &a.value);
    }
}

/// Write `values` into the `w:sectPr` `sect` (docx-agent's `apply`).
pub(crate) fn apply(work: &mut Work, sect: Id, values: &SectionValues, orders: &Orders) {
    if let Some(start) = &values.start {
        if start == "nextPage" {
            drop_all(work, sect, "type");
        } else {
            let node = child(work, sect, "type", orders);
            let val = work.qname(W, "val", "w");
            work.set(node, val, start);
        }
    }
    if values.page_width.is_some() || values.page_height.is_some() || values.orientation.is_some() {
        let size = child(work, sect, "pgSz", orders);
        let mut width = int_attr(work, size, "w", 11906);
        let mut height = int_attr(work, size, "h", 16838);
        if let Some(w) = values.page_width {
            width = w.0;
        }
        if let Some(h) = values.page_height {
            height = h.0;
        }
        let mut orient = work.get(size, Some(W), "orient").map(str::to_owned);
        if let Some(orientation) = &values.orientation {
            let landscape = orientation == "landscape";
            if (landscape && width < height) || (!landscape && width > height) {
                std::mem::swap(&mut width, &mut height);
            }
            orient = landscape.then(|| "landscape".to_owned());
        }
        let wanted = [
            ("w", Some(width.to_string())),
            ("h", Some(height.to_string())),
            ("orient", orient),
        ];
        set_attributes(work, size, &wanted, &["w", "h", "orient", "code"]);
    }
    if values.margins.iter().any(Option::is_some) {
        let node = child(work, sect, "pgMar", orders);
        let defaults = [1440, 1440, 1440, 1440, 720, 720, 0];
        let mut written: Vec<(&str, Option<String>)> = PGMAR_ORDER
            .iter()
            .zip(defaults)
            .map(|(side, default)| {
                let value = work
                    .get(node, Some(W), side)
                    .filter(|v| !v.is_empty())
                    .map_or_else(|| default.to_string(), str::to_owned);
                (*side, Some(value))
            })
            .collect();
        for (k, value) in values.margins.iter().enumerate() {
            if let (Some(value), Some(slot)) = (value, written.get_mut(k)) {
                slot.1 = Some(value.0.to_string());
            }
        }
        set_attributes(work, node, &written, &PGMAR_ORDER);
    }
    if let Some(count) = values.columns {
        let node = child(work, sect, "cols", orders);
        let count = if count == 0 { 1 } else { count };
        let mut attributes: Vec<(&str, Option<String>)> = vec![("num", (count > 1).then(|| count.to_string()))];
        if count <= 1 {
            attributes.push(("sep", None));
            attributes.push(("equalWidth", None));
            drop_all(work, node, "col");
        }
        set_attributes(work, node, &attributes, &COLS_ORDER);
    }
}

/// What an element says, whatever prefixes its names have (docx-agent's `_canonical`).
pub(crate) fn canonical(work: &Work, id: Id) -> String {
    let Some(name) = work.name(id) else {
        return String::new();
    };
    let mut attributes: Vec<String> = work
        .attributes(id)
        .iter()
        .map(|a| format!("{{{}}}{}={}", a.name.namespace().unwrap_or(""), a.name.local(), a.value))
        .collect();
    attributes.sort();
    let children: Vec<String> = work
        .children(id)
        .iter()
        .filter(|c| work.is_element(**c))
        .map(|c| canonical(work, *c))
        .collect();
    format!(
        "<{{{}}}{} {}>{}[{}]",
        name.namespace().unwrap_or(""),
        name.local(),
        attributes.join(" "),
        work.text(id).trim(),
        children.join(",")
    )
}

/// A `w:sectPr`'s property children by Word's name (references and the change record
/// aside; the last of a name wins), in order.
pub(crate) fn kinds(work: &Work, sect: Id) -> Vec<(String, Id)> {
    let mut out: Vec<(String, Id)> = Vec::new();
    for child in work.children(sect).to_vec() {
        if !work.is_element(child) || work.is_any(child, W, &["headerReference", "footerReference", "sectPrChange"]) {
            continue;
        }
        let local = work.name(child).map(|n| n.local().to_owned()).unwrap_or_default();
        match out.iter_mut().find(|(n, _)| *n == local) {
            Some(found) => found.1 = child,
            None => out.push((local, child)),
        }
    }
    out
}

/// The "off" form a tracked change records for a kind that was absent (`_ABSENT`).
fn absent(work: &mut Work, kind: &str) -> Option<Id> {
    Some(match kind {
        "titlePg" => make(work, "titlePg", &[("val", "0")]),
        "vAlign" => make(work, "vAlign", &[("val", "top")]),
        "type" => make(work, "type", &[("val", "nextPage")]),
        "pgNumType" => make(work, "pgNumType", &[("fmt", "decimal")]),
        "lnNumType" => make(work, "lnNumType", &[("countBy", "0")]),
        "footnotePr" => make(work, "footnotePr", &[]),
        "endnotePr" => make(work, "endnotePr", &[]),
        "cols" => make(work, "cols", &[("num", "1")]),
        _ => return None,
    })
}

/// Attributes whose absence means a value Word can be told (`DEFAULTS`).
const DEFAULTS: [(&str, &str, &str); 6] = [
    ("cols", "num", "1"),
    ("cols", "sep", "0"),
    ("cols", "equalWidth", "1"),
    ("pgSz", "orient", "portrait"),
    ("pgNumType", "fmt", "decimal"),
    ("lnNumType", "countBy", "0"),
];

/// Kinds whose meaning is their children.
const PARENTS: [&str; 3] = ["cols", "footnotePr", "endnotePr"];

fn is_absent_marker(work: &mut Work, node: Id) -> bool {
    let kind = work.name(node).map(|n| n.local().to_owned()).unwrap_or_default();
    if work.children(node).iter().any(|c| work.is_element(*c)) {
        return false;
    }
    let Some(marker) = absent(work, &kind) else {
        return false;
    };
    canonical(work, marker) == canonical(work, node)
}

/// Reject one recorded kind into `sect` as Word does (docx-agent's `restore_kind`).
fn restore_kind(work: &mut Work, sect: Id, recorded: Id, orders: &Orders) {
    let kind = work.name(recorded).map(|n| n.local().to_owned()).unwrap_or_default();
    let same = work.find(sect, W, &kind);
    if is_absent_marker(work, recorded) {
        if let Some(same) = same {
            work.remove(same);
        }
        return;
    }
    let same = match same {
        None => {
            let copy = work.deep_copy(recorded);
            insert_in_order(work, sect, copy, orders);
            copy
        }
        Some(same) => {
            for a in work.attributes(recorded).to_vec() {
                work.set(same, a.name, &a.value);
            }
            let has_children = work.children(recorded).iter().any(|c| work.is_element(*c));
            if PARENTS.contains(&kind.as_str()) || has_children {
                for c in work.children(same).to_vec() {
                    work.remove(c);
                }
                for c in work.children(recorded).to_vec() {
                    if work.is_element(c) {
                        let copy = work.deep_copy(c);
                        work.append(same, copy);
                    }
                }
            }
            same
        }
    };
    for (k, name, value) in DEFAULTS {
        if k == kind && work.get(same, Some(W), name) == Some(value) {
            work.remove_attr(same, Some(W), name);
        }
    }
}

/// The old value each newly changed kind is recorded with: its copy (an attribute the
/// change added at the value its absence meant), or its "off" form when it was absent.
fn old_values(
    work: &mut Work,
    before: &[(String, Id)],
    after: &[(String, Id)],
    new_kinds: &[String],
) -> Result<Vec<(String, Id)>> {
    let lookup = |list: &[(String, Id)], kind: &str| list.iter().find(|(n, _)| n == kind).map(|(_, id)| *id);
    let mut wanted = Vec::new();
    for kind in new_kinds {
        if let Some(was) = lookup(before, kind) {
            let copy = work.deep_copy(was);
            if let Some(now) = lookup(after, kind) {
                for a in work.attributes(now).to_vec() {
                    let local = a.name.local().to_owned();
                    let had = work.get(was, a.name.namespace(), &local).is_some();
                    if let Some((_, _, value)) = DEFAULTS.iter().find(|(k, n, _)| k == kind && *n == local) {
                        if !had {
                            work.set(copy, a.name, value);
                        }
                    }
                }
            }
            wanted.push((kind.clone(), copy));
        } else if let Some(marker) = absent(work, kind) {
            wanted.push((kind.clone(), marker));
        } else {
            return Err(EditError::Refused(format!(
                "a tracked change cannot add {kind} where the section had none"
            )));
        }
    }
    Ok(wanted)
}

/// Record in `sect`'s `w:sectPrChange` the old value of every kind that differs from
/// `before` (docx-agent's `record_change`).
///
/// # Errors
/// Another author's pending change, or a kind added that has no "off" form.
pub(crate) fn record_change(
    work: &mut Work,
    sect: Id,
    before: &[(String, Id)],
    stamp: &mut Stamp,
    orders: &Orders,
) -> Result<()> {
    let after = kinds(work, sect);
    let record = work.find(sect, W, "sectPrChange");
    let old = record.and_then(|r| work.find(r, W, "sectPr"));
    let recorded = old.map(|o| kinds(work, o)).unwrap_or_default();
    let lookup = |list: &[(String, Id)], kind: &str| list.iter().find(|(n, _)| n == kind).map(|(_, id)| *id);
    let mut all: Vec<String> = before.iter().map(|(n, _)| n.clone()).collect();
    for (n, _) in &after {
        if !all.contains(n) {
            all.push(n.clone());
        }
    }
    let changed: Vec<String> = all
        .into_iter()
        .filter(|k| lookup(before, k).map(|i| canonical(work, i)) != lookup(&after, k).map(|i| canonical(work, i)))
        .collect();
    let new_kinds: Vec<String> = changed
        .iter()
        .filter(|k| lookup(&recorded, k).is_none())
        .cloned()
        .collect();
    if let Some(record) = record {
        if work.get(record, Some(W), "author") != Some(stamp.author()) && !new_kinds.is_empty() {
            return Err(EditError::Refused(
                "the section has another author's pending property change: accept or reject it first".to_owned(),
            ));
        }
    }
    let mut wanted: Vec<(String, Id)> = recorded.clone();
    wanted.extend(old_values(work, before, &after, &new_kinds)?);
    let mut kept = Vec::new();
    for (kind, node) in wanted {
        let now = lookup(&after, &kind);
        if now.is_none() && is_absent_marker(work, node) {
            continue;
        }
        if let Some(now) = now {
            let holder = make(work, "sectPr", &[]);
            let trial = work.deep_copy(now);
            work.append(holder, trial);
            restore_kind(work, holder, node, orders);
            let restored = work.find(holder, W, &kind);
            if restored.map(|r| canonical(work, r)) == Some(canonical(work, now)) {
                continue;
            }
        }
        kept.push(node);
    }
    if let Some(record) = record {
        work.remove(record);
    }
    if kept.is_empty() {
        return Ok(());
    }
    let record = stamp.make(work, "sectPrChange");
    let inner = make(work, "sectPr", &[]);
    work.append(record, inner);
    for node in kept {
        let copy = work.deep_copy(node);
        insert_in_order(work, inner, copy, orders);
    }
    insert_in_order(work, sect, record, orders);
    Ok(())
}
