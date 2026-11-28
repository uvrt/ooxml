//! Owns: one version of a Word document as layout reads it ([`WordDocument`]): the body's
//! blocks in the final view, its sections, the header and footer stories, the notes, and
//! the parts every block resolves against (styles, numbering, settings, theme) -- docx2svg's
//! `Document` after `parse_package` -- with its list labels kept incrementally
//! (`crate::labels`). A block's resolved formats are asked of the document's `crate::Model`,
//! which memoises them across versions.
//!
//! Must not: read the package or memoise (`crate::projection` and `crate::Model` do), or
//! lay out.

use crate::labels::{label_after, BlockItems, Item, Lists};
use crate::model::{Block, Paragraph, Section};
use crate::numbering::{Numbering, PictureBullet};
use crate::resolve::{CharacterFormat, Slot};
use crate::settings::Settings;
use crate::styles::StyleSheet;
use crate::theme::{FontScheme, ThemeColours};
use crate::value::{Props, Text};
use ooxml_derive::{Monoid, MonoidTree};
use std::collections::BTreeMap;
use std::sync::Arc;

/// A part's notes (`word/footnotes.xml`, `word/endnotes.xml`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notes {
    /// The notes by id, each its blocks (footnotes: paragraphs only).
    pub notes: BTreeMap<i64, Arc<[Block]>>,
    /// The separators by type (`separator`, `continuationSeparator`, `continuationNotice`).
    pub separators: BTreeMap<Text, Arc<[Block]>>,
    /// The separators' ids by type.
    pub separator_ids: BTreeMap<Text, i64>,
}

/// The parts the blocks resolve against.
#[derive(Debug, Clone)]
pub struct Context {
    /// `word/styles.xml`, where the package has one.
    pub styles: Option<Arc<StyleSheet>>,
    /// The numbering part (empty where there is none).
    pub numbering: Arc<Numbering>,
    /// The settings (defaults where there is no part).
    pub settings: Arc<Settings>,
    /// The theme's fonts, where the package has a theme with a font scheme.
    pub font_scheme: Option<Arc<FontScheme>>,
    /// The theme's colours and the settings' mapping.
    pub colours: Arc<ThemeColours>,
}

impl Context {
    /// Whether `other` is this very context: each part's reading the same one (a version
    /// whose formatting parts are unchanged resolves against the context it was read in).
    #[must_use]
    pub fn same(&self, other: &Context) -> bool {
        let styles = match (&self.styles, &other.styles) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        let fonts = match (&self.font_scheme, &other.font_scheme) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        styles
            && fonts
            && Arc::ptr_eq(&self.numbering, &other.numbering)
            && Arc::ptr_eq(&self.settings, &other.settings)
            && Arc::ptr_eq(&self.colours, &other.colours)
    }
}

/// One version of a Word document, as layout reads it.
#[derive(Debug, Clone)]
pub struct WordDocument {
    /// The main part's name.
    pub main_part: Text,
    /// What the blocks resolve against.
    pub context: Context,
    /// The body's blocks in the final view.
    pub body: Arc<[Block]>,
    /// The body's sections.
    pub sections: Arc<[Section]>,
    /// Header and footer stories by relationship id.
    pub stories: BTreeMap<Text, Arc<[Block]>>,
    /// Their parts by relationship id.
    pub story_parts: BTreeMap<Text, Text>,
    /// The footnotes.
    pub footnotes: Notes,
    /// The endnotes.
    pub endnotes: Notes,
    /// `w:numPicBullet` by id.
    pub picture_bullets: BTreeMap<i64, PictureBullet>,
    /// The keyword of every field drawn as Word cached it, in the body and its stories.
    pub cached_fields: Arc<[Text]>,
    /// What the reader met and did not model, as stable codes.
    pub warnings: Arc<[Text]>,
    /// The body's list items per block, with their composed counts.
    pub(crate) labels: MonoidTree<BlockItems>,
    /// Its story parts' indexes: what its blocks' keys are named by at the edge
    /// (`data-docx-path`, ids).
    pub names: Arc<crate::docx::VersionNames>,
}

/// A paragraph's resolved formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedParagraph {
    /// Its own properties (`w:pPr` through the cascade).
    pub paragraph: Arc<Props>,
    /// Its mark's run properties.
    pub mark: Arc<Props>,
    /// Each run's properties.
    pub runs: Arc<[Arc<Props>]>,
}

impl WordDocument {
    /// The body's top-level paragraphs, which sections index.
    pub fn paragraphs(&self) -> impl Iterator<Item = &Arc<Paragraph>> {
        self.body.iter().filter_map(|b| match b {
            Block::Paragraph(p) => Some(p),
            Block::Table(_) => None,
        })
    }

