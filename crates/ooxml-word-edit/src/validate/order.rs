//! Owns: [`Orders`], the child sequences validation checks elements against and edits insert
//! children by, and the prefixes they are named with -- `ooxml_schema::orders`, generated
//! from the published XML schemas (with its overlay; `cargo xtask codegen`).
//!
//! Must not: hold an order of its own, or know what a sequence means.

use ooxml_schema::orders::{NAMESPACES, ORDERS};
use ooxml_xml::Element;
use std::collections::HashMap;

/// Child sequences by parent, as ranks.
#[derive(Debug)]
pub(crate) struct Orders {
    /// Namespace to prefix: the first prefix listed for it.
    prefixes: HashMap<String, String>,
    /// Parent (`w:body`) to its children's ranks.
    ranks: HashMap<String, HashMap<String, usize>>,
}

impl Orders {
    /// The generated orders.
    pub(crate) fn load() -> Orders {
        let mut prefixes = HashMap::new();
        for (prefix, uri) in NAMESPACES {
            prefixes
                .entry((*uri).to_owned())
                .or_insert_with(|| (*prefix).to_owned());
        }
        let mut ranks = HashMap::new();
        for (parent, steps) in ORDERS {
            let mut by_name = HashMap::new();
            for (rank, step) in steps.iter().enumerate() {
                for name in *step {
                    by_name.insert((*name).to_owned(), rank);
                }
            }
            ranks.insert((*parent).to_owned(), by_name);
        }
        Orders { prefixes, ranks }
    }

    /// An element's name as the table names it: the listed prefix of its namespace,
    /// else its local name (ooxml-edit's `prefixed_name`).
    pub(crate) fn prefixed(&self, element: &Element) -> String {
        let name = element.name();
        self.prefixed_name(name.namespace(), name.local())
    }

    /// [`Orders::prefixed`] of a name given by its namespace and local name.
    pub(crate) fn prefixed_name(&self, namespace: Option<&str>, local: &str) -> String {
        match namespace.and_then(|ns| self.prefixes.get(ns)) {
            Some(prefix) => format!("{prefix}:{local}"),
            None => local.to_owned(),
        }
    }

    /// An element's rank in `ranks`: by its name as written when its prefix is the listed
    /// one (no string made), else by the listed name.
    pub(crate) fn rank(&self, ranks: &HashMap<String, usize>, element: &Element) -> Option<usize> {
        let name = element.name();
        let listed = name
            .namespace()
            .and_then(|ns| self.prefixes.get(ns))
            .map(String::as_str);
        if listed.is_some() && listed == name.prefix() {
            return ranks.get(name.qualified()).copied();
        }
        ranks.get(self.prefixed(element).as_str()).copied()
    }

    /// The ranks of `parent`'s children, when the table has a sequence for it.
    pub(crate) fn ranks(&self, parent: &str) -> Option<&HashMap<String, usize>> {
        self.ranks.get(parent)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// A parent's children rank as its generated steps say; the first prefix listed for a
    /// namespace names it.
    #[test]
    fn ranks_and_prefixes_come_from_the_generated_orders() {
        let orders = Orders::load();
        let p_pr = orders.ranks("w:pPr").unwrap();
        assert!(p_pr["w:pStyle"] < p_pr["w:keepNext"]);
        let body = orders.ranks("w:body").unwrap();
        assert_eq!(body["w:p"], body["w:tbl"]);
        assert_eq!(
            orders.prefixed_name(
                Some("http://schemas.openxmlformats.org/wordprocessingml/2006/main"),
                "p"
            ),
            "w:p"
        );
        assert!(orders.ranks("w:unlisted").is_none());
    }
}
