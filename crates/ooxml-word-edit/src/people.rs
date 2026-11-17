//! Owns: listing a revision's author in `people.xml`, as Word lists everyone who made a
//! revision (docx-agent's `_ensure_person`, measured there) -- the part made when the
//! document has none, with its relationship from the main part and its content type, in
//! the forms ooxml-edit writes them (`add_part`, `add_relationship`, `declare_content_type`).
//!
//! Must not: write anything but through the sink.

use crate::doc::Key;
use crate::names::{RELS, TYPES, W15};
use crate::sink::EditSink;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_opc::name::{directory, rels_for};
use ooxml_opc::CONTENT_TYPES_PART;
use ooxml_xml::{Attribute, Element, NamespaceDecl, QName, Tree};
use std::sync::Arc;

/// The people part's relationship type.
const REL_PEOPLE: &str = "http://schemas.microsoft.com/office/2011/relationships/people";
/// The people part's content type.
const PEOPLE_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.people+xml";
const MC: &str = crate::names::MC;

/// List `author` in the people part, making the part when there is none.
///
/// # Errors
/// When a part is not XML.
pub(crate) fn ensure_person(sink: &mut EditSink<'_, '_>, author: &str) -> Result<()> {
    let docx = sink.docx()?;
    let main = docx.main.clone();
    let package = sink.doc().package().clone();
    let found = package
        .relationships(&main)?
        .targets_of_type(REL_PEOPLE)
        .next()
        .map(str::to_owned)
        .filter(|p| package.has_part(p));
    let part = match found {
        Some(part) => part,
        None => {
            let part = unused_name(&package, "word/people.xml", "word/people{n}.xml");
            sink.read(Key::Part(Arc::from(part.as_str())));
            sink.put(&part, Tree::from_root(Arc::new(empty_people())))?;
            declare_override(sink, &part)?;
            add_relationship(sink, &main, &part)?;
            part
        }
    };
    sink.read(Key::Part(Arc::from(part.as_str())));
    let tree = sink.doc().tree(&part)?.ok_or_else(|| EditError::NoPart(part.clone()))?;
    let listed = tree
        .root()
        .child_elements()
        .any(|p| p.name().is(Some(W15), "person") && p.attribute(Some(W15), "author") == Some(author));
    if listed {
        return Ok(());
    }
    let mut work = Work::of(tree.root(), Vec::new());
    let root = work.root();
    let person = work.new_element(work.element_name(root, W15, "person", "w15"));
    work.append(root, person);
    let name = work.qname(W15, "author", "w15");
    work.set(person, name, author);
    let presence = work.new_element(work.element_name(person, W15, "presenceInfo", "w15"));
    work.append(person, presence);
    for (local, value) in [("providerId", "None"), ("userId", author)] {
        let name = work.qname(W15, local, "w15");
        work.set(presence, name, value);
    }
    let root = work.build().ok_or_else(|| EditError::NoPart(part.clone()))?;
    sink.put(&part, tree.with_root(root))
}

/// `<w15:people xmlns:w15=... xmlns:mc=... mc:Ignorable="w15"/>`, as docx-agent makes it.
fn empty_people() -> Element {
    let ns = |prefix: &str, uri: &str| NamespaceDecl {
        prefix: Some(Arc::from(prefix)),
        uri: Arc::from(uri),
    };
    Element::new(
        Arc::new(QName::new(Some("w15"), "people", Some(W15))),
        vec![ns("w15", W15), ns("mc", MC)],
        vec![Attribute {
            name: Arc::new(QName::new(Some("mc"), "Ignorable", Some(MC))),
            value: Arc::from("w15"),
        }],
        Vec::new(),
    )
}

/// `default` when the package has no part of that name, else `template` with the lowest
/// number free.
fn unused_name(package: &ooxml_opc::Package, default: &str, template: &str) -> String {
    if !package.has_part(default) {
        return default.to_owned();
    }
    (1..)
        .map(|n: u32| template.replace("{n}", &n.to_string()))
        .find(|name| !package.has_part(name))
        .unwrap_or_else(|| default.to_owned())
}

