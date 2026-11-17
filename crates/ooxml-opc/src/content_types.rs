//! Owns: content types as typed values read from `[Content_Types].xml`, with
//! `ooxml_edit`'s lookup: the first `Override` whose part name matches wins, else the last
//! `Default` of the part's extension (compared without case).
//!
//! Must not: write `[Content_Types].xml` (declaring a type is a command of a later phase,
//! made as a tree patch in the form `ooxml_edit` writes it).

use crate::name::normalize;
use crate::{OpcError, Package};

/// The content types namespace.
pub const CONTENT_TYPES_NS: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
/// The content types part.
pub const CONTENT_TYPES_PART: &str = "[Content_Types].xml";

/// A package's declared content types, in document order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentTypes {
    /// `(extension, content type)` of each `Default`.
    pub defaults: Vec<(String, String)>,
    /// `(part name, content type)` of each `Override`, the part name normalised.
    pub overrides: Vec<(String, String)>,
}

impl ContentTypes {
    /// The content type of `part`: its `Override`, else its extension's `Default`.
    pub fn of(&self, part: &str) -> Option<&str> {
        let part = normalize(part);
        if let Some((_, t)) = self.overrides.iter().find(|(name, _)| *name == part) {
            return Some(t);
        }
        // `path.rsplit(".", 1)[-1].lower() if "." in path else ""`
        let extension = part.rsplit_once('.').map_or(String::new(), |(_, e)| e.to_lowercase());
        self.defaults
            .iter()
            .rev()
            .find(|(e, _)| e.to_lowercase() == extension)
            .map(|(_, t)| t.as_str())
    }
}

impl Package {
    /// The package's content types (empty when it has no `[Content_Types].xml`).
    pub fn content_types(&self) -> Result<ContentTypes, OpcError> {
        let Some(tree) = self.tree(CONTENT_TYPES_PART)? else {
            return Ok(ContentTypes::default());
        };
        let mut types = ContentTypes::default();
        for e in tree.root().child_elements() {
            let attr = |n| e.attribute(None, n).unwrap_or_default().to_owned();
            if e.name().is(Some(CONTENT_TYPES_NS), "Override") {
                types
                    .overrides
                    .push((normalize(&attr("PartName")), attr("ContentType")));
            } else if e.name().is(Some(CONTENT_TYPES_NS), "Default") {
                types.defaults.push((attr("Extension"), attr("ContentType")));
            }
        }
        Ok(types)
    }
}
