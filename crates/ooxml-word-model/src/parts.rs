//! A Word package's parts as docx-agent finds them (`WordPackage`, `OpcPackage`): the main
//! part, the parts related to it by type, its story parts, and the elements of a part that
//! the readers read (of an `mc:AlternateContent`, the branch [`crate::mc::chosen`] says).
//!
//! Owns: those rules, read from a version of an OPC package; relationships read as Python
//! keys them (a dictionary by id: the first place of an id, its last relationship).
//!
//! Must not: change a package, cache across versions, or know a tool's reply. (It moved here
//! from `ooxml-tools-word` in phase P1: the ids, the addresses and the reads use it.)

use ooxml_opc::{OpcError, Package, Relationship};
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::Arc;

/// WordprocessingML's main namespace.
pub const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
/// Markup compatibility (`mc:Fallback`).
pub const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
/// Word 2010's namespace (`w14:paraId`).
pub const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
/// Word 2012's namespace (`w15:commentEx`).
pub const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";
/// Word 2016's comment ids namespace (`w16cid:commentId`).
pub const W16CID: &str = "http://schemas.microsoft.com/office/word/2016/wordml/cid";

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";
/// The package's main part.
pub const REL_OFFICE_DOCUMENT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
/// The settings part.
pub const REL_SETTINGS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings";
/// The comments part.
pub const REL_COMMENTS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
/// Comments' threads and states.
pub const REL_COMMENTS_EXTENDED: &str = "http://schemas.microsoft.com/office/2011/relationships/commentsExtended";
/// Comments' durable ids.
pub const REL_COMMENTS_IDS: &str = "http://schemas.microsoft.com/office/2016/09/relationships/commentsIds";
/// The core properties.
pub const REL_CORE: &str = "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";

/// A part's relationships as a dictionary by id keeps them: each id once, at the place it
/// first appears, with the last relationship of that id -- read in one pass, each id's
/// found at once ([`PartRelationships::get`]). A version's reading keeps one per part
/// (`crate::docx::Docx::relationships`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartRelationships {
    items: Vec<Relationship>,
    by_id: HashMap<String, usize>,
}

impl PartRelationships {
    /// The relationships of `source` in `package`.
    ///
    /// # Errors
    /// Its relationships part is not XML.
    pub fn read(package: &Package, source: &str) -> Result<PartRelationships, OpcError> {
        let mut out = PartRelationships::default();
        for rel in package.relationships(source)?.iter() {
            match out.by_id.get(&rel.id).and_then(|&k| out.items.get_mut(k)) {
                Some(slot) => *slot = rel.clone(),
                None => {
                    out.by_id.insert(rel.id.clone(), out.items.len());
                    out.items.push(rel.clone());
                }
            }
        }
        Ok(out)
    }

    /// The relationship of id `id` (the last of that id).
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Relationship> {
        self.by_id.get(id).and_then(|&k| self.items.get(k))
    }

    /// Every relationship, each id once, in the order ids first appear.
    #[must_use]
    pub fn items(&self) -> &[Relationship] {
        &self.items
    }
}

/// The relationships of `source` as a dictionary by id keeps them: each id once, at the
/// place it first appears, with the last relationship of that id.
pub fn relationships(package: &Package, source: &str) -> Result<Vec<Relationship>, OpcError> {
    Ok(PartRelationships::read(package, source)?.items)
}

/// The internal targets of `source`'s relationships of type `rel_type`, in order.
pub fn related_parts(package: &Package, source: &str, rel_type: &str) -> Result<Vec<String>, OpcError> {
    Ok(relationships(package, source)?
        .into_iter()
        .filter(|r| r.rel_type == rel_type)
        .filter_map(|r| r.target_part)
        .collect())
}

/// The first part `source` relates by `rel_type`, where the package has it.
pub fn related(package: &Package, source: &str, rel_type: &str) -> Result<Option<String>, OpcError> {
    let first = related_parts(package, source, rel_type)?.into_iter().next();
    Ok(first.filter(|part| package.has_part(part)))
}

/// The main document part: the target of the package's `officeDocument` relationship, or
/// `word/document.xml`; `None` when there is neither (not a Word package).
pub fn main_part(package: &Package) -> Result<Option<String>, OpcError> {
    if let Some(part) = related_parts(package, "", REL_OFFICE_DOCUMENT)?.into_iter().next() {
        return Ok(Some(part));
    }
    Ok(package
        .has_part("word/document.xml")
        .then(|| "word/document.xml".to_owned()))
}

