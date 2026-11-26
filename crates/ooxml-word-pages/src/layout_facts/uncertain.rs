//! Owns: `uncertain` (docx-agent's `uncertain`): the pages, in order, on which the layout
//! drew a placeholder of what it cannot lay out, or laid something out by a rule no probe
//! measured -- each placeholder's reason and the block it stands for, each approximation's
//! reason and the block it is of (its path where the version names no block there). A
//! version laid out here never stops part way (a layout that stops is a fallback), so
//! nothing is unknown from any page.
//!
//! Must not: word an entry (the tool layer does) or cap a list.

use super::Facts;
use crate::coverage::Coverage;
use std::collections::BTreeMap;

/// One place drawn as a placeholder, or laid out by a rule no probe measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approximated {
    /// Why (the code after `layout-placeholder:` or `layout-approximate:`).
    pub reason: String,
    /// The block it is of, by id, or its path.
    pub at: Option<String>,
}

/// One page's uncertain places.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Uncertain {
    /// The 1-based page.
    pub page: usize,
    /// Its placeholders, in the order drawn.
    pub placeholders: Vec<Approximated>,
    /// Its approximations, in the order said.
    pub approximations: Vec<Approximated>,
}

/// `uncertain` of `coverage` (the version's), on the pages asked for: its placeholders, then
/// its approximations.
#[must_use]
pub fn uncertain(facts: &Facts<'_>, coverage: &Coverage) -> Vec<Uncertain> {
    let mut pages: BTreeMap<usize, Uncertain> = BTreeMap::new();
    let shown = |page: Option<usize>| page.filter(|p| facts.shown([*p]));
    for entry in &coverage.placeholders {
        let Some(page) = shown(entry.page) else { continue };
        let at = entry.path.as_deref().map(|path| named(facts, entry.key, path));
        on(&mut pages, page).placeholders.push(Approximated {
            reason: entry.reason().to_owned(),
            at,
        });
    }
    for entry in &coverage.approximations {
        let Some(page) = shown(entry.page) else { continue };
        let reason = entry.listed().0.to_owned();
        let at = entry.path.as_deref().map(|path| named(facts, entry.key, path));
        on(&mut pages, page).approximations.push(Approximated { reason, at });
    }
    pages.into_values().collect()
}

/// What an entry about the block `key` at `path` names: the block's id, where it is a body
/// block the version names, else the path.
fn named(facts: &Facts<'_>, key: Option<ooxml_word_model::ids::BlockKey>, path: &str) -> String {
    facts
        .version()
        .id_named(key)
        .map_or_else(|| path.to_owned(), |id| id.to_string())
}

/// Page `page`'s entry of `pages`, made where it has none yet.
fn on(pages: &mut BTreeMap<usize, Uncertain>, page: usize) -> &mut Uncertain {
    pages.entry(page).or_insert_with(|| Uncertain {
        page,
        ..Uncertain::default()
    })
}
