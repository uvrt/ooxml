//! Owns: [`DocId`], a document's id in its session (`d1`, `d2`, ...; never given twice),
//! and [`DocKind`], the kinds of document a session opens.
//!
//! Must not: know what a kind's package holds.

use std::fmt;

/// A document's id in its session, as the model writes it (`d1`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocId(u64);

impl DocId {
    /// The `n`th id given (`d<n>`).
    #[must_use]
    pub fn nth(n: u64) -> Self {
        Self(n)
    }

    /// The id written `text` (`d1`), if it is one.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('d')?;
        if digits.is_empty() || digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok().map(Self)
    }
}

impl fmt::Display for DocId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "d{}", self.0)
    }
}

/// The kinds of document a session opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DocKind {
    /// A Word document.
    Docx,
    /// A PowerPoint deck.
    Pptx,
    /// An Excel workbook.
    Xlsx,
}

impl DocKind {
    /// Every kind, in this order.
    pub const ALL: [DocKind; 3] = [DocKind::Docx, DocKind::Pptx, DocKind::Xlsx];

    /// The kind as tools name it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DocKind::Docx => "docx",
            DocKind::Pptx => "pptx",
            DocKind::Xlsx => "xlsx",
        }
    }

    /// The kind named `text`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == text)
    }
}

impl fmt::Display for DocKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_kinds_read_as_written() {
        assert_eq!(DocId::parse("d12"), Some(DocId::nth(12)));
        assert_eq!(DocId::nth(3).to_string(), "d3");
        for text in ["", "d", "d0", "d01", "x1", "d1x"] {
            assert_eq!(DocId::parse(text), None, "{text}");
        }
        assert_eq!(DocKind::parse("docx"), Some(DocKind::Docx));
        assert_eq!(DocKind::parse("doc"), None);
    }
}
