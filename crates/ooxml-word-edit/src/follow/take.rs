//! Owns: one kind of properties of a copy made its sample's (docx-agent's `follow._take`):
//! the sample's children copied in schema order but those the copy keeps of its own, which
//! stay; what changed told by each child's value ([`key`]: name, attributes, text and
//! children, whatever prefixes either document writes); and the copy document's style id
//! for a sample's ([`Styles`]: the same id within one document, by name from another, a
//! built-in one added where this document lacks it).
//!
//! Must not: decide which properties follow (the callers do), or write but on the copy's
//! editable tree and, for a style added, through the sink.

use crate::markdown::py_space;
use crate::names::W;
use crate::review::xml::{insert_in_order, make, set_attr};
use crate::sink::EditSink;
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::docx::{style_name, Docx};
use std::collections::BTreeMap;

/// An element's value (docx-agent's `_key`): its name by namespace, its attributes sorted,
/// its text trimmed and its child elements' values.
pub(super) fn key(work: &Work, node: Id) -> String {
    let mut out = String::new();
    if let Some(name) = work.name(node) {
        out.push_str(&format!("<{{{}}}{}", name.namespace().unwrap_or(""), name.local()));
    }
    let mut attributes: Vec<String> = work
        .attributes(node)
        .iter()
        .map(|a| {
            format!(
                "{{{}}}{}={:?}",
                a.name.namespace().unwrap_or(""),
                a.name.local(),
                &*a.value
            )
        })
        .collect();
    attributes.sort();
    for attribute in attributes {
        out.push(' ');
        out.push_str(&attribute);
    }
    out.push('|');
    out.push_str(work.text(node).trim_matches(py_space));
    for child in work.children(node) {
        if work.is_element(*child) {
            out.push_str(&key(work, *child));
        }
    }
    out.push('>');
    out
}

/// The children of `properties` but those of `keep`, by local name: each name's values in
/// order.
fn groups(work: &Work, properties: Option<Id>, keep: &[&str]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for child in properties.map(|p| work.children(p).to_vec()).unwrap_or_default() {
        if work.is_element(child) && !work.is_any(child, W, keep) {
            if let Some(name) = work.name(child).map(|n| n.local().to_owned()) {
                out.entry(name).or_default().push(key(work, child));
            }
        }
    }
    out
}

/// The copy document's style id for a sample's, with the styles a copy had to keep its own
/// for (`missing`: name and kind, in the order met).
pub(super) struct Styles<'a, 't, 's> {
    pub(super) sink: &'a mut EditSink<'t, 's>,
    /// The sample's document when it is another.
    pub(super) source: Option<&'a Docx>,
    /// Sample styles this document neither has nor would add, by name, with their kind.
    pub(super) missing: Vec<(String, String)>,
}

impl<'a, 't, 's> Styles<'a, 't, 's> {
    /// Styles for copies of a sample of `source` (`None`: this document).
    pub(super) fn new(sink: &'a mut EditSink<'t, 's>, source: Option<&'a Docx>) -> Self {
        Styles {
            sink,
            source,
            missing: Vec::new(),
        }
    }

    /// The id here of the sample's `kind` style `id` (`None`: none, or one this document
    /// neither has nor would add, which is remembered in `missing`).
    ///
    /// # Errors
    /// When the styles part cannot be read or written.
    pub(super) fn id(&mut self, id: Option<&str>, kind: &str) -> Result<Option<String>> {
        let Some(id) = id else { return Ok(None) };
        let Some(source) = self.source else {
            return Ok(Some(id.to_owned()));
        };
        let name = source.style(id).and_then(|s| style_name(s)).unwrap_or(id).to_owned();
        match crate::builtin::ensure(self.sink, &name, kind) {
            Ok(found) => Ok(Some(found)),
            Err(EditError::Refused(_)) => {
                match self.missing.iter_mut().find(|(n, _)| *n == name) {
                    Some(entry) => kind.clone_into(&mut entry.1),
                    None => self.missing.push((name, kind.to_owned())),
                }
                Ok(None)
            }
            Err(other) => Err(other),
        }
    }
}

/// What [`take`] takes: the sample's properties, the copy's own to keep, and the style
/// reference among them translated to this document's (its local name and kind).
pub(super) struct Taking<'k> {
    /// The local name of the properties (`tblPr`, `pPr`, ...).
    pub(super) local: &'k str,
    /// The children the copy keeps of its own.
    pub(super) keep: &'k [&'k str],
    /// The style reference's local name and the style's kind.
    pub(super) style: Option<(&'k str, &'k str)>,
}

/// `target`'s properties `taking.local` made `source`'s (the sample's), keeping its own of
/// `taking.keep`: the element now (`None` where left empty) and the local names that
/// changed, sorted.
///
/// # Errors
/// A style the sink cannot write.
pub(super) fn take(
    work: &mut Work,
    target: Id,
    source: Option<Id>,
    taking: &Taking<'_>,
    styles: &mut Styles<'_, '_, '_>,
    orders: &Orders,
) -> Result<(Option<Id>, Vec<String>)> {
    let old = work.find(target, W, taking.local);
    let new = make(work, taking.local, &[]);
    for child in source.map(|s| work.children(s).to_vec()).unwrap_or_default() {
        if !work.is_element(child) || work.is_any(child, W, taking.keep) {
            continue;
        }
        let taken = work.deep_copy(child);
        if let Some((_, kind)) = taking.style.filter(|(local, _)| work.is(taken, W, local)) {
            let wanted = work.get(taken, Some(W), "val").map(str::to_owned);
            match styles.id(wanted.as_deref(), kind)? {
                Some(id) => set_attr(work, taken, "val", &id),
                None => continue,
            }
        }
        insert_in_order(work, new, taken, orders);
    }
    for child in old.map(|o| work.children(o).to_vec()).unwrap_or_default() {
        if work.is_element(child) && work.is_any(child, W, taking.keep) {
            let kept = work.deep_copy(child);
            insert_in_order(work, new, kept, orders);
        }
    }
    let before = groups(work, old, taking.keep);
    let after = groups(work, Some(new), taking.keep);
    let names: Vec<String> = before
        .keys()
        .chain(after.keys())
        .filter(|name| before.get(*name) != after.get(*name))
        .cloned()
        .collect::<std::collections::BTreeSet<String>>()
        .into_iter()
        .collect();
    if names.is_empty() {
        return Ok((old, names));
    }
    let position = old.and_then(|o| work.index(o));
    if let Some(old) = old {
        work.remove(old);
    }
    if work.children(new).is_empty() {
        return Ok((None, names));
    }
    match position {
        Some(at) => work.insert(target, at, new),
        None => insert_in_order(work, target, new, orders),
    }
    Ok((Some(new), names))
}
