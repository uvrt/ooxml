//! A Word package's parts as docx-agent finds them (`WordPackage`, `OpcPackage`): the main
//! part, the parts related to it by type, its story parts, and the elements of a part that
//! Word shows (not under `mc:Fallback`).
//!
//! Owns: those rules, read from a version of an OPC package; relationships read as Python
//! keys them (a dictionary by id: the first place of an id, its last relationship).
//!
//! Must not: change a package, cache across versions, or know a tool's reply. (It moved here
//! from `ooxml-tools-word` in phase P1: the ids, the addresses and the reads use it.)

use ooxml_opc::{OpcError, Package, Relationship};
use ooxml_xml::{Element, Node};
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

/// The relationships of `source` as a dictionary by id keeps them: each id once, at the
/// place it first appears, with the last relationship of that id.
pub fn relationships(package: &Package, source: &str) -> Result<Vec<Relationship>, OpcError> {
    let mut out: Vec<Relationship> = Vec::new();
    for rel in package.relationships(source)?.iter() {
        match out.iter_mut().find(|r| r.id == rel.id) {
            Some(slot) => *slot = rel.clone(),
            None => out.push(rel.clone()),
        }
    }
    Ok(out)
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
/// those inside an `mc:Fallback` (a fallback repeats its choice's content and is not what
/// Word shows).
pub fn live_elements(root: &Element, namespace: &str, local: &str) -> Vec<Arc<Element>> {
    let mut found = Vec::new();
    let mut stack = vec![root.children().iter()];
    while let Some(top) = stack.last_mut() {
        match top.next() {
            Some(Node::Element(e)) => {
                if e.name().is(Some(MC), "Fallback") {
                    continue;
                }
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
