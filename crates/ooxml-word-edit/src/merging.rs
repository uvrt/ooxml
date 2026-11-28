//! Owns: the package's bookkeeping parts merged where another editor's version lands
//! (co-authoring refresh, phase 3 CF2c-2): a relationships part as the union by id of theirs
//! and what was added here since their base, an id of ours that theirs gives another
//! target renamed (and the references to it in our blocks of the source part with it); the
//! content types as the union by part name and extension; a part both added since the
//! base under one name, ours moved to a name of its own (its relationships' targets and its
//! override with it). Theirs wins wherever both name the same entry.
//!
//! Must not: decide what arrives (`crate::arrival` does), or drop an entry of ours added
//! since the base: what cannot be kept under its own id is renamed.

use ooxml_opc::name::{directory, resolve_target};
use ooxml_opc::{Package, CONTENT_TYPES_PART};
use ooxml_xml::{Attribute, Element, Head, Node, QName, Tree};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use crate::doc::ModelError;

/// Relationships.
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// The part a relationships part is of (`word/document.xml` of
/// `word/_rels/document.xml.rels`); `None` for a name that is not one.
#[must_use]
pub(crate) fn source_of(rels: &str) -> Option<String> {
    let (dir, file) = rels.rsplit_once("_rels/")?;
    let name = file.strip_suffix(".rels")?;
    Some(format!("{dir}{name}"))
}

/// An element's attribute `local` (no namespace).
fn attribute<'e>(e: &'e Element, local: &str) -> Option<&'e str> {
    e.attribute(None, local)
}

/// `e` with its attribute `local` (no namespace) set to `value`.
fn with_attribute(e: &Element, local: &str, value: &str) -> Element {
    let mut head: Head = (**e.head()).clone();
    let mut attributes = head.attributes.to_vec();
    match attributes.iter_mut().find(|a| a.name.is(None, local)) {
        Some(a) => a.value = Arc::from(value),
        None => attributes.push(Attribute {
            name: Arc::new(QName::new(None, local, None)),
            value: Arc::from(value),
        }),
    }
    head.attributes = attributes.into();
    Element::from_head(Arc::new(head), e.children().clone())
}

/// The ids of a relationships part's entries.
fn ids(tree: Option<&Tree>) -> BTreeSet<String> {
    tree.map(|t| {
        t.root()
            .child_elements()
            .filter_map(|e| attribute(e, "Id").map(str::to_owned))
            .collect()
    })
    .unwrap_or_default()
}

/// The package with the relationships part `rels` the union of `theirs` and the entries the
/// part here added since `base` (by id); an id of ours theirs holds for another entry
/// renamed to the next free `rIdN`, and the references to it in our blocks of the source
/// part (those `from`, their version, does not hold) with it.
///
/// # Errors
/// When a part is not XML.
pub(crate) fn relinked(
    package: &Package,
    rels: &str,
    base: Option<&Tree>,
    theirs: &Tree,
    from: &Package,
) -> Result<Package, ModelError> {
    let ours = package.tree(rels)?;
    let base_ids = ids(base);
    let mut taken = ids(Some(theirs));
    let mut children: Vec<Node> = theirs.root().children().iter().cloned().collect();
    let mut renamed: BTreeMap<String, String> = BTreeMap::new();
    if let Some(ours) = &ours {
        taken.extend(ids(Some(ours)));
        let theirs_entries: Vec<&Arc<Element>> = theirs.root().child_elements().collect();
        for e in ours.root().child_elements() {
            let Some(id) = attribute(e, "Id") else { continue };
            if base_ids.contains(id) {
                continue;
            }
            match theirs_entries.iter().find(|t| attribute(t, "Id") == Some(id)) {
                // the very entry: added by both
                Some(t) if ***t == **e => {}
                Some(_) => {
                    let fresh = (1..)
                        .map(|n| format!("rId{n}"))
                        .find(|c| !taken.contains(c))
                        .unwrap_or_default();
                    taken.insert(fresh.clone());
                    renamed.insert(id.to_owned(), fresh.clone());
                    children.push(Node::Element(Arc::new(with_attribute(e, "Id", &fresh))));
                }
                None => children.push(Node::Element(Arc::clone(e))),
            }
        }
    }
    let root = theirs.root().with_children(children.into_iter().collect());
    let mut package = put(package, rels, theirs.with_root(Arc::new(root)));
    if renamed.is_empty() {
        return Ok(package);
    }
    let Some(source) = source_of(rels) else {
        return Ok(package);
    };
    let (Some(tree), theirs_source) = (package.tree(&source)?, from.tree(&source)?) else {
        return Ok(package);
    };
    // their blocks are theirs as their version holds them (shared where the change landed)
    let theirs_blocks: HashSet<*const Element> = theirs_source
        .iter()
        .flat_map(|t| blocks(t.root()))
        .map(|e| Arc::as_ptr(&e))
        .collect();
    let root = renamed_in(tree.root(), &renamed, &theirs_blocks, 0);
    if let Some(root) = root {
        package = package.with_tree(&source, tree.with_root(root))?;
    }
    Ok(package)
}

