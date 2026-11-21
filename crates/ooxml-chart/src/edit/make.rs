//! Owns: [`Make`], the elements a chart edit makes, named as ooxml-edit's `make` names them:
//! in the chart namespace with the chart part's own prefix, and in DrawingML's with the
//! prefix the chart part's root declares for it (declared on the made element where the
//! root declares none).
//!
//! Must not: put an element anywhere.

use super::owned::Owned;
use ooxml_xml::Element;

/// DrawingML's namespace.
pub(crate) const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
/// The chart namespace.
pub(crate) const C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";

/// How a chart part's new elements are named.
#[derive(Debug, Clone)]
pub(crate) struct Make {
    c: Option<String>,
    a: Option<String>,
}

impl Make {
    /// The names `root` (a chart part's `c:chartSpace`) declares.
    pub(crate) fn of(root: &Element) -> Make {
        let declared = |uri: &str| {
            root.namespaces()
                .iter()
                .find(|d| &*d.uri == uri)
                .map(|d| d.prefix.as_deref().unwrap_or("").to_owned())
        };
        let c = if root.name().namespace() == Some(C) {
            Some(root.name().prefix().unwrap_or("").to_owned())
        } else {
            declared(C)
        };
        Make { c, a: declared(A) }
    }

    fn prefix(given: Option<&String>, fallback: &'static str) -> Option<String> {
        match given {
            Some(p) if p.is_empty() => None,
            Some(p) => Some(p.clone()),
            None => Some(fallback.to_owned()),
        }
    }

    /// A new chart element.
    pub(crate) fn c(&self, local: &str) -> Owned {
        Owned::named(Make::prefix(self.c.as_ref(), "c").as_deref(), local, Some(C))
    }

    /// A new DrawingML element.
    pub(crate) fn a(&self, local: &str) -> Owned {
        Owned::named(Make::prefix(self.a.as_ref(), "a").as_deref(), local, Some(A))
    }

    /// `element`, the top of what is made, with DrawingML's prefix declared on it where the
    /// root declares none.
    pub(crate) fn top(&self, mut element: Owned) -> Owned {
        if self.a.is_none() {
            element.declare("a", A);
        }
        element
    }
}
