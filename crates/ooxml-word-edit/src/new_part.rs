//! Owns: adding a part to a package as ooxml-edit adds one for docx-agent (`add_part` with
//! an `Override`, `add_relationship`): the part's tree, its content type declared at the
//! end of `[Content_Types].xml`, a relationship from its source with the first free
//! `rIdN`, each new element indented like its siblings; and the empty roots docx-agent
//! makes such parts with (`<w15:people .../>`, `<w15:commentsEx .../>`, `<w:styles/>`,
//! `<w:numbering/>`).
//!
//! Must not: decide which part a document needs, or touch a story part.

use crate::doc::ModelError;
use crate::names::{MC, RELS, TYPES};
use crate::work::{Id, Work};
use ooxml_opc::name::{directory, rels_for};
use ooxml_opc::{Package, CONTENT_TYPES_PART};
use ooxml_xml::{Attribute, Element, NamespaceDecl, QName, Tree};
use std::sync::Arc;

/// `<prefix:local xmlns:prefix=uri xmlns:mc=... mc:Ignorable="prefix"/>`.
pub(crate) fn ignorable_root(prefix: &str, local: &str, uri: &str) -> Element {
    let ns = |prefix: &str, uri: &str| NamespaceDecl {
        prefix: Some(Arc::from(prefix)),
        uri: Arc::from(uri),
    };
    Element::new(
        Arc::new(QName::new(Some(prefix), local, Some(uri))),
        vec![ns(prefix, uri), ns("mc", MC)],
        vec![Attribute {
            name: Arc::new(QName::new(Some("mc"), "Ignorable", Some(MC))),
            value: Arc::from(prefix),
        }],
        Vec::new(),
    )
}

/// `<w:local xmlns:w=...>`, empty: the root docx-agent makes a styles or numbering part
/// with.
pub(crate) fn word_root(local: &str) -> Element {
    Element::new(
        Arc::new(QName::new(Some("w"), local, Some(crate::names::W))),
        vec![NamespaceDecl {
            prefix: Some(Arc::from("w")),
            uri: Arc::from(crate::names::W),
        }],
        Vec::new(),
        Vec::new(),
    )
}

/// `package` with `part` added: its tree `root`, its content type `content_type`, a
/// relationship of `rel_type` from `main`.
///
/// # Errors
/// When the content types or relationships are not XML.
pub(crate) fn add_part(
    package: &Package,
    main: &str,
    part: &str,
    root: Element,
    content_type: &str,
    rel_type: &str,
) -> Result<Package, ModelError> {
    let made = package.with_new_tree(part, Tree::from_root(Arc::new(root)));
    let made = declare_override(&made, part, content_type)?;
    add_relationship(&made, main, part, rel_type)
}