/// The elements a part's blocks are: the children of its root's first child that has
/// children (a body), or of the root.
fn blocks(root: &Arc<Element>) -> Vec<Arc<Element>> {
    let body = root.child_elements().find(|e| e.child_elements().next().is_some());
    body.unwrap_or(root).child_elements().cloned().collect()
}

/// `e` with every relationship reference (an attribute in the relationships namespace)
/// named in `renamed` renamed, but in the blocks of theirs; `None` where nothing changed.
/// `depth` 0 is the root, 1 its children, 2 a body's blocks.
fn renamed_in(
    e: &Arc<Element>,
    renamed: &BTreeMap<String, String>,
    theirs: &HashSet<*const Element>,
    depth: usize,
) -> Option<Arc<Element>> {
    if depth > 0 && theirs.contains(&Arc::as_ptr(e)) {
        return None;
    }
    let mut changed = false;
    let mut head: Head = (**e.head()).clone();
    let mut attributes = head.attributes.to_vec();
    for a in &mut attributes {
        if a.name.namespace() == Some(R) {
            if let Some(new) = renamed.get(&*a.value) {
                a.value = Arc::from(new.as_str());
                changed = true;
            }
        }
    }
    head.attributes = attributes.into();
    let mut children: Vec<Node> = Vec::new();
    for child in e.children().iter() {
        match child {
            Node::Element(c) => match renamed_in(c, renamed, theirs, depth + 1) {
                Some(new) => {
                    changed = true;
                    children.push(Node::Element(new));
                }
                None => children.push(child.clone()),
            },
            other => children.push(other.clone()),
        }
    }
    changed.then(|| Arc::new(Element::from_head(Arc::new(head), children.into_iter().collect())))
}

/// The key of a content types entry: an override's part name, a default's extension.
fn type_key(e: &Element) -> Option<String> {
    if let Some(part) = attribute(e, "PartName") {
        return Some(format!("o:{}", part.to_ascii_lowercase()));
    }
    attribute(e, "Extension").map(|x| format!("d:{}", x.to_ascii_lowercase()))
}

/// The package with the content types the union of `theirs` and the entries added here
/// since `base` (by part name and extension), theirs winning where both name one.
///
/// # Errors
/// When the content types are not XML.
pub(crate) fn typed(package: &Package, base: Option<&Tree>, theirs: &Tree) -> Result<Package, ModelError> {
    let keys = |t: &Tree| -> BTreeSet<String> { t.root().child_elements().filter_map(|e| type_key(e)).collect() };
    let base_keys = base.map(keys).unwrap_or_default();
    let theirs_keys = keys(theirs);
    let mut children: Vec<Node> = theirs.root().children().iter().cloned().collect();
    if let Some(ours) = package.tree(CONTENT_TYPES_PART)? {
        for e in ours.root().child_elements() {
            let Some(key) = type_key(e) else { continue };
            if !base_keys.contains(&key) && !theirs_keys.contains(&key) {
                children.push(Node::Element(Arc::clone(e)));
            }
        }
    }
    let root = theirs.root().with_children(children.into_iter().collect());
    Ok(put(package, CONTENT_TYPES_PART, theirs.with_root(Arc::new(root))))
}

