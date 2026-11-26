//! Owns: [`VersionDelta::between`], what one version of a document changed against another
//! read from their readings: every package part whose content the two do not share, the
//! story parts among them compared block by block through their indexes
//! ([`crate::ids::PartIds::delta`]) -- one function for an edit, an undo, a restore or a
//! span of many versions (phase 3 identity design, section 2).
//!
//! Must not: read a part the two versions share, or name a block by text.

use super::{Docx, StoryPart};
use crate::ids::{PartsDelta, StoryChange, StoryDelta, StoryTable, VersionDelta};
use ooxml_opc::Package;
use std::collections::BTreeSet;
use std::sync::Arc;

impl VersionDelta {
    /// What `after` changed against `before` (two versions of one document, in either
    /// order of making), its stories interned in the document's `stories`; a story past the
    /// table's limit is left out of `stories`, its part listed with the other parts.
    #[must_use]
    pub fn between(before: &Docx, after: &Docx, stories: &mut StoryTable) -> VersionDelta {
        let story_of = |docx: &Docx, part: &str| docx.parts.iter().find(|p| p.part == part).map(Arc::clone);
        let mut parts = PartsDelta::default();
        let mut changes = Vec::new();
        for part in PartsDelta::between(&before.package, &after.package).changed {
            let (old, new) = (story_of(before, &part), story_of(after, &part));
            let story = new.as_ref().or(old.as_ref()).and_then(|p| stories.intern(&p.story));
            match story {
                Some(story) if old.is_some() || new.is_some() => {
                    let delta = story_delta(old.as_deref(), new.as_deref(), story);
                    let order = new.as_ref().map_or(usize::MAX, |n| {
                        after.parts.iter().position(|p| Arc::ptr_eq(p, n)).unwrap_or(usize::MAX)
                    });
                    changes.push((order, StoryChange { story, part, delta }));
                }
                _ => parts.changed.push(part),
            }
        }
        changes.sort_by_key(|(order, _)| *order);
        VersionDelta {
            span: None,
            parts,
            stories: changes.into_iter().map(|(_, change)| change).collect(),
        }
    }
}

impl PartsDelta {
    /// Every part of either package whose content the other does not share (put, removed or
    /// replaced), stories' parts among them, sorted by name.
    #[must_use]
    pub fn between(before: &Package, after: &Package) -> PartsDelta {
        let mut names: BTreeSet<String> = before.part_names().into_iter().collect();
        names.extend(after.part_names());
        let differs = |part: &str| match (before.part(part), after.part(part)) {
            (Some(a), Some(b)) => !a.same(b),
            (None, None) => false,
            _ => true,
        };
        PartsDelta {
            changed: names.into_iter().filter(|p| differs(p)).collect(),
        }
    }
}

fn story_delta(old: Option<&StoryPart>, new: Option<&StoryPart>, story: crate::ids::StoryIx) -> StoryDelta {
    match (old, new) {
        (Some(old), Some(new)) if old.story == new.story => old.ids.delta(old.root(), &new.ids, new.root(), story),
        _ => StoryDelta::Whole,
    }
}
