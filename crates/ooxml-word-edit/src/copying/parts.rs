//! Owns: the relationships and parts a copy brings, as docx-agent's importer brings them
//! (`edit/importing.py`, `_relationships`, `relationship`, `copy_part`,
//! `_rename_relationship`): each relationship a copied element names made again from the
//! place's part -- an address as an external relationship (one of that type and address
//! reused), a part as a relationship to this document's copy of it (one of that type to
//! that part reused); a part copied once per copy under its own name, or its name numbered
//! when taken, its content type declared, its own relationships made again (its references
//! following an id that changed); a picture or other media found by its bytes among the
//! parts of its folder instead of copied again.
//!
//! The parts are built in a package of their own and put in place at the end, new parts in
//! the order they were made (docx-agent's archive order).
//!
//! Must not: place the copy, or change a part this document had but its relationships and
//! content types.

use crate::doc::Key;
use crate::names::{O, R};
use crate::new_part::{add_external, add_internal, declare_type};
use crate::sink::EditSink;
use crate::work::{Id, Work};
use crate::Result;
use ooxml_opc::name::{directory, normalize, rels_for};
use ooxml_opc::{Package, PartVersion, Relationship, CONTENT_TYPES_PART};
use ooxml_xml::Attribute;
use std::collections::HashMap;
use std::sync::Arc;

/// The parts a copy brings, built beside the document's package.
pub(crate) struct Parts {
    /// The source document's package.
    source: Package,
    /// This document's package as the copy found it.
    original: Package,
    /// This document's package with what the copy brought.
    package: Package,
    /// Source part -> this document's copy of it (`part_map`).
    copied: HashMap<String, String>,
}

impl Parts {
    /// Parts copied from `source` into `destination`.
    pub(crate) fn new(source: &Package, destination: &Package) -> Parts {
        Parts {
            source: source.clone(),
            original: destination.clone(),
            package: destination.clone(),
            copied: HashMap::new(),
        }
    }

    /// `_relationships`: every relationship id `elements` (copies from `source_part`, in
    /// `work`) name made again from `dest_part`, each id once per call. Reads the
    /// relationships of `dest_part` and the content types where there is any.
    ///
    /// # Errors
    /// When a part or relationships part is not readable.
    pub(crate) fn bring(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        work: &mut Work,
        elements: &[Id],
        (source_part, dest_part): (&str, &str),
    ) -> Result<()> {
        let mut cache: HashMap<String, String> = HashMap::new();
        for &element in elements {
            for node in work.descendants(element) {
                let named: Vec<Attribute> = work
                    .attributes(node)
                    .iter()
                    .filter(|a| names_relationship(a))
                    .cloned()
                    .collect();
                for attribute in named {
                    if cache.is_empty() {
                        sink.read(Key::Part(Arc::from(normalize(&rels_for(dest_part)))));
                        sink.read(Key::Part(Arc::from(CONTENT_TYPES_PART)));
                    }
                    let value: &str = &attribute.value;
                    let new = match cache.get(value) {
                        Some(new) => new.clone(),
                        None => {
                            let new = self.relationship(source_part, value, dest_part)?;
                            cache.insert(value.to_owned(), new.clone());
                            new
                        }
                    };
                    work.set(node, Arc::clone(&attribute.name), &new);
                }
            }
        }
        Ok(())
    }

    /// Put in place every part the copy made or changed: changed ones where they are, new
    /// ones in the order they were made.
    ///
    /// # Errors
    /// When the change cannot be applied.
    pub(crate) fn finish(self, sink: &mut EditSink<'_, '_>) -> Result<()> {
        for name in self.package.part_names() {
            let Some(version) = self.package.part(&name) else {
                continue;
            };
            if self.original.part(&name).is_some_and(|was| was.same(version)) {
                continue;
            }
            match version {
                PartVersion::Tree(tree) => sink.put(&name, tree.clone())?,
                PartVersion::Bytes(bytes) => sink.put_bytes(&name, Arc::clone(bytes))?,
                PartVersion::Raw(_) => {}
            }
        }
        Ok(())
    }

    /// `relationship`: a relationship from `dest_part` like `source_part`'s `rid` (`rid`
    /// itself where the source has none, or its part is missing).
    fn relationship(&mut self, source_part: &str, rid: &str, dest_part: &str) -> Result<String> {
        let Some(rel) = by_id(&self.source, source_part)?.into_iter().find(|r| r.id == rid) else {
            return Ok(rid.to_owned());
        };
        if rel.external {
            return self.external(dest_part, &rel);
        }
        let Some(target) = rel.target_part.as_deref().filter(|t| self.source.has_part(t)) else {
            return Ok(rid.to_owned());
        };
        let copy = self.copy_part(target)?;
        self.internal(dest_part, &rel.rel_type, &copy)
    }

    fn external(&mut self, part: &str, rel: &Relationship) -> Result<String> {
        let (made, id) = add_external(&self.package, part, &rel.rel_type, &rel.target)?;
        if let Some(made) = made {
            self.package = made;
        }
        Ok(id)
    }

    fn internal(&mut self, part: &str, rel_type: &str, target: &str) -> Result<String> {
        let (made, id) = add_internal(&self.package, part, &normalize(target), rel_type)?;
        if let Some(made) = made {
            self.package = made;
        }
        Ok(id)
    }

