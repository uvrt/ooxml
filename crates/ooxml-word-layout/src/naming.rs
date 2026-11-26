//! Owns: what names an item the layout lays out or paints at the edge ([`ItemName`]): its
//! anchor where a story's index holds its block, else the path it was made with (a text
//! box's item, until RD1 keys text-box blocks; a paragraph or separator the layout makes);
//! and a block's path named through the version's names ([`paragraph_path`],
//! [`table_path`]) for what the layout says of it.
//!
//! Must not: key anything by a name (names are for the edge, ADR-008).

use ooxml_word_model::docx::VersionNames;
use ooxml_word_model::ids::{Anchor, Sub};
use ooxml_word_model::model::{Paragraph, Table};
use ooxml_word_model::value::Text;

/// What names a painted item at the edge.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ItemName {
    /// What a story's index holds: a block, or a run, label, cell or section's place on it.
    Anchored(Anchor),
    /// What no story's index holds, by the path it was made with: a text box's item (until
    /// RD1 keys text-box blocks), a paragraph the layout makes.
    Path(Text),
}

impl ItemName {
    /// `anchor`'s name where there is one, else the path `path` makes (made only then).
    pub fn of(anchor: Option<Anchor>, path: impl FnOnce() -> String) -> ItemName {
        match anchor {
            Some(anchor) => ItemName::Anchored(anchor),
            None => ItemName::Path(Text::from(path().as_str())),
        }
    }

    /// Its anchor, where a story's index holds it.
    #[must_use]
    pub fn anchor(&self) -> Option<Anchor> {
        match self {
            ItemName::Anchored(anchor) => Some(*anchor),
            ItemName::Path(_) => None,
        }
    }

    /// Its path, where no story's index holds it.
    #[must_use]
    pub fn path(&self) -> Option<&str> {
        match self {
            ItemName::Anchored(_) => None,
            ItemName::Path(path) => Some(path),
        }
    }

    /// Its name through the version's names `names`: a block's, label's, cell's or
    /// section's path, or its own path (a run's is named from its paragraph's projection,
    /// [`crate::paint::names::PageNames`]: empty here, as is a block not in the version).
    #[must_use]
    pub fn named(&self, names: &VersionNames) -> String {
        match self {
            ItemName::Anchored(anchor) => names.path(*anchor).unwrap_or_default(),
            ItemName::Path(path) => path.to_string(),
        }
    }
}

/// What names `sub` of `paragraph` (itself, its label or a run): its anchor, else the path
/// made from the paragraph's own (one no story's index holds).
pub fn paragraph_item(paragraph: &Paragraph, sub: Sub) -> ItemName {
    if let Some(block) = paragraph.key {
        return ItemName::Anchored(Anchor { block, sub });
    }
    let path = paragraph.unkeyed_path().map_or("", |p| &**p);
    ItemName::Path(match sub {
        Sub::Label => Text::from(format!("{path}/label").as_str()),
        Sub::Run(k) => match usize::try_from(k)
            .ok()
            .and_then(|k| paragraph.runs.get(k))
            .map(|r| &*r.path)
        {
            Some(run) if !run.is_empty() => Text::from(format!("{path}/{run}").as_str()),
            _ => Text::from(path),
        },
        _ => Text::from(path),
    })
}

/// `paragraph`'s path: named by the version's names `names` where a story's index holds it,
/// else its own (what names it then).
pub fn paragraph_path(names: &VersionNames, paragraph: &Paragraph) -> String {
    match paragraph.key {
        Some(key) => names.path(Anchor::whole(key)).unwrap_or_default(),
        None => paragraph.unkeyed_path().map(ToString::to_string).unwrap_or_default(),
    }
}

/// `table`'s path: named by the version's names `names` where a story's index holds it,
/// else its own.
pub fn table_path(names: &VersionNames, table: &Table) -> String {
    match table.key {
        Some(key) => names.path(Anchor::whole(key)).unwrap_or_default(),
        None => table.unkeyed_path().map(ToString::to_string).unwrap_or_default(),
    }
}

/// What names `table` (itself): its key, else its own path.
pub fn table_item(table: &Table) -> ItemName {
    match table.key {
        Some(key) => ItemName::Anchored(Anchor::whole(key)),
        None => ItemName::Path(table.unkeyed_path().cloned().unwrap_or_default()),
    }
}
