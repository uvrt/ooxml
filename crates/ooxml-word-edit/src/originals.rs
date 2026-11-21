//! Owns: [`Originals`], which paragraphs of a document were the file's when the session
//! opened it and which revision records this session wrote (docx-agent's
//! `tools/originals.py`, round 57, R122) -- what an edit's watch on removed text and
//! `check`'s `accept_loses_original` read.
//!
//! **Original**: a paragraph is the file's when it held text in the file as opened (the
//! session's opened version, its final view), found by the id the document gave it there,
//! followed through the renames since (the version's aliases). Its text is the final
//! view's less what this session inserted as tracked changes (records whose author and date
//! the file did not have). A cell is original when one of its paragraphs is and that text
//! is not empty.
//!
//! Must not: change the document, or keep anything between calls (what the file had is the
//! opened version's, read on demand).

use crate::facts::py_space;
use crate::WordDoc;
use ooxml_word_model::docx::revisions::records;
use ooxml_word_model::docx::text::{paragraph_text, View};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::{Entry, W};
use ooxml_xml::{Children, Element, Node};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A revision record's author and date.
type Stamp = (Option<String>, Option<String>);

/// Insertions: their content is this session's when the record is.
const INSERTED: [&str; 2] = ["ins", "moveTo"];

/// Which paragraphs of a document were the file's, and which records this session wrote.
pub struct Originals<'a> {
    /// The document as opened.
    opened: Arc<Docx>,
    /// The version the edit starts from (its renames).
    now: &'a WordDoc,
    /// Per part, the records' stamps in the file as opened.
    stamps: HashMap<String, HashSet<Stamp>>,
    /// Each id now and the ids that name it through renames (made on first use).
    named_by: Option<HashMap<String, Vec<String>>>,
}

impl<'a> Originals<'a> {
    /// Over the document `opened` was, and its version `now`.
    pub fn new(opened: Arc<Docx>, now: &'a WordDoc) -> Originals<'a> {
        Originals {
            opened,
            now,
            stamps: HashMap::new(),
            named_by: None,
        }
    }

    /// Whether the record `record` of `part` was written since the document was opened: a
    /// record whose author and date the file had was there; an edit's records carry its time.
    pub fn session_record(&mut self, part: &str, record: &Element) -> bool {
        let Some(date) = record.attribute(Some(W), "date") else {
            return false;
        };
        let opened = &self.opened;
        let stamps = self.stamps.entry(part.to_owned()).or_insert_with(|| {
            records(opened)
                .into_iter()
                .filter(|r| r.part == part)
                .map(|r| {
                    let get = |l: &str| r.element.attribute(Some(W), l).map(str::to_owned);
                    (get("author"), get("date"))
                })
                .collect()
        });
        let author = record.attribute(Some(W), "author").map(str::to_owned);
        !stamps.contains(&(author, Some(date.to_owned())))
    }

    /// Whether the paragraph `id` (of `part`, now) was in the file as opened, with text: an
    /// id the file gave a paragraph with text, followed through the renames since, is it.
    pub fn paragraph(&mut self, part: &str, id: &str) -> bool {
        let ids = self.now.ids();
        let named_by = self.named_by.get_or_insert_with(|| {
            let mut out: HashMap<String, Vec<String>> = HashMap::new();
            for old in ids.aliases().keys() {
                out.entry(ids.follow(old)).or_default().push(old.clone());
            }
            out
        });
        let itself = (ids.follow(id) == id).then(|| id.to_owned());
        let Some(opened) = self.opened.part(part) else {
            return false;
        };
        itself
            .into_iter()
            .chain(named_by.get(id).into_iter().flatten().cloned())
            .any(|old| match opened.ids.get(&old) {
                Some(Entry::Paragraph(k)) => opened.ids.paragraphs().get(k).is_some_and(|p| holds_text(&p.element)),
                _ => false,
            })
    }

    /// The final view's text of `node` (a paragraph or a cell), less what this session
    /// inserted as tracked changes, white space runs as one space.
    pub fn text(&mut self, part: &str, node: &Arc<Element>) -> String {
        let mut out = Vec::new();
        for paragraph in paragraphs(node) {
            let kept = self.without_inserted(part, &paragraph);
            out.push(paragraph_text(&kept, View::Current));
        }
        collapse(&out.join(" "))
    }

    /// `element` without the insertions this session made (a copy where it held any).
    fn without_inserted(&mut self, part: &str, element: &Arc<Element>) -> Arc<Element> {
        let mut changed = false;
        let mut children = Vec::new();
        for node in element.children().iter() {
            if let Node::Element(child) = node {
                let name = child.name();
                if name.namespace() == Some(W) && INSERTED.contains(&name.local()) && self.session_record(part, child) {
                    changed = true;
                    continue;
                }
                let kept = self.without_inserted(part, child);
                changed |= !Arc::ptr_eq(&kept, child);
                children.push(Node::Element(kept));
            } else {
                children.push(node.clone());
            }
        }
        if changed {
            Arc::new(element.with_children(Children::from_vec(children)))
        } else {
            Arc::clone(element)
        }
    }
}

/// Whether a paragraph held text in the final view: a `w:t` with more than white space.
fn holds_text(element: &Element) -> bool {
    element.children().iter().any(|node| match node {
        Node::Element(child) if child.name().is(Some(W), "t") => !child.text_content().chars().all(py_space),
        Node::Element(child) => holds_text(child),
        _ => false,
    })
}

/// A paragraph itself, else the `w:p` under `node`, in document order (lxml's `iter`).
fn paragraphs(node: &Arc<Element>) -> Vec<Arc<Element>> {
    fn walk(element: &Arc<Element>, out: &mut Vec<Arc<Element>>) {
        if element.name().is(Some(W), "p") {
            out.push(Arc::clone(element));
        }
        for child in element.child_elements() {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    walk(node, &mut out);
    if node.name().is(Some(W), "p") {
        out.truncate(1);
    }
    out
}

/// What the final view shows of `node`, stripped (`_shown`).
pub fn shown(node: &Arc<Element>) -> String {
    let text: String = paragraphs(node)
        .iter()
        .map(|p| paragraph_text(p, View::Current))
        .collect();
    text.trim_matches(py_space).to_owned()
}

/// White space runs as one space, none at the ends (`" ".join(text.split())`).
pub(crate) fn collapse(text: &str) -> String {
    text.split(py_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
