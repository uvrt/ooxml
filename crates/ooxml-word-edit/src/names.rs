//! Owns: the namespaces this crate writes and reads names in.
//!
//! Must not: hold anything but constants.

pub(crate) use ooxml_word_model::ids::{MC, W, W14, WP, WP14};

/// Office relationships (`r:id`, `r:embed`).
pub(crate) const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// VML's Office namespace (`o:relid`).
pub(crate) const O: &str = "urn:schemas-microsoft-com:office:office";
/// Office math.
pub(crate) const M: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";
/// Word 2023's revision dates (`w16du:dateUtc`).
pub(crate) const W16DU: &str = "http://schemas.microsoft.com/office/word/2023/wordml/word16du";
/// Word 2012 (`w15:people`).
pub(crate) const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";
/// XML's own namespace (`xml:space`).
pub(crate) const XML: &str = "http://www.w3.org/XML/1998/namespace";
/// Package relationships.
pub(crate) const RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
/// Content types.
pub(crate) const TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
