//! Owns: shared strings ([`Atom`]) and qualified names ([`QName`]): a name as written
//! (`w:p`) with the namespace its prefix resolved to where it was parsed.
//!
//! Must not: know any OOXML name or namespace.

use std::fmt;
use std::sync::Arc;

/// An immutable shared string. The parser shares equal short values (most attribute values
/// are short and repeated, like `w:val="24"`), so equal atoms are often the same allocation.
pub type Atom = Arc<str>;

/// A qualified name as written, plus the namespace URI its prefix resolved to.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct QName {
    qualified: Atom,
    /// The colon's byte index in `qualified`, or 0 when there is no prefix (a prefix is
    /// never empty, so 0 is free).
    colon: u32,
    namespace: Option<Atom>,
}

/// A shared [`QName`]; elements and attributes hold names this way.
pub type Name = Arc<QName>;

impl QName {
    /// A name from its parts: `prefix:local` (or `local`) in `namespace`.
    pub fn new(prefix: Option<&str>, local: &str, namespace: Option<&str>) -> QName {
        let (qualified, colon) = match prefix {
            Some(p) if !p.is_empty() => (format!("{p}:{local}"), u32::try_from(p.len()).unwrap_or(0)),
            _ => (local.to_owned(), 0),
        };
        QName { qualified: qualified.into(), colon, namespace: namespace.map(Atom::from) }
    }

    /// A name from its qualified form as written and its resolved namespace (the parser's
    /// constructor; `qualified` was checked to hold at most one colon, not at either end).
    pub(crate) fn from_parts(qualified: Atom, namespace: Option<Atom>) -> QName {
        let colon = qualified.find(':').and_then(|i| u32::try_from(i).ok()).unwrap_or(0);
        QName { qualified, colon, namespace }
    }

    /// The name as written: `w:p`.
    pub fn qualified(&self) -> &str {
        &self.qualified
    }

    /// The prefix, if any: `w`.
    pub fn prefix(&self) -> Option<&str> {
        match self.colon {
            0 => None,
            c => self.qualified.get(..c as usize),
        }
    }

    /// The local part: `p`.
    pub fn local(&self) -> &str {
        match self.colon {
            0 => &self.qualified,
            c => self.qualified.get(c as usize + 1..).unwrap_or_default(),
        }
    }

    /// The namespace URI, if the name is in one.
    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    /// Whether this is `{namespace}local`.
    pub fn is(&self, namespace: Option<&str>, local: &str) -> bool {
        self.local() == local && self.namespace() == namespace
    }
}

impl fmt::Debug for QName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.namespace {
            Some(ns) => write!(f, "{{{ns}}}{}", self.qualified),
            None => write!(f, "{}", self.qualified),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts() {
        let n = QName::new(Some("w"), "p", Some("urn:w"));
        assert_eq!((n.qualified(), n.prefix(), n.local(), n.namespace()), ("w:p", Some("w"), "p", Some("urn:w")));
        assert!(n.is(Some("urn:w"), "p"));
        let m = QName::new(None, "a", None);
        assert_eq!((m.qualified(), m.prefix(), m.local()), ("a", None, "a"));
        assert_eq!(QName::from_parts("x:y".into(), None).prefix(), Some("x"));
    }
}
