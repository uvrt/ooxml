//! The OPC package (docs/architecture/README.md, section 4, "Packages").
//!
//! Owns: [`Package`], a persistent map from part name to [`PartVersion`] -- raw until read,
//! parsed lazily and once -- with typed [`Relationships`] and [`ContentTypes`], and save:
//! untouched entries raw, touched parts in the write contract's form.
//!
//! Must not: know a format's parts (Word, slides, sheets) or its XML vocabulary beyond
//! OPC's own.
#![forbid(unsafe_code)]

mod content_types;
mod error;
pub mod name;
mod package;
mod rels;

pub use content_types::{ContentTypes, CONTENT_TYPES_NS, CONTENT_TYPES_PART};
pub use error::OpcError;
pub use ooxml_base::Limits;
pub use package::{Package, PartVersion, RawPart};
pub use rels::{Relationship, Relationships, RELS_NS};
