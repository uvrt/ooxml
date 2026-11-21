//! Owns: the parts nothing uses taken out of a package, as ooxml-edit's `drop_orphans`
//! takes them: those no chain of relationships from the package root reaches (a page and
//! its notes that point only at each other count), and those no content type is declared
//! for that no relationship targets (a stray zip entry such as `[trash]/0000.dat`); each with
//! its relationships part and its `Override`, an extension's `Default` once no part has the
//! extension.
//!
//! Must not: take out the content types part, or a part some relationship reaches.

use crate::content_types::remove_keeping_layout;
use crate::doc::ModelError;
use crate::names::TYPES;
use crate::new_part::edit_part;
use crate::Result;
use ooxml_opc::name::{normalize, rels_for};
use ooxml_opc::{Package, CONTENT_TYPES_PART};
use ooxml_word_model::parts::relationships;
use std::collections::BTreeSet;

/// The part a relationships part belongs to (`dir/file` for `dir/_rels/file.rels`, `""`
/// for the package's own); the name itself for any other part (ooxml-edit's `part_for_rels`).
fn owner_of(path: &str) -> String {
    let (directory, name) = path.rsplit_once('/').unwrap_or(("", path));
    match (directory.strip_suffix("_rels"), name.strip_suffix(".rels")) {
        (Some(dir), Some(file)) => {
            let dir = dir.trim_end_matches('/');
            if dir.is_empty() {
                file.to_owned()
            } else {
                format!("{dir}/{file}")
            }
        }
        _ => path.to_owned(),
    }
}

/// The parts nothing uses, in name order (ooxml-edit's `orphaned_parts`).
fn orphaned(package: &Package) -> Result<Vec<String>> {
    let parts: BTreeSet<String> = package.part_names().into_iter().collect();
    let mut reached: BTreeSet<String> = BTreeSet::from([String::new()]);
    let mut pending = vec![String::new()];
    while let Some(source) = pending.pop() {
        for rel in relationships(package, &source).map_err(ModelError::from)? {
            if let Some(target) = rel.target_part {
                if !reached.contains(&target) && parts.contains(&target) {
                    reached.insert(target.clone());
                    pending.push(target);
                }
            }
        }
    }
    let mut found: BTreeSet<String> = parts
        .iter()
        .filter(|p| p.as_str() != CONTENT_TYPES_PART && !reached.contains(*p))
        .filter(|p| {
            let o = owner_of(p);
            !(o != **p && reached.contains(&o))
        })
        .cloned()
        .collect();
    let mut targets = BTreeSet::new();
    for path in &parts {
        let o = owner_of(path);
        if o != *path || path == "_rels/.rels" {
            for rel in relationships(package, &o).map_err(ModelError::from)? {
                targets.extend(rel.target_part);
            }
        }
    }
    let types = package.content_types().map_err(ModelError::from)?;
    for path in &parts {
        if path != CONTENT_TYPES_PART && !targets.contains(path) && owner_of(path) == *path && types.of(path).is_none()
        {
            found.insert(path.clone());
        }
    }
    Ok(found
        .iter()
        .filter(|p| {
            let o = owner_of(p);
            !(o != **p && found.contains(&o))
        })
        .cloned()
        .collect())
}

/// `package` without the parts nothing uses, and `(part, bytes)` for every part taken out,
/// relationships parts included, in name order (ooxml-edit's `drop_orphans`).
///
/// # Errors
/// When the package's relationships or content types are not XML.
pub(super) fn drop_orphans(package: &Package) -> Result<(Package, Vec<(String, u64)>)> {
    let mut out = package.clone();
    let mut removed: Vec<(String, u64)> = Vec::new();
    for path in orphaned(package)? {
        if !out.has_part(&path) {
            continue;
        }
        let rels = rels_for(&path);
        let mut gone = vec![path.clone()];
        if rels != path && out.has_part(&rels) {
            gone.push(rels);
        }
        for name in gone {
            removed.push((name.clone(), size(&out, &name)?));
            out = out.without(&name).map_err(ModelError::from)?;
        }
    }
    if removed.is_empty() {
        return Ok((out, removed));
    }
    if out.has_part(CONTENT_TYPES_PART) {
        out = without_types(&out, &removed)?;
    }
    removed.sort();
    Ok((out, removed))
}

/// The size of a part's bytes (its stored size while untouched).
pub(super) fn size(package: &Package, name: &str) -> Result<u64> {
    if let Some(size) = package.size(name) {
        return Ok(size);
    }
    let bytes = package.read(name).map_err(ModelError::from)?;
    Ok(bytes.map_or(0, |b| b.len() as u64))
}

/// The content types without the `Override` of each part `removed`, and without the
/// `Default` of each of their extensions no part has any more.
fn without_types(package: &Package, removed: &[(String, u64)]) -> Result<Package> {
    let gone: BTreeSet<&str> = removed.iter().map(|(p, _)| p.as_str()).collect();
    let in_use: BTreeSet<String> = package
        .part_names()
        .iter()
        .filter_map(|p| p.rsplit_once('.').map(|(_, e)| e.to_lowercase()))
        .collect();
    let unused: BTreeSet<String> = removed
        .iter()
        .filter_map(|(p, _)| p.rsplit_once('.').map(|(_, e)| e.to_lowercase()))
        .filter(|e| !in_use.contains(e))
        .collect();
    let tree = package
        .tree(CONTENT_TYPES_PART)
        .map_err(ModelError::from)?
        .ok_or_else(|| ModelError::NoPart(CONTENT_TYPES_PART.to_owned()))?;
    let touches = tree.root().child_elements().any(|e| {
        (e.name().is(Some(TYPES), "Override")
            && gone.contains(normalize(e.attribute(None, "PartName").unwrap_or_default()).as_str()))
            || (e.name().is(Some(TYPES), "Default")
                && unused.contains(&e.attribute(None, "Extension").unwrap_or_default().to_lowercase()))
    });
    if !touches {
        return Ok(package.clone());
    }
    Ok(edit_part(package, CONTENT_TYPES_PART, |work| {
        let root = work.root();
        let overrides: Vec<_> = work
            .children(root)
            .iter()
            .copied()
            .filter(|c| {
                work.is(*c, TYPES, "Override")
                    && gone.contains(normalize(work.get(*c, None, "PartName").unwrap_or_default()).as_str())
            })
            .collect();
        overrides.into_iter().for_each(|n| remove_keeping_layout(work, n));
        let defaults: Vec<_> = work
            .children(root)
            .iter()
            .copied()
            .filter(|c| {
                work.is(*c, TYPES, "Default")
                    && unused.contains(&work.get(*c, None, "Extension").unwrap_or_default().to_lowercase())
            })
            .collect();
        defaults.into_iter().for_each(|n| remove_keeping_layout(work, n));
    })?)
}

#[cfg(test)]
mod tests {
    use super::owner_of;

    #[test]
    fn owners_of_relationships_parts() {
        assert_eq!(owner_of("_rels/.rels"), "");
        assert_eq!(owner_of("word/_rels/document.xml.rels"), "word/document.xml");
        assert_eq!(owner_of("word/document.xml"), "word/document.xml");
        assert_eq!(owner_of("[trash]/0000.dat"), "[trash]/0000.dat");
    }
}
