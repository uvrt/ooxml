//! Owns: keeping `people.xml` listing who made the revisions and comments, as docx-agent
//! does after a tracked edit (`Stamp.finish`: `_ensure_person`, then `_tidy_people`) --
//! the author listed once anything was recorded, the part made when the document has
//! none (with its relationship from the main part and its content type, in the forms
//! ooxml-edit writes them); an author with nothing left in use dropped, and the part with
//! its relationship once nobody is left (`_drop_comment_part`, `release`, `reap`).
//!
//! These are package operations applied where a change is committed
//! ([`crate::Change::People`]): the people part is merged at commit, so two writers
//! listing themselves are no conflict and the result is the serial one.
//!
//! Must not: decide who is in use (the version's tally says), or touch a story part.

use crate::doc::ModelError;
use crate::names::{RELS, TYPES, W15};
use crate::work::{Id, Work};
use ooxml_opc::name::{directory, rels_for};
use ooxml_opc::{Package, CONTENT_TYPES_PART};
use ooxml_xml::{Attribute, Element, NamespaceDecl, QName, Tree};
use std::sync::Arc;

/// The people part's relationship type.
const REL_PEOPLE: &str = "http://schemas.microsoft.com/office/2011/relationships/people";
/// The people part's content type.
const PEOPLE_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.people+xml";
const MC: &str = crate::names::MC;

/// The people part the main part relates to, when the package has it.
fn people_part(package: &Package, main: &str) -> Result<Option<String>, ModelError> {
    Ok(package
        .relationships(main)?
        .targets_of_type(REL_PEOPLE)
        .next()
        .map(str::to_owned)
        .filter(|p| package.has_part(p)))
}

/// The parts listing or dropping a person may write: the people part (the one there, or
/// the one that would be made), the content types and the main part's relationships.
///
/// # Errors
/// When the main part's relationships are not XML.
pub(crate) fn parts_written(package: &Package, main: &str) -> Result<Vec<Arc<str>>, ModelError> {
    let part = match people_part(package, main)? {
        Some(part) => part,
        None => unused_name(package, "word/people.xml", "word/people{n}.xml"),
    };
    Ok(vec![
        Arc::from(part.as_str()),
        Arc::from(CONTENT_TYPES_PART),
        Arc::from(rels_for(main).as_str()),
    ])
}

fn is_person(e: &Element, author: &str) -> bool {
    e.name().is(Some(W15), "person") && e.attribute(Some(W15), "author") == Some(author)
}

/// `package` with `author` listed in the people part, the part made when there is none.
///
/// # Errors
/// When a part is not XML.
pub(crate) fn with_person(package: &Package, main: &str, author: &str) -> Result<Package, ModelError> {
    let (package, part) = match people_part(package, main)? {
        Some(part) => (package.clone(), part),
        None => {
            let part = unused_name(package, "word/people.xml", "word/people{n}.xml");
            let made = package.with_new_tree(&part, Tree::from_root(Arc::new(empty_people())));
            let made = declare_override(&made, &part)?;
            (add_relationship(&made, main, &part)?, part)
        }
    };
    let tree = package.tree(&part)?.ok_or_else(|| ModelError::NoPart(part.clone()))?;
    if tree.root().child_elements().any(|p| is_person(p, author)) {
        return Ok(package);
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
    let root = work.build().ok_or_else(|| ModelError::NoPart(part.clone()))?;
    Ok(package.with_tree(&part, tree.with_root(root))?)
}

/// `package` without `author` in the people part (one with nothing left in use), and
/// without the part and its relationship once nobody is listed (`_tidy_people`).
///
/// # Errors
/// When a part is not XML.
pub(crate) fn without_person(package: &Package, main: &str, author: &str) -> Result<Package, ModelError> {
    let Some(part) = people_part(package, main)? else {
        return Ok(package.clone());
    };
    let tree = package.tree(&part)?.ok_or_else(|| ModelError::NoPart(part.clone()))?;
    let mut work = Work::of(tree.root(), Vec::new());
    let root = work.root();
    let gone: Vec<Id> = work
        .children(root)
        .iter()
        .copied()
        .filter(|c| work.is(*c, W15, "person") && work.get(*c, Some(W15), "author") == Some(author))
        .collect();
    if gone.is_empty() {
        return Ok(package.clone());
    }
    for person in gone {
        work.remove(person);
    }
    let root = work.build().ok_or_else(|| ModelError::NoPart(part.clone()))?;
    let package = package.with_tree(&part, tree.with_root(Arc::clone(&root)))?;
    if root.child_elements().any(|p| p.name().is(Some(W15), "person")) {
        return Ok(package);
    }
    drop_part(&package, main, &part)
}

/// The people part dropped (`_drop_comment_part`): its relationship from the main part
/// released, and the part reaped with it.
fn drop_part(package: &Package, main: &str, part: &str) -> Result<Package, ModelError> {
    let found = package
        .relationships(main)?
        .iter()
        .find(|r| r.target_part.as_deref() == Some(part))
        .map(|r| r.id.clone());
    let Some(id) = found else { return Ok(package.clone()) };
    Ok(crate::release::release(package, main, &[id])?.package)
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
fn unused_name(package: &Package, default: &str, template: &str) -> String {
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
fn declare_override(package: &Package, part: &str) -> Result<Package, ModelError> {
    edit_part(package, CONTENT_TYPES_PART, |work| {
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
fn add_relationship(package: &Package, source: &str, target: &str) -> Result<Package, ModelError> {
    let rels = rels_for(source);
    let mut package = package.clone();
    if !package.has_part(&rels) {
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
        package = package.with_new_tree(&rels, Tree::from_root(Arc::new(root)));
    }
    let used: Vec<String> = package.relationships(source)?.iter().map(|r| r.id.clone()).collect();
    let mut index = used.len() + 1;
    while used.contains(&format!("rId{index}")) {
        index += 1;
    }
    let relative = relative_to(target, directory(source));
    edit_part(&package, &rels, |work| {
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

/// Edit a small part (relationships, content types) in an editable copy.
fn edit_part(package: &Package, part: &str, edit: impl FnOnce(&mut Work)) -> Result<Package, ModelError> {
    let tree = package.tree(part)?.ok_or_else(|| ModelError::NoPart(part.to_owned()))?;
    let mut work = Work::of(tree.root(), Vec::new());
    edit(&mut work);
    let root = work.build().ok_or_else(|| ModelError::NoPart(part.to_owned()))?;
    Ok(package.with_tree(part, tree.with_root(root))?)
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
