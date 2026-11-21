//! Owns: a table settled on a copy (docx-agent's `_accepted_copy` and `_fits_after`): the
//! table alone in the body of a scratch version of its package -- the main part's
//! relationships dropped, so no other story is read or settled -- its tracked changes
//! accepted or rejected by [`Review`] in a store of its own, and the table that comes out.
//!
//! Must not: touch the document the table is read from, or keep the scratch store.

use crate::names::W;
use crate::review::{Review, Selection};
use crate::validate::Unlimited;
use crate::WordDoc;
use ooxml_opc::name::rels_for;
use ooxml_store::{Caller, Clock, Store, StoreConfig};
use ooxml_word_model::docx::revisions::records;
use ooxml_xml::{Children, Element, Node, Tree};
use std::sync::Arc;
use std::time::Duration;

/// A clock for the scratch store, whose times nobody reads (a review dates nothing: the
/// records keep their own dates).
struct Stopped;

impl Clock for Stopped {
    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

/// `table` (an element of `doc`'s main part) with its tracked changes settled on a copy:
/// accepted (else rejected); only the records at `wanted` (places below the table) where
/// given, else all. `None` where the copy cannot be made, the review refuses, or no table
/// comes out.
pub(super) fn settled(
    doc: &WordDoc,
    table: &Arc<Element>,
    accept: bool,
    wanted: Option<&[Vec<u32>]>,
) -> Option<Arc<Element>> {
    let main = doc.docx().ok()?.main.clone();
    let package = doc.package();
    let tree = package.tree(&main).ok()??;
    let root = tree.root();
    let (k, body) = root
        .children()
        .iter()
        .enumerate()
        .find_map(|(k, n)| n.as_element().filter(|e| e.name().is(Some(W), "body")).map(|e| (k, e)))?;
    let mut children = vec![Node::Element(Arc::clone(table))];
    if let Some(section) = body.child_elements().last().filter(|e| e.name().is(Some(W), "sectPr")) {
        children.push(Node::Element(Arc::clone(section)));
    }
    let new_body = Arc::new(body.with_children(Children::from_vec(children)));
    let mut root_children: Vec<Node> = root.children().iter().cloned().collect();
    *root_children.get_mut(k)? = Node::Element(new_body);
    let new_root = Arc::new(root.with_children(Children::from_vec(root_children)));
    let mut scratch = package.with_tree(&main, Tree::from_root(new_root)).ok()?;
    let rels = rels_for(&main);
    if scratch.has_part(&rels) {
        scratch = scratch.without(&rels).ok()?;
    }
    let copy = WordDoc::open(scratch, Arc::new(Unlimited)).ok()?;
    let body_at = u32::try_from(k).ok()?;
    let selection = match wanted {
        None => Selection::All,
        Some(places) => {
            let docx = copy.docx().ok()?;
            let ids: Vec<String> = records(&docx)
                .into_iter()
                .filter(|r| {
                    r.path.split_first().is_some_and(|(b, rest)| {
                        *b == body_at
                            && rest
                                .split_first()
                                .is_some_and(|(t, below)| *t == 0 && places.iter().any(|p| p == below))
                    })
                })
                .map(|r| r.id)
                .collect();
            if ids.is_empty() {
                return None;
            }
            Selection::Ids(ids)
        }
    };
    let store = Store::new(copy, StoreConfig::default(), Arc::new(Stopped));
    let done = store.run(&Caller::anonymous(), &Review { accept, selection }).ok()?;
    let docx = done.version.doc().docx().ok()?;
    let body = docx.body()?.root().find(Some(W), "body")?.clone();
    body.find(Some(W), "tbl").cloned()
}
