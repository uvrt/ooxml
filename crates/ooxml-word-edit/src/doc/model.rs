//! Owns: how a [`WordDoc`] version takes a change ([`DocModel`]): applying it, its inverse,
//! the keys it writes and merges, rebasing it past another, the per-transaction state
//! (stand-ins drawn, spaces written) begun afresh.
//!
//! Must not: decide a change, or keep anything but the version it makes.

use super::{Change, Key, ModelError, Setup, WordDoc};
use crate::pool::IdsChange;
use ooxml_opc::PartName;
use ooxml_store::{DocModel, First, KeySet};
use ooxml_word_model::ids::VersionDelta;
use ooxml_xml::PatchError;
use std::sync::Arc;

impl DocModel for WordDoc {
    type Key = Key;
    type Change = Change;
    type Error = ModelError;
    type Setup = Setup;
    type Delta = VersionDelta;

    fn delta(before: &Self, after: &Self) -> VersionDelta {
        after.delta_from(before)
    }

    fn apply(&mut self, change: &Change) -> Result<(), ModelError> {
        let (package, ids) = match change {
            Change::Splice { part, patch, draws, .. } if !draws.is_empty() => {
                // what the transaction's earlier splices inserted, named as applied here
                let named = self.redrawn_patch(part, patch, draws);
                let patch = named.as_ref().unwrap_or(patch);
                let (applied, added) = self.drawn_patch(patch, draws)?;
                let mut ids = self.ids.clone();
                ids.apply(&added);
                let package = self.with_tree(part, |t| applied.apply(t))?;
                self.redrawn.note(applied.inserted(), draws);
                (package, ids)
            }
            Change::Splice { part, patch, .. } => {
                debug_assert!(
                    !crate::drawn::uncovered(patch.inserted(), &[], &self.drawn),
                    "a stand-in in an id attribute no draw writes"
                );
                let named = self.redrawn_patch(part, patch, &[]);
                let patch = named.as_ref().unwrap_or(patch);
                (self.with_tree(part, |t| patch.apply(t))?, self.ids.clone())
            }
            Change::Head { part, patch, .. } => (self.with_tree(part, |t| patch.apply(t))?, self.ids.clone()),
            Change::Put { part, tree } => (self.package.with_new_tree(part, tree.clone()), self.ids.clone()),
            Change::Received { part, tree } => {
                let (package, ids) = self.received(part, tree)?;
                *self = self.succeed(package, ids);
                return Ok(());
            }
            Change::Bytes { part, bytes } => (self.package.with_bytes(part, Arc::clone(bytes)), self.ids.clone()),
            Change::Remove { part } => (self.package.without(part)?, self.ids.clone()),
            Change::Restore { parts, from } => {
                let mut package = self.package.package().clone();
                for part in parts.iter() {
                    package = package.restored(part, from)?;
                }
                (package, self.ids.clone())
            }
            Change::People { add, tidy, .. } => (self.people(add.as_deref(), tidy.as_deref())?, self.ids.clone()),
            Change::Ids(added) => {
                // the package is as it was: no part is written
                let mut ids = self.ids.clone();
                ids.apply(added);
                *self = self.succeed(self.package.package().clone(), ids);
                return Ok(());
            }
            Change::Draw { seed, stand_ins } => {
                let (values, added) = self.ids.draw(seed, stand_ins.len(), &self.shared.sequences);
                let mut ids = self.ids.clone();
                ids.apply(&added);
                // the package is as it was: its reading and tally stay this version's
                self.ids = ids;
                for (stand_in, value) in stand_ins.iter().zip(values) {
                    self.drawn.insert(stand_in, seed, value);
                }
                return Ok(());
            }
            Change::Spaced(count) => {
                // the package is as it was: its reading and tally stay this version's
                self.spaced += count;
                return Ok(());
            }
            Change::Number => {
                let ceiling =
                    crate::validate::annotation_ceiling(self).map_err(|e| ModelError::Unreadable(e.to_string()))?;
                let mut ids = self.ids.clone();
                ids.number_from(ceiling + 1);
                *self = self.succeed(self.package.package().clone(), ids);
                return Ok(());
            }
        };
        // a change to the package writes every part read with ids given at open: Word keeps
        // a document's ids only where every story's blocks have theirs (paraid-durability),
        // and the parts are read as they are written (no block is renamed)
        *self = self.succeed(package.written(), ids);
        Ok(())
    }

