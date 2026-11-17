//! Owns: [`OpcError`].
//! Must not: hide which part an error is about.

use ooxml_xml::XmlError;
use ooxml_zip::ZipError;
use thiserror::Error;

/// What went wrong opening, reading or saving a package.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OpcError {
    /// The archive itself.
    #[error(transparent)]
    Zip(#[from] ZipError),
    /// A part's XML.
    #[error("{part}: {error}")]
    Xml {
        /// The part.
        part: String,
        /// What its parser said.
        error: XmlError,
    },
    /// No part of that name.
    #[error("no part {0:?}")]
    NoPart(String),
    /// A part of that name exists already.
    #[error("part {0:?} exists already")]
    PartExists(String),
    /// Two entries name one part (OPC part names are unique; equivalent names are refused).
    #[error("two entries name the part {0:?}")]
    DuplicatePart(String),
}