/// Indent a new last child like its siblings, for parts written pretty-printed
/// (ooxml-edit's `_match_tail`).
fn match_tail(work: &mut Work, node: Id) {
    let Some(previous) = work.previous(node) else { return };
    let tail = work.tail(previous).to_owned();
    if tail.is_empty() || !tail.trim().is_empty() {
        return;
    }
    if work.next(node).is_none() {
        work.set_tail(node, &tail);
        if let Some(before) = work.previous(previous) {
            let between = work.tail(before).to_owned();
            if !between.is_empty() && between.trim().is_empty() {
                work.set_tail(previous, &between);
            }
        }
    } else {
        work.set_tail(node, &tail);
    }
}

/// An `Override` declaring `part` as a people part, at the end of `[Content_Types].xml`.
fn declare_override(sink: &mut EditSink<'_, '_>, part: &str) -> Result<()> {
    edit_part(sink, CONTENT_TYPES_PART, |work| {
        let root = work.root();
        let node = work.new_element(work.element_name(root, TYPES, "Override", ""));
        work.append(root, node);
        for (local, value) in [
            ("PartName", format!("/{part}")),
            ("ContentType", PEOPLE_TYPE.to_owned()),
        ] {
            work.set(node, Arc::new(QName::new(None, local, None)), &value);
        }
        match_tail(work, node);
    })
}

/// A relationship from `source` to `target` of the people type, `rIdN` the first number
/// from one more than the relationships it has.
fn add_relationship(sink: &mut EditSink<'_, '_>, source: &str, target: &str) -> Result<()> {
    let rels = rels_for(source);
    if !sink.doc().package().has_part(&rels) {
        let root = Element::new(
            Arc::new(QName::new(None, "Relationships", Some(RELS))),
            vec![NamespaceDecl {
                prefix: None,
                uri: Arc::from(RELS),
            }],
            Vec::new(),
            Vec::new(),
        );
        // `.rels` has its `Default` content type in every Word package: nothing declared
        sink.put(&rels, Tree::from_root(Arc::new(root)))?;
    }
    let used: Vec<String> = sink
        .doc()
        .package()
        .relationships(source)?
        .iter()
        .map(|r| r.id.clone())
        .collect();
    let mut index = used.len() + 1;
    while used.contains(&format!("rId{index}")) {
        index += 1;
    }
    let base = directory(source);
    let relative = relative_to(target, base);
    edit_part(sink, &rels, |work| {
        let root = work.root();
        let node = work.new_element(work.element_name(root, RELS, "Relationship", ""));
        work.append(root, node);
        for (local, value) in [
            ("Id", format!("rId{index}")),
            ("Type", REL_PEOPLE.to_owned()),
            ("Target", relative.clone()),
        ] {
            work.set(node, Arc::new(QName::new(None, local, None)), &value);
        }
        match_tail(work, node);
    })
}

/// `posixpath.relpath(target, base)` for a target below or beside `base`.
fn relative_to(target: &str, base: &str) -> String {
    let t: Vec<&str> = target.split('/').filter(|s| !s.is_empty()).collect();
    let b: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    let common = t.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut out: Vec<&str> = vec![".."; b.len() - common];
    out.extend(t.iter().skip(common));
    if out.is_empty() {
        ".".to_owned()
    } else {
        out.join("/")
    }
}

/// Edit a small part (relationships, content types) in an editable copy and put it back.
fn edit_part(sink: &mut EditSink<'_, '_>, part: &str, edit: impl FnOnce(&mut Work)) -> Result<()> {
    sink.read(Key::Part(Arc::from(part)));
    let tree = sink
        .doc()
        .tree(part)?
        .ok_or_else(|| EditError::NoPart(part.to_owned()))?;
    let mut work = Work::of(tree.root(), Vec::new());
    edit(&mut work);
    let root = work.build().ok_or_else(|| EditError::NoPart(part.to_owned()))?;
    sink.put(part, tree.with_root(root))
}

#[cfg(test)]
mod tests {
    use super::relative_to;

    #[test]
    fn relative_targets() {
        assert_eq!(relative_to("word/people.xml", "word"), "people.xml");
        assert_eq!(relative_to("customXml/item1.xml", "word"), "../customXml/item1.xml");
        assert_eq!(relative_to("word/people.xml", ""), "word/people.xml");
    }
}