    fn inverse(&self, change: &Change) -> Result<Change, ModelError> {
        Ok(match change {
            Change::Splice {
                part,
                patch,
                keys,
                draws,
                created,
            } => {
                // what was inserted is what the draws made of it here, the blocks made
                // under stand-ins named as drawn here; what it removed of the transaction's
                // earlier splices, as they were applied here
                let named = self.redrawn_patch(part, patch, draws);
                let patch = named.as_ref().unwrap_or(patch);
                let patch = if draws.is_empty() {
                    patch.clone()
                } else {
                    self.drawn_patch(patch, draws)?.0
                };
                let mut all: Vec<Key> = keys.to_vec();
                all.extend(created.iter().map(|key| self.resolved(key)));
                Change::Splice {
                    part: Arc::clone(part),
                    patch: patch.inverse(),
                    keys: all.into(),
                    draws: Arc::from([]),
                    created: Arc::from([]),
                }
            }
            Change::Head { part, patch, keys } => Change::Head {
                part: Arc::clone(part),
                patch: patch.inverse(),
                keys: Arc::clone(keys),
            },
            Change::Put { part, .. }
            | Change::Received { part, .. }
            | Change::Bytes { part, .. }
            | Change::Remove { part } => Change::Restore {
                parts: Arc::from([Arc::clone(part)]),
                from: self.package.package().clone(),
            },
            Change::Restore { parts, .. } => Change::Restore {
                parts: Arc::clone(parts),
                from: self.package.package().clone(),
            },
            Change::People { .. } => {
                let main = self
                    .docx()
                    .map_err(|e| ModelError::Unreadable(e.to_string()))?
                    .main
                    .clone();
                Change::Restore {
                    parts: crate::people::parts_written(&self.package, &main)?.into(),
                    from: self.package.package().clone(),
                }
            }
            // ids given are never given again: nothing to take back
            Change::Ids(_) | Change::Draw { .. } | Change::Number => Change::Ids(IdsChange::default()),
            // a count for the reply: undoing the step writes no spaces
            Change::Spaced(_) => Change::Spaced(0),
        })
    }

    fn agrees(&self, other: &Self, keys: &KeySet<Key>) -> bool {
        let same_part = |name: &str| match (self.package.part(name), other.package.part(name)) {
            (None, None) => true,
            (Some(a), Some(b)) => a.same(b),
            _ => false,
        };
        match keys.keys() {
            // a block is compared through its part: conservative, never a false agreement
            Some(keys) => keys.iter().all(|key| match key {
                Key::Block { part, .. } | Key::Part(part) | Key::Root(part) | Key::StyleUse(part) => same_part(part),
                Key::Seq(_) | Key::Annotations | Key::People => true,
            }),
            None => {
                let names = self.package.part_names();
                names == other.package.part_names() && names.iter().all(|n| same_part(n))
            }
        }
    }

    fn written(change: &Change, into: &mut KeySet<Key>) {
        match change {
            Change::Splice { keys, .. } | Change::Head { keys, .. } => {
                for key in keys.iter() {
                    into.insert(key.clone());
                }
            }
            Change::Put { part, .. }
            | Change::Received { part, .. }
            | Change::Bytes { part, .. }
            | Change::Remove { part } => {
                let part = PartName::normalize(part);
                into.insert(Key::StyleUse(part.clone()));
                into.insert(Key::Part(part));
            }
            Change::Restore { parts, .. } => {
                for part in parts.iter() {
                    let part = PartName::normalize(part);
                    into.insert(Key::StyleUse(part.clone()));
                    into.insert(Key::Part(part));
                }
            }
            Change::People { keys, .. } => {
                for key in keys.iter() {
                    into.insert(key.clone());
                }
            }
            Change::Ids(added) => {
                for (seed, _) in &added.cursors {
                    into.insert(Key::Seq(Arc::from(seed.as_str())));
                }
                if added.highest_drawing > 0 || !added.drawings.is_empty() {
                    into.insert(Key::Seq(Arc::from(crate::pool::DRAWING_NUMBERS)));
                }
            }
            // numbering revision records adds annotation ids: a transaction that read the
            // ids in use (to number past them where it is prepared) conflicts with it
            Change::Number => {
                into.insert(Key::Annotations);
            }
            // a draw moves its seed's sequence: a transaction that read it (a stand-in
            // pinned, values chosen by it where prepared) conflicts with it
            Change::Draw { seed, .. } => {
                into.insert(Key::Seq(Arc::clone(seed)));
            }
            // a count for the reply: no key written
            Change::Spaced(_) => {}
        }
    }

    fn created(change: &Change, into: &mut KeySet<Key>) {
        if let Change::Splice { created, .. } = change {
            for key in created.iter() {
                into.insert(key.clone());
            }
        }
    }

