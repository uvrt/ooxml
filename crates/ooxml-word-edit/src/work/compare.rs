//! Owns: comparing two elements of a [`Work`] as canonical serialisation compares them.
//!
//! Must not: change the work.

use super::{Id, Kind, Work};
use ooxml_xml::Attribute;

impl Work {
    /// Whether two elements are the same XML as canonical serialisation compares them
    /// (lxml's c14n): names and attributes by namespace (attributes in any order), text,
    /// and children in order with their tails; prefixes do not matter.
    pub(crate) fn same_xml(&self, a: Id, b: Id) -> bool {
        let (Some(x), Some(y)) = (self.slot(a), self.slot(b)) else {
            return false;
        };
        match (&x.kind, &y.kind) {
            (
                Kind::Element {
                    name: n1,
                    attributes: a1,
                    text: t1,
                    ..
                },
                Kind::Element {
                    name: n2,
                    attributes: a2,
                    text: t2,
                    ..
                },
            ) => {
                let key = |attrs: &[Attribute]| {
                    let mut v: Vec<(Option<String>, String, String)> = attrs
                        .iter()
                        .map(|t| {
                            (
                                t.name.namespace().map(str::to_owned),
                                t.name.local().to_owned(),
                                t.value.to_string(),
                            )
                        })
                        .collect();
                    v.sort();
                    v
                };
                n1.namespace() == n2.namespace()
                    && n1.local() == n2.local()
                    && t1 == t2
                    && key(a1) == key(a2)
                    && x.children.len() == y.children.len()
                    && x.children
                        .iter()
                        .zip(&y.children)
                        .all(|(c, d)| self.tail(*c) == self.tail(*d) && self.same_xml(*c, *d))
            }
            (Kind::Comment(c), Kind::Comment(d)) | (Kind::Entity(c), Kind::Entity(d)) => c == d,
            (Kind::Pi(c), Kind::Pi(d)) => c == d,
            _ => false,
        }
    }
}
