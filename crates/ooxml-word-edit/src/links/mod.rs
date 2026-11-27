//! Owns: bookmarks and hyperlinks as docx-agent's `edit/links.py` edits them --
//! [`AddBookmark`], [`RenameBookmark`], [`RemoveBookmark`] and [`AddHyperlink`] -- and
//! what they share: a bookmark name's rule ([`check_name`]), a hyperlink's text in its
//! paragraph ([`hyperlink_span`]), the draws numbering a new marker again where the change
//! is applied ([`marker_draws`]), and what one edit says it did ([`LinkEdited`]).
//!
//! Must not: write but through the sink, or read less than it records reading: a bookmark
//! name is unique in the whole document, so these edits read every story -- or, adding one
//! at a place named by id, every bookmark of its name by condition.

mod bookmarks;
mod hyperlinks;
mod rename;

pub use bookmarks::AddBookmark;
pub use hyperlinks::AddHyperlink;
pub(crate) use rename::rename_in_instruction;
pub use rename::{RemoveBookmark, RenameBookmark};

use crate::doc::{Draw, DrawFrom};
use crate::marks::insert_in_order;
use crate::names::W;
use crate::text::atoms;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// What a link edit did, as docx-agent's `EditResult` says it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinkEdited {
    /// The id it names (`bm:Name`, `hl:<paragraph>/<n>`, a paragraph's), if any.
    pub id: Option<String>,
    /// Whether it changed the document.
    pub changed: bool,
    /// Ids it made.
    pub created: Vec<String>,
    /// Ids it removed.
    pub removed: Vec<String>,
    /// Ids renamed, old to new (a bookmark's).
    pub renamed: ooxml_word_model::ids::Renames,
    /// What it could not do, or what is left dangling.
    pub warnings: Vec<String>,
}

/// A bookmark name as Word takes one: a letter, then letters, digits or underscores, at
/// most 40 characters -- or Word's hidden `_Ref` names (docx-agent's `check_name`).
///
/// # Errors
/// `refused`, saying the rule.
pub(crate) fn check_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let plain = chars.next().is_some_and(|c| c.is_alphabetic()) && chars.all(word) && name.chars().count() <= 40;
    let hidden = name
        .strip_prefix("_Ref")
        .is_some_and(|rest| (1..=36).contains(&rest.chars().count()) && rest.chars().all(word));
    if plain || hidden {
        return Ok(());
    }
    Err(EditError::Refused(format!(
        "{} is not a bookmark name: a letter, then letters, digits or underscores, at most 40 characters -- or Word's hidden _Ref<digits>",
        ooxml_base::pyfmt::repr_str(name)
    )))
}

/// The characters `[start, end)` of `paragraph`'s current text that its `index`-th
/// hyperlink covers (`Hyperlink.range`); `None` when it has no such hyperlink.
#[must_use]
pub fn hyperlink_span(paragraph: &Arc<Element>, index: usize) -> Option<(usize, usize)> {
    let work = Work::of(paragraph, Vec::new());
    let link = *work.iter(work.root(), W, &["hyperlink"]).get(index)?;
    let items = atoms(&work, work.root());
    let mine: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, a)| a.run.is_some_and(|r| inside(&work, r, link)))
        .map(|(k, _)| k)
        .collect();
    match (mine.first(), mine.last()) {
        (Some(a), Some(b)) => Some((*a, b + 1)),
        _ => Some((0, 0)),
    }
}

/// Whether `node` is below `ancestor`.
pub(crate) fn inside(work: &Work, node: Id, ancestor: Id) -> bool {
    let mut at = work.parent(node);
    while let Some(parent) = at {
        if parent == ancestor {
            return true;
        }
        at = work.parent(parent);
    }
    false
}

/// The draws numbering again, where the change is applied, the markers among `nodes`
/// (`locals`, Word's names) whose `w:id` is `value`: the edit's annotation id at `offset`.
pub(crate) fn marker_draws(nodes: &[Node], locals: &[&str], value: i64, offset: u32) -> Vec<Draw> {
    let wanted = value.to_string();
    let mut out = Vec::new();
    for (k, node) in nodes.iter().enumerate() {
        let (Node::Element(element), Ok(k)) = (node, u32::try_from(k)) else {
            continue;
        };
        let mut stack: Vec<(&Element, Vec<u32>)> = vec![(element, Vec::new())];
        while let Some((e, path)) = stack.pop() {
            let named = e.name().namespace() == Some(W) && locals.contains(&e.name().local());
            if named && e.attribute(Some(W), "id") == Some(wanted.as_str()) {
                if let Some(attribute) = e.attributes().iter().find(|a| a.name.is(Some(W), "id")) {
                    out.push(Draw {
                        from: DrawFrom::Annotation(offset),
                        node: k,
                        at: path.clone(),
                        attribute: Arc::clone(&attribute.name),
                    });
                }
            }
            for (j, child) in e.children().iter().enumerate() {
                if let (Node::Element(child), Ok(j)) = (child, u32::try_from(j)) {
                    let mut below = path.clone();
                    below.push(j);
                    stack.push((child, below));
                }
            }
        }
    }
    out
}

/// A new element `local` in Word's namespace, named as its parent's scope writes it, with
/// `attributes` (Word's names) in order.
pub(crate) fn new_w(work: &mut Work, parent: Id, local: &str, attributes: &[(&str, &str)]) -> Id {
    let node = work.new_element(work.element_name(parent, W, local, "w"));
    for (name, value) in attributes {
        let name = work.qname(W, name, "w");
        work.set(node, name, value);
    }
    node
}

/// Give `run` the character style `style` (`w:rPr/w:rStyle`), as docx-agent's
/// `_set_child` writes it: an existing one keeps its place, a new one goes in schema order.
pub(crate) fn set_run_style(work: &mut Work, run: Id, style: &str, orders: &crate::validate::Orders) {
    let properties = work.find(run, W, "rPr").unwrap_or_else(|| {
        let p = new_w(work, run, "rPr", &[]);
        insert_in_order(work, run, p, orders);
        p
    });
    let node = work.find(properties, W, "rStyle").unwrap_or_else(|| {
        let s = new_w(work, properties, "rStyle", &[]);
        insert_in_order(work, properties, s, orders);
        s
    });
    let val = work.qname(W, "val", "w");
    work.set(node, val, style);
}

#[cfg(test)]
mod tests {
    use super::check_name;

    #[test]
    fn bookmark_names() {
        assert!(check_name("Results").is_ok());
        assert!(check_name("_Ref123456789").is_ok());
        assert!(check_name("1st").is_err());
        assert!(check_name("_Toc1").is_err());
        assert!(check_name(&"a".repeat(41)).is_err());
        assert!(check_name("with space").is_err());
    }
}
