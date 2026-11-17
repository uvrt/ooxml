//! The lossless immutable XML tree (docs/architecture/README.md, section 4 and ADR-003/004).
//!
//! Owns: the green tree ([`Tree`], [`Element`], [`Node`], [`Children`]); the parser
//! ([`parse`]) with libxml2's reading rules and limits; the writer ([`Tree::write`]) in
//! libxml2's form, the contract for every touched part; cursors that know their parents
//! ([`Cursor`]); patches by path checked by node pointer ([`Patch`]); and the structural diff
//! that short-circuits on shared subtrees ([`diff`]); and what generated typed views call to
//! read and build elements by local name ([`view`]).
//!
//! Must not: know any OOXML name or namespace, do I/O, or mutate a node once built.
#![forbid(unsafe_code)]

mod cursor;
mod diff;
mod encoding;
mod error;
mod name;
mod node;
mod parse;
mod patch;
mod rope;
pub mod view;
mod write;

pub use cursor::{ChildCursors, Cursor};
pub use diff::diff;
pub use error::{PatchError, XmlError};
pub use name::{Atom, Name, QName};
pub use node::{Attribute, Doctype, Element, ExternalId, Head, NamespaceDecl, Node, Pi, Tree};
pub use ooxml_base::XmlLimits as Limits;
pub use parse::parse;
pub use patch::{NodePath, Patch};
pub use rope::{Children, Iter, BRANCH_MAX, LEAF_MAX};