    /// The format a run's characters of `slot` are measured in.
    pub fn character_format(&self, resolved: &Props, slot: Slot) -> CharacterFormat {
        crate::resolve::character_format(
            resolved,
            slot,
            self.context.font_scheme.as_deref(),
            &self.context.settings,
        )
    }

    /// The list labels of the body's paragraphs, in document order (cells included), each
    /// `None` where the paragraph is no list item or its level draws none: what the layout
    /// counts with one counter for the body.
    pub fn body_labels(&self) -> Vec<Option<String>> {
        let mut out = Vec::new();
        let mut prefix = Lists::empty();
        for block in self.labels.iter() {
            for (item, lists) in block.items.iter() {
                if let Some(l) = lists {
                    prefix = prefix.combine(l);
                }
                out.push(match item {
                    Some(item) if lists.is_some() => label_after(&self.context.numbering, &prefix, *item),
                    _ => None,
                });
            }
        }
        out
    }

    /// The body's list labels as [`WordDocument::body_labels`] gives them, taken from
    /// `old_labels` (those of `old`, a version before over the same numbering) wherever
    /// they are the same: the blocks before the first changed one, and from the first of
    /// the unchanged blocks after it where the lists' counts stand where they stood;
    /// counted only between.
    pub fn body_labels_from(&self, old: &WordDocument, old_labels: &[Option<String>]) -> Vec<Option<String>> {
        let same = |a: Option<&BlockItems>, b: Option<&BlockItems>| match (a, b) {
            (Some(a), Some(b)) => Arc::ptr_eq(&a.items, &b.items),
            _ => false,
        };
        let (n_new, n_old) = (self.labels.len(), old.labels.len());
        let prefix = (0..n_new.min(n_old))
            .take_while(|&k| same(self.labels.get(k), old.labels.get(k)))
            .count();
        let room = n_new.min(n_old) - prefix;
        let suffix = (1..=room)
            .take_while(|&j| same(self.labels.get(n_new - j), old.labels.get(n_old - j)))
            .count();
        let count = |tree: &MonoidTree<BlockItems>, from: usize, to: usize| -> usize {
            (from..to).filter_map(|k| tree.get(k)).map(|b| b.items.len()).sum()
        };
        let head = count(&self.labels, 0, prefix);
        let mut out: Vec<Option<String>> = old_labels.get(..head).map(<[_]>::to_vec).unwrap_or_default();
        if out.len() != head {
            return self.body_labels();
        }
        let Ok(mut lists) = self.labels.prefix(prefix) else {
            return self.body_labels();
        };
        let tail_start = n_new - suffix;
        for k in prefix..n_new {
            if k == tail_start {
                let old_k = n_old - suffix;
                if old.labels.prefix(old_k).ok().as_ref() == Some(&lists) {
                    let old_head = count(&old.labels, 0, old_k);
                    if let Some(rest) = old_labels.get(old_head..) {
                        out.extend_from_slice(rest);
                        return out;
                    }
                }
            }
            let Some(block) = self.labels.get(k) else { break };
            for (item, block_lists) in block.items.iter() {
                if let Some(l) = block_lists {
                    lists = lists.combine(l);
                }
                out.push(match item {
                    Some(item) if block_lists.is_some() => label_after(&self.context.numbering, &lists, *item),
                    _ => None,
                });
            }
        }
        out
    }

    /// The label of the `k`-th paragraph (document order, cells included) of top-level
    /// block `block`, from the counts before the block (O(log n)).
    pub fn label(&self, block: usize, k: usize) -> Option<String> {
        let mut prefix = self.labels.prefix(block).ok()?;
        let items = self.labels.get(block)?;
        for (_, lists) in items.items.iter().take(k + 1) {
            if let Some(l) = lists {
                prefix = prefix.combine(l);
            }
        }
        let (item, lists) = items.items.get(k)?;
        lists.as_ref()?;
        label_after(&self.context.numbering, &prefix, (*item)?)
    }
}

/// A paragraph's list item as the labels count it: its resolved `numPr`.
pub(crate) fn item_of(resolved: &Props) -> Option<Item> {
    let num_id = resolved
        .get("numPr.numId")
        .and_then(crate::value::Value::int)
        .filter(|n| *n != 0)?;
    let ilvl = resolved
        .get("numPr.ilvl")
        .and_then(crate::value::Value::int)
        .unwrap_or(0);
    Some(Item { num_id, ilvl })
}