/// The relationship types' namespace (`.../relationships/styles`) and what ISO/IEC 29500
/// Strict names it.
const REL_TRANSITIONAL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";
const REL_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships/";

/// Whether `written`, a relationship's type, is `transitional` (a transitional type), or
/// the type a strict document writes for it (`.../relationships/styles` written under
/// `purl.oclc.org`): what the layout finds a strict document's parts by.
#[must_use]
pub fn rel_type_is(written: &str, transitional: &str) -> bool {
    written == transitional
        || transitional
            .strip_prefix(REL_TRANSITIONAL)
            .is_some_and(|kind| written.strip_prefix(REL_STRICT) == Some(kind))
}

/// The `officeDocument` relationship as ISO/IEC 29500 Strict names it.
const REL_OFFICE_DOCUMENT_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument";

/// Whether `package` is a strict Open XML document (ISO/IEC 29500 Strict, Word's "Strict
/// Open XML Document"): its main relationship is strict's, or its main part's root is
/// strict WordprocessingML. Its other vocabularies, relationship types and units are
/// strict's too (`35.40pt` for 708 twips), which the reading tools and the edits do not
/// read or write: only its layout reads it, through the namespace-exact views.
///
/// # Errors
/// The package's relationships cannot be read.
pub fn strict(package: &Package) -> Result<bool, OpcError> {
    if !related_parts(package, "", REL_OFFICE_DOCUMENT_STRICT)?.is_empty() {
        return Ok(true);
    }
    let Some(main) = main_part(package)? else {
        return Ok(false);
    };
    Ok(package.tree(&main)?.is_some_and(|tree| {
        let name = tree.root().name();
        crate::xmlread::Ns::W.holds(name.namespace()) && name.namespace() != Some(W)
    }))
}

/// The main part and every story part: the parts ids are read from (docx-agent's
/// `Document._parts`).
pub fn parts(package: &Package) -> Result<Vec<String>, OpcError> {
    let Some(main) = main_part(package)? else {
        return Ok(Vec::new());
    };
    let mut out = vec![main.clone()];
    out.extend(story_parts(package, &main)?);
    Ok(out)
}

/// Every part related to `main` that holds a story, in relationship order by type:
/// headers, footers, footnotes, endnotes, comments.
pub fn story_parts(package: &Package, main: &str) -> Result<Vec<String>, OpcError> {
    let mut out: Vec<String> = Vec::new();
    for kind in ["header", "footer", "footnotes", "endnotes", "comments"] {
        for part in related_parts(package, main, &format!("{REL}{kind}"))? {
            if !out.contains(&part) && package.has_part(&part) {
                out.push(part);
            }
        }
    }
    Ok(out)
}

/// The elements below `root` named `{namespace}local`, in document order, leaving out
/// those inside a branch of an `mc:AlternateContent` the readers do not read (a Fallback
/// repeats its Choice's content: [`crate::mc::chosen`]).
pub fn live_elements(root: &Element, namespace: &str, local: &str) -> Vec<Arc<Element>> {
    let mut found = Vec::new();
    let mut stack = vec![(root, root.children().iter())];
    while let Some((parent, top)) = stack.last_mut() {
        match top.next() {
            Some(Node::Element(e)) => {
                if crate::mc::unchosen(parent, e) {
                    continue;
                }
                if e.name().is(Some(namespace), local) {
                    found.push(Arc::clone(e));
                }
                stack.push((e, e.children().iter()));
            }
            Some(_) => {}
            None => {
                stack.pop();
            }
        }
    }
    found
}

/// The elements below `root` named `{namespace}local`, in document order (fallbacks
/// included, as lxml's `iter` finds them).
pub fn descendants(root: &Element, namespace: &str, local: &str) -> Vec<Arc<Element>> {
    let mut found = Vec::new();
    let mut stack = vec![root.children().iter()];
    while let Some(top) = stack.last_mut() {
        match top.next() {
            Some(Node::Element(e)) => {
                if e.name().is(Some(namespace), local) {
                    found.push(Arc::clone(e));
                }
                stack.push(e.children().iter());
            }
            Some(_) => {}
            None => {
                stack.pop();
            }
        }
    }
    found
}

/// An element's text as lxml's `.text` reads it: the text before its first child that is
/// not text.
pub fn leading_text(element: &Element) -> String {
    element.children().iter().map_while(Node::as_text).collect()
}
