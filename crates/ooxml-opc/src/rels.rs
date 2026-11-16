//! Owns: relationships as typed values read from a part's relationships part, with
//! `ooxml_edit`'s reading rules: `Relationship` elements of the package relationships
//! namespace, in document order; one lacking `Id`, `Type` or `Target` is skipped; an
//! internal target resolved against the source part's directory.
//!
//! Must not: write a relationships part (edits to relationships are commands of a later
//! phase, made as tree patches in the form `ooxml_edit` writes them).

use crate::name::{directory, normalize, rels_for, resolve_target};
use crate::{OpcError, Package};

/// The package relationships namespace.
pub const RELS_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// One relationship.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    /// Its id (`rId1`).
    pub id: String,
    /// Its type URI.
    pub rel_type: String,
    /// Its target as written.
    pub target: String,
    /// Whether `TargetMode="External"`.
    pub external: bool,
    /// An internal target as a part name.
    pub target_part: Option<String>,
}

/// A part's relationships, in document order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Relationships {
    items: Vec<Relationship>,
}

impl Relationships {
    /// All of them, in document order.
    pub fn iter(&self) -> impl Iterator<Item = &Relationship> {
        self.items.iter()
    }

    /// How many.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The relationship `id` (the last of that id, as a dictionary keyed by id keeps it).
    pub fn get(&self, id: &str) -> Option<&Relationship> {
        self.items.iter().rev().find(|r| r.id == id)
    }

    /// The internal targets of relationships of type `rel_type`, in document order.
    pub fn targets_of_type<'a>(&'a self, rel_type: &'a str) -> impl Iterator<Item = &'a str> {
        self.items.iter().filter(move |r| r.rel_type == rel_type).filter_map(|r| r.target_part.as_deref())
    }
}

impl Package {
    /// The relationships of the part `source` (`""` for the package's own).
    pub fn relationships(&self, source: &str) -> Result<Relationships, OpcError> {
        let source = if source.is_empty() { String::new() } else { normalize(source) };
        let Some(tree) = self.tree(&rels_for(&source))? else {
            return Ok(Relationships::default());
        };
        let base = directory(&source);
        let items = tree
            .root()
            .child_elements()
            .filter(|e| e.name().is(Some(RELS_NS), "Relationship"))
            .filter_map(|e| {
                let (id, rel_type, target) = (e.attribute(None, "Id")?, e.attribute(None, "Type")?, e.attribute(None, "Target")?);
                let external = e.attribute(None, "TargetMode") == Some("External");
                Some(Relationship {
                    id: id.to_owned(),
                    rel_type: rel_type.to_owned(),
                    target: target.to_owned(),
                    external,
                    target_part: (!external).then(|| resolve_target(base, target)),
                })
            })
            .collect();
        Ok(Relationships { items })
    }
}