    /// `copy_part`: this document's copy of the source's part `source_part` -- media found
    /// by its bytes, anything else copied once under its name (numbered when taken), its
    /// content type declared, its relationships made again.
    fn copy_part(&mut self, source_part: &str) -> Result<String> {
        if let Some(copy) = self.copied.get(source_part) {
            return Ok(copy.clone());
        }
        let data = self.source.read(source_part)?.unwrap_or_else(|| Arc::from(Vec::new()));
        let content_type = self.source.content_types()?.of(source_part).map(str::to_owned);
        let folder = directory(source_part);
        if folder.rsplit('/').next() == Some("media") {
            if let Some(existing) = self.with_bytes(&data, folder)? {
                self.copied.insert(source_part.to_owned(), existing.clone());
                return Ok(existing);
            }
        }
        let target = if self.package.has_part(source_part) {
            let file = source_part.rsplit('/').next().unwrap_or(source_part);
            let (base, extension) = split_extension(file);
            let stem = match base.trim_end_matches(|c: char| c.is_ascii_digit()) {
                "" => base,
                stem => stem,
            };
            let prefix = if folder.is_empty() {
                String::new()
            } else {
                format!("{folder}/")
            };
            (1u32..)
                .map(|n| normalize(&format!("{prefix}{stem}{n}{extension}")))
                .find(|name| !self.package.has_part(name))
                .unwrap_or_else(|| source_part.to_owned())
        } else {
            source_part.to_owned()
        };
        self.package = self.package.with_bytes(&target, data);
        if let Some(content_type) = content_type {
            if let Some(made) = declare_type(&self.package, &target, &content_type)? {
                self.package = made;
            }
        }
        self.copied.insert(source_part.to_owned(), target.clone());
        for rel in by_id(&self.source, source_part)? {
            let new = if rel.external {
                self.external(&target, &rel)?
            } else {
                match rel.target_part.as_deref().filter(|t| self.source.has_part(t)) {
                    Some(part) => {
                        let copy = self.copy_part(part)?;
                        self.internal(&target, &rel.rel_type, &copy)?
                    }
                    None => continue,
                }
            };
            if new != rel.id {
                self.rename(&target, &rel.id, &new)?;
            }
        }
        Ok(target)
    }

    /// `find_part_with_bytes`: a part below `folder` holding exactly `data` (none changed
    /// as a tree), the first in archive order.
    fn with_bytes(&self, data: &[u8], folder: &str) -> Result<Option<String>> {
        let prefix = format!("{}/", normalize(folder).trim_end_matches('/'));
        let size = u64::try_from(data.len()).ok();
        for name in self.package.part_names() {
            if !name.starts_with(&prefix) || self.package.size(&name) != size {
                continue;
            }
            if self.package.read(&name)?.is_some_and(|bytes| *bytes == *data) {
                return Ok(Some(name));
            }
        }
        Ok(None)
    }

    /// `_rename_relationship`: a copied part's relationship `old` got the id `new`: its
    /// references follow (the part then written from its tree). A part that is not XML is
    /// left as it is.
    fn rename(&mut self, part: &str, old: &str, new: &str) -> Result<()> {
        let Ok(Some(tree)) = self.package.tree(part) else {
            return Ok(());
        };
        let mut work = Work::of(tree.root(), Vec::new());
        for node in work.descendants(work.root()) {
            let named: Vec<Attribute> = work
                .attributes(node)
                .iter()
                .filter(|a| names_relationship(a) && &*a.value == old)
                .cloned()
                .collect();
            for attribute in named {
                work.set(node, attribute.name, new);
            }
        }
        if let Some(root) = work.build() {
            self.package = self.package.with_tree(part, tree.with_root(root))?;
        }
        Ok(())
    }
}

/// Whether an attribute holds a relationship id: one in the relationships namespace, or
/// VML's `o:relid`.
fn names_relationship(attribute: &Attribute) -> bool {
    let name = &attribute.name;
    name.namespace() == Some(R) || (name.namespace() == Some(O) && name.local() == "relid")
}

/// A part's relationships as ooxml-edit's dictionary holds them: by id, each in its first
/// place with its last value.
fn by_id(package: &Package, part: &str) -> Result<Vec<Relationship>> {
    let mut out: Vec<Relationship> = Vec::new();
    for rel in package.relationships(part)?.iter() {
        match out.iter_mut().find(|r| r.id == rel.id) {
            Some(found) => *found = rel.clone(),
            None => out.push(rel.clone()),
        }
    }
    Ok(out)
}

/// `posixpath.splitext` of a file name: the stem and the extension with its dot (leading
/// dots belong to the stem).
fn split_extension(file: &str) -> (&str, &str) {
    match file.rfind('.') {
        Some(at) if file[..at].chars().any(|c| c != '.') => file.split_at(at),
        _ => (file, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::split_extension;

    #[test]
    fn extensions_split_as_posixpath() {
        assert_eq!(split_extension("image1.png"), ("image1", ".png"));
        assert_eq!(split_extension("a.b.emf"), ("a.b", ".emf"));
        assert_eq!(split_extension(".rels"), (".rels", ""));
        assert_eq!(split_extension("bin"), ("bin", ""));
    }
}
