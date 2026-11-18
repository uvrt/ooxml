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
use crate::names::W15;
use crate::new_part::{add_part, unused_name};
use crate::work::{Id, Work};
use ooxml_opc::name::rels_for;
use ooxml_opc::{Package, CONTENT_TYPES_PART};
use ooxml_xml::Element;
use std::sync::Arc;

/// The people part's relationship type.
const REL_PEOPLE: &str = "http://schemas.microsoft.com/office/2011/relationships/people";
/// The people part's content type.
const PEOPLE_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.people+xml";

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
            let made = add_part(package, main, &part, empty_people(), PEOPLE_TYPE, REL_PEOPLE)?;
            (made, part)
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
    crate::new_part::ignorable_root("w15", "people", W15)
}
