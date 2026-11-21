//! Owns: `uncertain` (docx-agent's `uncertain`): the pages, in order, on which the layout
//! laid something out by a rule no probe measured -- each place's reason and the block it
//! is of (its path where the version names no block there). A version laid out here draws
//! no placeholder and never stops part way (a layout that stops is a fallback), so its
//! entries hold approximations only, and nothing is unknown from any page.
//!
//! Must not: word an entry (the tool layer does) or cap a list.

use super::Facts;
use crate::warnings::Approximation;
use std::collections::BTreeMap;

/// One place laid out by a rule no probe measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approximated {
    /// Why (the approximation's code after `layout-approximate:`).
    pub reason: String,
    /// The block it is of, by id, or its path.
    pub at: Option<String>,
}

/// One page's uncertain places.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uncertain {
    /// The 1-based page.
    pub page: usize,
    /// Its approximations, in the order said.
    pub approximations: Vec<Approximated>,
}

/// `uncertain` of `approximations` (the version's coverage's), on the pages asked for.
#[must_use]
pub fn uncertain(facts: &Facts<'_>, approximations: &[Approximation]) -> Vec<Uncertain> {
    let mut pages: BTreeMap<usize, Vec<Approximated>> = BTreeMap::new();
    for entry in approximations {
        let Some(page) = entry.page.filter(|p| facts.shown([*p])) else {
            continue;
        };
        let reason = entry.code.split_once(':').map_or("", |(_, r)| r).to_owned();
        let at = entry.path.as_deref().map(|path| {
            path.starts_with("w:body")
                .then(|| facts.version().id_of(path))
                .flatten()
                .unwrap_or(path)
                .to_owned()
        });
        pages.entry(page).or_default().push(Approximated { reason, at });
    }
    pages
        .into_iter()
        .map(|(page, approximations)| Uncertain { page, approximations })
        .collect()
}
