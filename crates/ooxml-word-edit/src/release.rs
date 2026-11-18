//! Owns: dropping the relationships an edit stopped using and the parts nothing uses any
//! more, as ooxml-edit does (`OpcPackage.release`, `reap`, `remove_part`, `_prune_defaults`,
//! `_remove_keeping_layout`) -- a relationship goes only when no attribute of its part
//! holds its id; a relationships part left empty goes; a part goes with what only it leads
//! to once nothing outside that set relates to it, with its relationships part and its
//! content-type override, and an extension's `Default` once no part has that extension.
//!
//! Package operations on one version: [`Released`] says which parts changed so the caller
//! writes them through the sink, or applies them where a change is committed.
//!
//! Must not: decide what an edit removed (the edit names the ids), or touch a story part.

use crate::doc::ModelError;
use crate::names::{RELS, TYPES};
use crate::work::{Id, Work};
use ooxml_opc::name::{normalize, rels_for};
use ooxml_opc::{Package, CONTENT_TYPES_PART};
use ooxml_xml::Element;
use std::collections::{BTreeSet, HashMap, HashSet};

/// A package after a release: the version, and the parts it changed or removed.
#[derive(Debug, Clone)]
pub(crate) struct Released {
    pub(crate) package: Package,
    /// Parts whose content changed (relationships, content types).
    pub(crate) changed: BTreeSet<String>,
    /// Parts removed.
    pub(crate) removed: BTreeSet<String>,
}

/// Every attribute value in `root` (ooxml-edit's `referenced_values`: what a relationship
/// id is proved unused against).
fn values(root: &Element, out: &mut HashSet<String>) {
    for attribute in root.attributes() {
        out.insert(attribute.value.to_string());
    }
    for child in root.child_elements() {
        values(child, out);
    }
}

/// `package` with `part`'s relationships `ids` dropped where nothing in the part uses them,
/// then the parts they led to reaped.
///
/// # Errors
/// When a part is not XML.
pub(crate) fn release(package: &Package, part: &str, ids: &[String]) -> Result<Released, ModelError> {
    let mut out = Released {
        package: package.clone(),
        changed: BTreeSet::new(),
        removed: BTreeSet::new(),
    };
    let part = normalize(part);
    let relationships = package.relationships(&part)?;
    let mut wanted: Vec<&String> = Vec::new();
    for id in ids {
        if relationships.get(id).is_some() && !wanted.contains(&id) {
            wanted.push(id);
        }
    }
    if wanted.is_empty() {
        return Ok(out);
    }
    let mut used = HashSet::new();
    if let Some(tree) = package.tree(&part)? {
        values(tree.root(), &mut used);
    }
    let rels = rels_for(&part);
    let mut targets = Vec::new();
    let mut gone: Vec<&str> = Vec::new();
    for id in wanted {
        if used.contains(id.as_str()) {
            continue;
        }
        gone.push(id);
        if let Some(target) = relationships.get(id).and_then(|r| r.target_part.clone()) {
            targets.push(target);
        }
    }
    if gone.is_empty() {
        return Ok(out);
    }
    let tree = package.tree(&rels)?.ok_or_else(|| ModelError::NoPart(rels.clone()))?;
    let mut work = Work::of(tree.root(), Vec::new());
    let root = work.root();
    for id in gone {
        let node = work
            .children(root)
            .iter()
            .copied()
            .find(|c| work.is(*c, RELS, "Relationship") && work.get(*c, None, "Id") == Some(id));
        if let Some(node) = node {
            remove_keeping_layout(&mut work, node);
        }
    }
    let left = work.children(root).iter().any(|c| work.is(*c, RELS, "Relationship"));
    if left {
        let built = work.build().ok_or_else(|| ModelError::NoPart(rels.clone()))?;
        out.package = out.package.with_tree(&rels, tree.with_root(built))?;
        out.changed.insert(rels);
    } else {
        // Word writes no empty relationships part
        let current = out.package.clone();
        out.package = remove_part(&current, &rels, &mut out)?;
    }
    reap(&mut out, targets)?;
    Ok(out)
}

/// The parts relationships come from (a relationships part's owner; the package's).
fn owners(package: &Package) -> Vec<String> {
    package.part_names().iter().filter_map(|p| owner_of(p)).collect()
}

/// The part a relationships part belongs to (`""` for the package's).
fn owner_of(path: &str) -> Option<String> {
    if path == "_rels/.rels" {
        return Some(String::new());
    }
    let (directory, name) = path.rsplit_once('/')?;
    let file = name.strip_suffix(".rels")?;
    let parent = directory.strip_suffix("_rels")?;
    let parent = parent.strip_suffix('/').unwrap_or(parent);
    Some(normalize(&if parent.is_empty() {
        file.to_owned()
    } else {
        format!("{parent}/{file}")
    }))
}