    fn resolve(&self, created: &KeySet<Key>) -> KeySet<Key> {
        match created.keys() {
            Some(keys) => KeySet::of(keys.iter().map(|key| self.resolved(key))),
            None => KeySet::Everything,
        }
    }

    fn begin(&mut self) {
        if !self.drawn.is_empty() {
            self.drawn = crate::Drawn::default();
        }
        if !self.redrawn.is_empty() {
            self.redrawn = super::redrawn::Redrawn::default();
        }
        self.spaced = 0;
        self.as_given = false;
        self.begun = Some(self.package.clone());
        self.stamped = Arc::default();
    }

    fn set_up(&mut self, setup: &Setup) {
        self.as_given = setup.as_given;
    }

    fn kept(key: &Key) -> bool {
        // ids drawn stay drawn: undoing a step gives none of them back
        matches!(key, Key::Seq(_) | Key::Annotations)
    }

    fn derived(key: &Key) -> bool {
        // what a part's paragraphs show a reader of style uses is a function of its tree,
        // whose blocks (or the part) the writer keys as well
        matches!(key, Key::StyleUse(_))
    }

    fn carry(&mut self, head: &Self) {
        // the ledger is the document's: an undo or redo keeps every id given
        self.ids.carry(&head.ids);
    }

    fn settle(&self) {
        // the reading and the tally, each from the version before (the head's, settled):
        // what the next commit's draws and people change read where they land
        let _ = self.docx();
        let _ = self.tally();
    }

    fn merged(change: &Change, into: &mut KeySet<Key>) {
        match change {
            // revision ids drawn where the splice lands, the people part kept there
            Change::Splice { keys, .. } => {
                if keys.contains(&Key::Annotations) {
                    into.insert(Key::Annotations);
                }
            }
            Change::People { keys, .. } => {
                for key in keys.iter() {
                    into.insert(key.clone());
                }
            }
            // ids numbered from the version the change lands on: two edits numbering their
            // revision records give the serial result in either commit order
            Change::Number => {
                into.insert(Key::Annotations);
            }
            // values drawn from the version the change lands on, likewise
            Change::Draw { seed, .. } => {
                into.insert(Key::Seq(Arc::clone(seed)));
            }
            _ => {}
        }
    }

    fn rebase(change: &Change, past: &Change, first: First) -> Result<Change, ModelError> {
        // parts restored whole (an undo's inverse of a part put) cannot move past another
        // change of one of them, nor another change past them
        let restores = |restore: &Change, other: &Change| match (restore, other.part()) {
            (Change::Restore { parts, .. }, Some(part)) => parts.iter().any(|p| **p == *part),
            _ => false,
        };
        if restores(change, past) || restores(past, change) {
            return Err(ModelError::Patch(PatchError::Conflict(Vec::new())));
        }
        if change.part().is_none() || change.part() != past.part() {
            return Ok(change.clone());
        }
        Ok(match (change, past) {
            (
                Change::Splice {
                    part,
                    patch,
                    keys,
                    draws,
                    created,
                },
                Change::Splice { patch: earlier, .. },
            ) => Change::Splice {
                part: Arc::clone(part),
                // at one insertion point, the change published first keeps its place first
                patch: match first {
                    First::Past => patch.rebase(earlier)?,
                    First::Change => patch.rebase_first(earlier)?,
                },
                keys: Arc::clone(keys),
                draws: Arc::clone(draws),
                created: Arc::clone(created),
            },
            (
                Change::Splice {
                    part,
                    patch,
                    keys,
                    draws,
                    created,
                },
                Change::Head { patch: earlier, .. },
            ) => Change::Splice {
                part: Arc::clone(part),
                patch: patch.rebase_head(earlier)?,
                keys: Arc::clone(keys),
                draws: Arc::clone(draws),
                created: Arc::clone(created),
            },
            (Change::Head { part, patch, keys }, Change::Splice { patch: earlier, .. }) => Change::Head {
                part: Arc::clone(part),
                patch: patch.rebase(earlier)?,
                keys: Arc::clone(keys),
            },
            (Change::Head { part, patch, keys }, Change::Head { patch: earlier, .. }) => Change::Head {
                part: Arc::clone(part),
                patch: patch.rebase_head(earlier)?,
                keys: Arc::clone(keys),
            },
            // a part put or restored whole cannot be moved past another change of it
            _ => return Err(ModelError::Patch(PatchError::Conflict(Vec::new()))),
        })
    }
}
