//! Owns: what an edit of the parts behind a graphic frame makes ([`GraphicEdit`]): the parts
//! it reads beside the graphic's own, the parts it puts whole, as trees or as bytes (a chart
//! part, its embedded workbook: [`PartWrite`]), what becomes of the chart's entry in the
//! document's records ([`RecordChange`]), and its refusal ([`GraphicError`]) -- as data, so
//! each format's host applies it through its own store command.
//!
//! Must not: know which format's document holds the graphic, or write a part itself.

use ooxml_opc::{OpcError, Package};
use ooxml_xml::Tree;
use std::sync::Arc;

/// A part an edit writes whole.
#[derive(Debug, Clone)]
pub enum PartWrite {
    /// As a tree (an XML part).
    Tree(String, Tree),
    /// As bytes (an embedded package).
    Bytes(String, Arc<[u8]>),
}

/// What becomes of the chart's entry in the document's records of where placed values came
/// from (the `prov:source` keyed by the drawing's id), beside the record the chart part
/// keeps itself (one of the edit's [`PartWrite`]s).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RecordChange {
    /// It stays as it is.
    #[default]
    Kept,
    /// It holds this record (written as a record is), made where there is none.
    Stored(String),
    /// It goes.
    Dropped,
}

/// What a [`GraphicEdit`] made: the parts it read beside the graphic's own, those it
/// writes, what becomes of the chart's record, and its outcome.
#[derive(Debug, Clone)]
pub struct GraphicParts<T> {
    /// The parts read beside the graphic's part (an embedded workbook).
    pub read: Vec<String>,
    /// The parts written.
    pub writes: Vec<PartWrite>,
    /// The chart's entry in the document's records.
    pub record: RecordChange,
    /// The outcome.
    pub out: T,
}

/// A graphic edit refused: nothing is written. Each host words it as its tools word the
/// same refusal of any other edit.
#[derive(Debug, Clone, thiserror::Error)]
pub enum GraphicError {
    /// An argument the edit cannot take as given (`invalid_arguments`, at no field).
    #[error("{0}")]
    Argument(String),
    /// What the edit would have to do is not ported yet.
    #[error("not supported yet: {0}")]
    Unsupported(String),
    /// A part the graphic names that the package lacks: a bug, not the caller's input.
    #[error("no part {0}")]
    NoPart(String),
    /// The package could not be read.
    #[error(transparent)]
    Package(#[from] OpcError),
}

/// An edit of the parts behind a chart, made on the version it is prepared on.
pub trait GraphicEdit {
    /// Its outcome.
    type Out;

    /// The edit of the chart whose part is `part` in `package`.
    ///
    /// # Errors
    /// The edit's refusal: nothing is written.
    fn edit(&self, package: &Package, part: &str) -> Result<GraphicParts<Self::Out>, GraphicError>;
}