/// Remove the `candidates` -- and what only they lead to -- that nothing else uses.
fn reap(out: &mut Released, candidates: Vec<String>) -> Result<(), ModelError> {
    let package = out.package.clone();
    let mut doomed: BTreeSet<String> = BTreeSet::new();
    let mut pending: Vec<String> = candidates.iter().map(|c| normalize(c)).collect();
    while let Some(path) = pending.pop() {
        if doomed.contains(&path) || !package.has_part(&path) || path == CONTENT_TYPES_PART {
            continue;
        }
        if let Ok(rels) = package.relationships(&path) {
            pending.extend(rels.iter().filter_map(|r| r.target_part.clone()));
        }
        doomed.insert(path);
    }
    if doomed.is_empty() {
        return Ok(());
    }
    let mut incoming: HashMap<String, HashSet<String>> = doomed.iter().map(|p| (p.clone(), HashSet::new())).collect();
    for owner in owners(&package) {
        let Ok(rels) = package.relationships(&owner) else {
            continue;
        };
        for rel in rels.iter() {
            if let Some(sources) = rel.target_part.as_ref().and_then(|t| incoming.get_mut(t)) {
                sources.insert(owner.clone());
            }
        }
    }
    loop {
        let keep: Vec<String> = doomed
            .iter()
            .filter(|p| {
                incoming
                    .get(*p)
                    .is_some_and(|s| s.iter().any(|source| !doomed.contains(source)))
            })
            .cloned()
            .collect();
        if keep.is_empty() {
            break;
        }
        for path in keep {
            doomed.remove(&path);
        }
    }
    let mut extensions = BTreeSet::new();
    for path in &doomed {
        if out.package.has_part(path) {
            let current = out.package.clone();
            out.package = remove_part(&current, path, out)?;
        }
        if let Some((_, extension)) = path.rsplit_once('.') {
            extensions.insert(extension.to_lowercase());
        }
    }
    prune_defaults(out, &extensions)
}

/// `package` without `path`, its relationships part and its content-type override.
fn remove_part(package: &Package, path: &str, out: &mut Released) -> Result<Package, ModelError> {
    let mut package = package.clone();
    let rels = rels_for(path);
    if rels != path && package.has_part(&rels) {
        package = remove_part(&package, &rels, out)?;
    }
    package = package.without(path)?;
    out.removed.insert(path.to_owned());
    out.changed.remove(path);
    edit_types(&package, out, |work, root| {
        work.children(root)
            .iter()
            .copied()
            .filter(|c| {
                work.is(*c, TYPES, "Override") && work.get(*c, None, "PartName").is_some_and(|p| normalize(p) == path)
            })
            .collect()
    })
}

/// The `Default` of each of `extensions` no part has any more, removed.
fn prune_defaults(out: &mut Released, extensions: &BTreeSet<String>) -> Result<(), ModelError> {
    let in_use: HashSet<String> = out
        .package
        .part_names()
        .iter()
        .filter_map(|p| p.rsplit_once('.').map(|(_, e)| e.to_lowercase()))
        .collect();
    let unused: HashSet<&String> = extensions.iter().filter(|e| !in_use.contains(*e)).collect();
    if unused.is_empty() {
        return Ok(());
    }
    let package = out.package.clone();
    out.package = edit_types(&package, out, |work, root| {
        work.children(root)
            .iter()
            .copied()
            .filter(|c| {
                work.is(*c, TYPES, "Default")
                    && unused.contains(&work.get(*c, None, "Extension").unwrap_or("").to_lowercase())
            })
            .collect()
    })?;
    Ok(())
}

/// The content types without the nodes `pick` chooses (each removed keeping the layout).
fn edit_types(
    package: &Package,
    out: &mut Released,
    pick: impl FnOnce(&Work, Id) -> Vec<Id>,
) -> Result<Package, ModelError> {
    let Some(tree) = package.tree(CONTENT_TYPES_PART)? else {
        return Ok(package.clone());
    };
    let mut work = Work::of(tree.root(), Vec::new());
    let root = work.root();
    let nodes = pick(&work, root);
    if nodes.is_empty() {
        return Ok(package.clone());
    }
    for node in nodes {
        remove_keeping_layout(&mut work, node);
    }
    let built = work
        .build()
        .ok_or_else(|| ModelError::NoPart(CONTENT_TYPES_PART.to_owned()))?;
    out.changed.insert(CONTENT_TYPES_PART.to_owned());
    Ok(package.with_tree(CONTENT_TYPES_PART, tree.with_root(built))?)
}

/// ooxml-edit's `_remove_keeping_layout`: a last child hands its tail (the closing
/// indentation) to the one before; the node goes with its tail (lxml's `remove`).
fn remove_keeping_layout(work: &mut Work, node: Id) {
    if work.next(node).is_none() {
        if let Some(previous) = work.previous(node) {
            let tail = work.tail(node).to_owned();
            work.set_tail(previous, &tail);
        }
    }
    work.set_tail(node, "");
    work.remove(node);
}
