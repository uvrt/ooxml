//! Owns: where an edit's stand-ins go ([`EditSink::draw`]'s values, drawn again where the
//! change is applied; every stand-in the transaction drew so far, as its working version
//! keeps them, [`crate::Drawn`]): each write is looked over before it reaches the
//! transaction. A splice gets a draw for every stand-in in an id attribute of what it
//! inserts, and the keys of the blocks it makes under stand-ins are kept apart as created
//! ([`ooxml_store::DocModel::created`]). A stand-in written where no draw reaches -- a part
//! put whole, a head changed in place, another attribute, text, a rename in the ledger -- is
//! pinned: its seed's sequence is read, so the commit conflicts with any draw from it since
//! and lands only where the stand-in is the value drawn (as every draw did before
//! stand-ins). A pinned stand-in a later splice holds is drawn there too, as the same value.
//!
//! Must not: write; decide what a value is (the ledger does where the change lands).

use super::EditSink;
use crate::doc::{Change, Draw, DrawFrom, Key};
use crate::drawn::{block_hex, scan, Drawn};
use ooxml_xml::Node;
use std::sync::Arc;

/// The stand-in a draw writes.
fn drawn_stand_in(draw: &Draw) -> Option<Arc<str>> {
    match &draw.from {
        DrawFrom::Drawn(stand_in) => Some(Arc::clone(stand_in)),
        _ => None,
    }
}

impl EditSink<'_, '_> {
    /// `change` as it goes into the transaction: a splice with its stand-ins drawn where it
    /// lands and the keys of what it makes under them apart; any stand-in it holds where no
    /// draw reaches pinned.
    pub(super) fn placed(&mut self, change: Change) -> Change {
        let drawn = self.tx.doc().drawn().clone();
        if drawn.is_empty() {
            return change;
        }
        match change {
            Change::Splice {
                part,
                patch,
                keys,
                draws,
                created,
            } => self.placed_splice(&drawn, part, patch, (&keys, &draws, &created)),
            Change::Head { part, patch, keys } => {
                let held: Vec<Arc<str>> = patch
                    .new_head()
                    .attributes
                    .iter()
                    .filter_map(|a| drawn.stand_in(&a.value).map(|(s, _)| s))
                    .collect();
                for stand_in in &held {
                    self.pin(stand_in);
                }
                Change::Head { part, patch, keys }
            }
            Change::Put { part, tree } => {
                // a part written whole is not drawn into: every stand-in it holds is pinned
                let (found, elsewhere) = scan(&[Node::Element(Arc::clone(tree.root()))], &drawn);
                for stand_in in found.iter().filter_map(drawn_stand_in).chain(elsewhere) {
                    self.pin(&stand_in);
                }
                Change::Put { part, tree }
            }
            Change::Bytes { .. } => {
                // bytes are not looked through: every stand-in is pinned
                let all: Vec<Arc<str>> = drawn.stand_ins().map(|(s, _)| Arc::clone(s)).collect();
                for stand_in in &all {
                    self.pin(stand_in);
                }
                change
            }
            Change::Ids(ids) => {
                // a rename or a forgotten alias naming a stand-in keeps it as written
                let named: Vec<Arc<str>> = ids
                    .aliases
                    .iter()
                    .flat_map(|(old, new)| [old, new])
                    .chain(&ids.forget)
                    .chain(&ids.forget_targets)
                    .filter_map(|id| drawn.stand_in(block_hex(id).1).map(|(s, _)| s))
                    .collect();
                for stand_in in &named {
                    self.pin(stand_in);
                }
                Change::Ids(ids)
            }
            other => other,
        }
    }

    /// A splice with a draw for every stand-in in an id attribute of what it inserts (beside
    /// its own draws), the keys of the blocks it makes under stand-ins kept apart as
    /// created; a stand-in no draw reaches pinned.
    fn placed_splice(
        &mut self,
        drawn: &Drawn,
        part: Arc<str>,
        patch: ooxml_xml::Patch,
        (keys, draws, created): (&[Key], &[Draw], &[Key]),
    ) -> Change {
        let (found, elsewhere) = scan(patch.inserted(), drawn);
        for stand_in in &elsewhere {
            self.pin(stand_in);
        }
        let mut all: Vec<Draw> = draws.to_vec();
        for draw in found {
            let given = all
                .iter()
                .any(|d| d.node == draw.node && d.at == draw.at && d.attribute == draw.attribute);
            if !given {
                all.push(draw);
            }
        }
        let names = |key: &Key| match key {
            Key::Block { id, .. } => drawn.stand_in(block_hex(id).1).is_some(),
            _ => false,
        };
        let (made, kept): (Vec<Key>, Vec<Key>) = keys.iter().cloned().partition(names);
        let mut created = created.to_vec();
        created.extend(made);
        Change::Splice {
            part,
            patch,
            keys: kept.into(),
            draws: all.into(),
            created: created.into(),
        }
    }

    /// Pin `stand_in`: read its seed's sequence, so the commit lands only where nothing
    /// was drawn from it since the snapshot (the stand-in is then the value drawn).
    fn pin(&mut self, stand_in: &str) {
        let seed = self
            .tx
            .doc()
            .drawn()
            .stand_in(stand_in)
            .map(|(_, seed)| Arc::clone(seed));
        if let Some(seed) = seed {
            self.tx.read(Key::Seq(seed));
        }
    }
}
