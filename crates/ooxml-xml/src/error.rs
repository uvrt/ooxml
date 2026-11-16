//! Owns: [`XmlError`], every way parsing, patching or diffing a tree can fail.
//! Must not: carry other crates' errors.

use thiserror::Error;

/// What went wrong. Offsets are byte offsets into the part as decoded to UTF-8 (the input's
/// own offsets for a UTF-8 part).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum XmlError {
    /// The part is not well-formed XML (as libxml2 would refuse it).
    #[error("not well-formed at byte {offset}: {message}")]
    Syntax {
        /// Where.
        offset: usize,
        /// What.
        message: &'static str,
    },
    /// A namespace prefix is used without a declaration in scope.
    #[error("namespace prefix {prefix:?} is not defined (byte {offset})")]
    UndefinedPrefix {
        /// The prefix.
        prefix: String,
        /// Where.
        offset: usize,
    },
    /// An entity is referenced that is neither predefined nor allowed to stay a reference.
    #[error("entity {name:?} is not defined (byte {offset})")]
    UndefinedEntity {
        /// The entity's name.
        name: String,
        /// Where.
        offset: usize,
    },
    /// The bytes are not valid in the part's encoding, or the encoding is not supported.
    #[error("encoding: {0}")]
    Encoding(String),
    /// A limit on untrusted input was exceeded.
    #[error("limit exceeded: {what} (limit {limit})")]
    Limit {
        /// Which limit.
        what: &'static str,
        /// Its value.
        limit: usize,
    },
    /// Well-formed XML this crate refuses rather than approximates (an internal DTD subset,
    /// an entity reference in an attribute value).
    #[error("not supported at byte {offset}: {what}")]
    Unsupported {
        /// What.
        what: &'static str,
        /// Where.
        offset: usize,
    },
}

/// Why a patch could not be rebased or applied.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PatchError {
    /// The patch's path does not lead to an element in this tree.
    #[error("no element at path {0:?}")]
    NoElement(Vec<usize>),
    /// The children the patch removes are not at its position in this tree (another change
    /// replaced them).
    #[error("the removed nodes are not at {parent:?}[{start}]")]
    Stale {
        /// The parent's path.
        parent: Vec<usize>,
        /// The position.
        start: usize,
    },
    /// Two patches change overlapping nodes, so one cannot be moved past the other.
    #[error("the patches overlap at {0:?}")]
    Conflict(Vec<usize>),
    /// A patch at the document level replaces anything but one root element.
    #[error("a document-level patch must replace the root element with one element")]
    BadRoot,
}
