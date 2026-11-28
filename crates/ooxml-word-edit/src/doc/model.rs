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
        if self.strict() {
            return Err(ModelError::Strict);
        }
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
            Change::RootHead { .. }
            | Change::Declare { .. }
            | Change::Relinked { .. }
            | Change::Typed { .. }
            | Change::Moved { .. } => (self.merged_in(change)?, self.ids.clone()),
            Change::Put { part, tree } => (self.package.with_new_tree(part, tree.clone()), self.ids.clone()),
            Change::Arrived {
                part,
                patches,
                from,
                on,
                ..
            } => {
                let (package, ids) = self.arrived(part, patches.as_deref(), from, on.as_ref())?;
                *self = self.succeed(package, ids);
                return Ok(());
            }
            Change::Bytes { part, bytes } => (self.package.with_bytes(part, Arc::clone(bytes)), self.ids.clone()),
            Change::Remove { part } => (self.package.without(part)?, self.ids.clone()),
            Change::Restore { parts, from } => (self.restored(parts, from)?, self.ids.clone()),
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
            | Change::Arrived { part, .. }
            | Change::Bytes { part, .. }
            | Change::Remove { part } => Change::Restore {
                parts: Arc::from([Arc::clone(part)]),
                from: self.package.package().clone(),
            },
            Change::Restore { parts, .. } => Change::Restore {
                parts: Arc::clone(parts),
                from: self.package.package().clone(),
            },
            // the person listed is dropped again where nothing of theirs is left, one dropped
            // listed again: where the inverse lands, as the change was (no part restored
            // whole, so two steps listing people are not entangled by it)
            Change::People { add, tidy, keys } => Change::People {
                add: tidy.clone(),
                tidy: add.clone(),
                keys: Arc::clone(keys),
            },
            // another editor's bookkeeping merged (their step, which no undo takes back): the
            // parts it may write put back as they are here
            Change::Relinked { .. } | Change::Typed { .. } | Change::Moved { .. } => Change::Restore {
                parts: bookkeeping_parts(change).into(),
                from: self.package.package().clone(),
            },
            // ids given are never given again, a declaration made stays: nothing to take back
            // (another editor's head merged: their step, never taken back)
            Change::Ids(_)
            | Change::Draw { .. }
            | Change::Number
            | Change::Declare { .. }
            | Change::RootHead { .. } => Change::Ids(IdsChange::default()),
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
            Change::Declare { part, .. } | Change::RootHead { part, .. } => {
                into.insert(Key::Root(PartName::normalize(part)));
            }
            Change::Relinked { .. } | Change::Typed { .. } | Change::Moved { .. } => {
                for part in bookkeeping_parts(change) {
                    into.insert(Key::Part(PartName::normalize(&part)));
                }
            }
            Change::Arrived { part, keys, .. } => {
                into.insert(Key::StyleUse(PartName::normalize(part)));
                for key in keys.iter() {
                    into.insert(key.clone());
                }
            }
            Change::Splice { keys, .. } | Change::Head { keys, .. } => {
                for key in keys.iter() {
                    into.insert(key.clone());
                }
            }
            Change::Put { part, .. } | Change::Bytes { part, .. } | Change::Remove { part } => {
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
    }

    fn set_up(&mut self, setup: &Setup) {
        self.as_given = setup.as_given;
    }

    fn kept(key: &Key) -> bool {
        // ids drawn stay drawn: undoing a step gives none of them back
        matches!(key, Key::Seq(_) | Key::Annotations)
    }

    fn key_name(key: &Key) -> Option<String> {
        match key {
            Key::Block { id, .. } => Some(id.to_string()),
            Key::Part(part) | Key::Root(part) => Some(part.to_string()),
            Key::StyleUse(_) | Key::Seq(_) | Key::Annotations | Key::People => None,
        }
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
            // ids added to the ledger: a set, two adders do not conflict (a reader of a
            // sequence still conflicts with them)
            Change::Ids(_) => Self::written(change, into),
            // declarations added where the change lands, the root's own kept
            Change::Declare { part, .. } => {
                into.insert(Key::Root(PartName::normalize(part)));
            }
            // another editor's bookkeeping merged where it lands: ids and names made there
            Change::Relinked { .. } | Change::Typed { .. } | Change::Moved { .. } => {
                for part in bookkeeping_parts(change) {
                    into.insert(Key::Part(PartName::normalize(&part)));
                }
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
        // a declaration changes a root's head where it lands, never a child's place
        let head_only = |c: &Change| matches!(c, Change::Declare { .. } | Change::RootHead { .. });
        if head_only(change) || head_only(past) {
            return Ok(change.clone());
        }
        if let Some(moved) = arrived_rebase(change, past, first) {
            return moved;
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

/// `change` moved past `past` where another editor's arrival is one of them
/// ([`Change::Arrived`]); `None` where neither is.
fn arrived_rebase(change: &Change, past: &Change, first: First) -> Option<Result<Change, ModelError>> {
    // another editor's patches, a sequence made on the version `past` was made on, moved
    // past a splice of the same part: each past it as it stands where that patch was
    // made (carried past the ones before); the part then no longer theirs as written
    if let (
        Change::Arrived {
            part,
            patches: Some(theirs),
            keys,
            from,
            ..
        },
        Change::Splice { patch: other, .. },
    ) = (change, past)
    {
        let mut other = other.clone();
        let mut placed = theirs.to_vec();
        for patch in &mut placed {
            let moved = match first {
                First::Past => match patch.rebase(&other) {
                    Ok(p) => p,
                    Err(e) => return Some(Err(e.into())),
                },
                First::Change => match patch.rebase_first(&other) {
                    Ok(p) => p,
                    Err(e) => return Some(Err(e.into())),
                },
            };
            other = match first {
                First::Past => match other.rebase_first(patch) {
                    Ok(p) => p,
                    Err(e) => return Some(Err(e.into())),
                },
                First::Change => match other.rebase(patch) {
                    Ok(p) => p,
                    Err(e) => return Some(Err(e.into())),
                },
            };
            *patch = moved;
        }
        return Some(Ok(Change::Arrived {
            part: Arc::clone(part),
            patches: Some(placed.into()),
            keys: Arc::clone(keys),
            from: from.clone(),
            on: None,
        }));
    }
    // a change made before another editor's commit, moved past its patches in order
    // (that commit is published first: its nodes keep their place first)
    if let Change::Arrived {
        patches: Some(theirs), ..
    } = past
    {
        let mut moved = change.clone();
        for earlier in theirs.iter() {
            let splice = Change::Splice {
                part: Arc::from(change.part().unwrap_or_default()),
                patch: earlier.clone(),
                keys: Arc::from([]),
                draws: Arc::from([]),
                created: Arc::from([]),
            };
            moved = match WordDoc::rebase(&moved, &splice, first) {
                Ok(moved) => moved,
                Err(error) => return Some(Err(error)),
            };
        }
        return Some(Ok(moved));
    }
    None
}

/// The parts another editor's merged bookkeeping may write: a relationships part and its
/// source part; the content types; a moved part, the name it moves to, the content types and
/// every relationships part.
fn bookkeeping_parts(change: &Change) -> Vec<Arc<str>> {
    match change {
        Change::Relinked { part, .. } => {
            let mut parts = vec![Arc::clone(part)];
            parts.extend(crate::merging::source_of(part).map(|s| Arc::from(s.as_str())));
            parts
        }
        Change::Typed { .. } => vec![Arc::from(ooxml_opc::CONTENT_TYPES_PART)],
        Change::Moved { from, to, rels } => {
            let mut parts = vec![
                Arc::clone(from),
                Arc::clone(to),
                Arc::from(ooxml_opc::CONTENT_TYPES_PART),
            ];
            parts.extend(rels.iter().cloned());
            parts
        }
        _ => Vec::new(),
    }
}