/// The package with the part `from` moved to `to` (a name of its own): its content, the
/// relationships of every part that target it, and its content type's override.
///
/// # Errors
/// When a part is not XML.
pub(crate) fn moved(package: &Package, from: &str, to: &str) -> Result<Package, ModelError> {
    let Some(bytes) = package.read(from)? else {
        return Ok(package.clone());
    };
    let mut next = package.without(from)?.with_bytes(to, bytes);
    for rels in package.part_names().into_iter().filter(|n| n.ends_with(".rels")) {
        let Some(source) = source_of(&rels) else { continue };
        let Some(tree) = next.tree(&rels)? else { continue };
        let dir = directory(&source);
        let mut changed = false;
        let children: Vec<Node> = tree
            .root()
            .children()
            .iter()
            .map(|n| match n {
                Node::Element(e)
                    if attribute(e, "TargetMode") != Some("External")
                        && attribute(e, "Target").is_some_and(|t| resolve_target(dir, t) == from) =>
                {
                    changed = true;
                    let target = crate::new_part::relative_to(to, dir);
                    Node::Element(Arc::new(with_attribute(e, "Target", &target)))
                }
                other => other.clone(),
            })
            .collect();
        if changed {
            let root = tree.root().with_children(children.into_iter().collect());
            next = next.with_tree(&rels, tree.with_root(Arc::new(root)))?;
        }
    }
    if let Some(types) = next.tree(CONTENT_TYPES_PART)? {
        let named = format!("/{from}");
        let children: Vec<Node> = types
            .root()
            .children()
            .iter()
            .map(|n| match n {
                Node::Element(e) if attribute(e, "PartName").is_some_and(|p| p.eq_ignore_ascii_case(&named)) => {
                    Node::Element(Arc::new(with_attribute(e, "PartName", &format!("/{to}"))))
                }
                other => other.clone(),
            })
            .collect();
        let root = types.root().with_children(children.into_iter().collect());
        next = next.with_tree(CONTENT_TYPES_PART, types.with_root(Arc::new(root)))?;
    }
    Ok(next)
}

/// `tree` as `part`, put in place (a new part at the end).
fn put(package: &Package, part: &str, tree: Tree) -> Package {
    if package.has_part(part) {
        package
            .with_tree(part, tree.clone())
            .unwrap_or_else(|_| package.with_new_tree(part, tree))
    } else {
        package.with_new_tree(part, tree)
    }
}

/// The relationships parts of `package` with an entry targeting the part `target`.
///
/// # Errors
/// When a relationships part is not XML.
pub(crate) fn targeting(package: &Package, target: &str) -> Result<Vec<String>, ModelError> {
    let mut out = Vec::new();
    for rels in package.part_names().into_iter().filter(|n| n.ends_with(".rels")) {
        let Some(source) = source_of(&rels) else { continue };
        let Some(tree) = package.tree(&rels)? else { continue };
        let dir = directory(&source);
        let targets = tree.root().child_elements().any(|e| {
            attribute(e, "TargetMode") != Some("External")
                && attribute(e, "Target").is_some_and(|t| resolve_target(dir, t) == target)
        });
        if targets {
            out.push(rels);
        }
    }
    Ok(out)
}

/// A name for a part of ours that `name` another's part takes: `name` with the lowest number
/// in place of its stem's trailing digits that neither `ours` nor `theirs` has.
#[must_use]
pub(crate) fn fresh_name(ours: &Package, theirs: &Package, name: &str) -> String {
    let (dir, file) = name.rsplit_once('/').map_or(("", name), |(d, f)| (d, f));
    let (stem, ext) = file.rsplit_once('.').map_or((file, ""), |(s, e)| (s, e));
    let stem = stem.trim_end_matches(|c: char| c.is_ascii_digit());
    let named = |n: u32| {
        let file = if ext.is_empty() {
            format!("{stem}{n}")
        } else {
            format!("{stem}{n}.{ext}")
        };
        if dir.is_empty() {
            file
        } else {
            format!("{dir}/{file}")
        }
    };
    (1..)
        .map(named)
        .find(|candidate| !ours.has_part(candidate) && !theirs.has_part(candidate))
        .unwrap_or_else(|| format!("{name}.ours"))
}