/// `default` when the package has no part of that name, else `template` with the lowest
/// number free.
pub(crate) fn unused_name(package: &Package, default: &str, template: &str) -> String {
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
pub(crate) fn match_tail(work: &mut Work, node: Id) {
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

/// `part` of `content_type` declared as ooxml-edit's `add_part` with `override=True`
/// declares it: nothing when its extension's `Default` gives that type already, else an
/// `Override`. `None`: nothing to declare.
///
/// # Errors
/// When the content types are not XML.
pub(crate) fn declare_type_override(
    package: &Package,
    part: &str,
    content_type: &str,
) -> Result<Option<Package>, ModelError> {
    if package.content_types()?.of(part) == Some(content_type) {
        return Ok(None);
    }
    declare_override(package, part, content_type).map(Some)
}

/// An `Override` declaring `part` of `content_type`, at the end of `[Content_Types].xml`.
fn declare_override(package: &Package, part: &str, content_type: &str) -> Result<Package, ModelError> {
    edit_part(package, CONTENT_TYPES_PART, |work| {
        let root = work.root();
        let node = work.new_element(work.element_name(root, TYPES, "Override", ""));
        work.append(root, node);
        for (local, value) in [
            ("PartName", format!("/{part}")),
            ("ContentType", content_type.to_owned()),
        ] {
            work.set(node, Arc::new(QName::new(None, local, None)), &value);
        }
        match_tail(work, node);
    })
}

/// A relationship from `source` to `target` of `rel_type`, `rIdN` the first number from
/// one more than the relationships it has.
fn add_relationship(package: &Package, source: &str, target: &str, rel_type: &str) -> Result<Package, ModelError> {
    let relative = relative_to(target, directory(source));
    Ok(relate(package, source, rel_type, &relative, false)?.0)
}

/// A relationship of `rel_type` from `source` to the part `target`, as ooxml-edit's
/// `add_relationship` makes one: an existing one of that type to that part reused. Returns
/// the package made (`None`: none was added) and the relationship's id.
///
/// # Errors
/// When the relationships part is not XML.
pub(crate) fn add_internal(
    package: &Package,
    source: &str,
    target: &str,
    rel_type: &str,
) -> Result<(Option<Package>, String), ModelError> {
    let existing = package
        .relationships(source)?
        .iter()
        .find(|r| r.rel_type == rel_type && !r.external && r.target_part.as_deref() == Some(target))
        .map(|r| r.id.clone());
    match existing {
        Some(id) => Ok((None, id)),
        None => {
            let relative = relative_to(target, directory(source));
            relate(package, source, rel_type, &relative, false).map(|(p, id)| (Some(p), id))
        }
    }
}

/// `part` of `content_type` declared as ooxml-edit's `add_part` declares a new part's type
/// ([`crate::content_types::declare`]). `None`: nothing to declare.
///
/// # Errors
/// When the content types are not XML.
pub(crate) fn declare_type(package: &Package, part: &str, content_type: &str) -> Result<Option<Package>, ModelError> {
    crate::content_types::declare(package, part, content_type)
}

/// ooxml-edit's `find_part_with_bytes`: a part below `folder` holding exactly `data`, the
/// first in archive order; a part held as a tree (changed by an edit) is not one (sizes are
/// compared before any bytes are read).
///
/// # Errors
/// When a part cannot be read.
pub(crate) fn part_with_bytes(package: &Package, data: &[u8], folder: &str) -> Result<Option<String>, ModelError> {
    let prefix = format!("{}/", ooxml_opc::name::normalize(folder).trim_end_matches('/'));
    let size = u64::try_from(data.len()).ok();
    for name in package.part_names() {
        if !name.starts_with(&prefix) || package.size(&name) != size {
            continue;
        }
        if package.read(&name)?.is_some_and(|bytes| *bytes == *data) {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

/// An external relationship from `source` to the address `target` of `rel_type`, as
/// ooxml-edit's `add_external_relationship` adds one: an existing one of that type and
/// address reused. Returns the package made (`None`: none was added) and the
/// relationship's id.
///
/// # Errors
/// When the relationships part is not XML.
pub(crate) fn add_external(
    package: &Package,
    source: &str,
    rel_type: &str,
    target: &str,
) -> Result<(Option<Package>, String), ModelError> {
    let existing = package
        .relationships(source)?
        .iter()
        .find(|r| r.rel_type == rel_type && r.external && r.target == target)
        .map(|r| r.id.clone());
    match existing {
        Some(id) => Ok((None, id)),
        None => relate(package, source, rel_type, target, true).map(|(p, id)| (Some(p), id)),
    }
}

/// A new relationship written into `source`'s relationships part (made when missing).
fn relate(
    package: &Package,
    source: &str,
    rel_type: &str,
    target: &str,
    external: bool,
) -> Result<(Package, String), ModelError> {
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
    let id = format!("rId{index}");
    let mut attributes = vec![
        ("Id", id.clone()),
        ("Type", rel_type.to_owned()),
        ("Target", target.to_owned()),
    ];
    if external {
        attributes.push(("TargetMode", "External".to_owned()));
    }
    let package = edit_part(&package, &rels, |work| {
        let root = work.root();
        let node = work.new_element(work.element_name(root, RELS, "Relationship", ""));
        work.append(root, node);
        for (local, value) in &attributes {
            work.set(node, Arc::new(QName::new(None, local, None)), value);
        }
        match_tail(work, node);
    })?;
    Ok((package, id))
}

/// `posixpath.relpath(target, base)` for a target below or beside `base`.
pub(crate) fn relative_to(target: &str, base: &str) -> String {
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
pub(crate) fn edit_part(package: &Package, part: &str, edit: impl FnOnce(&mut Work)) -> Result<Package, ModelError> {
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
