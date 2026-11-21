//! Owns: a Word document's retired ids ([`Retired`]): each block id and drawing number a
//! version of the document lost, with the step that removed it ([`RetiredBy`]: the version
//! it made, who made it and with which tool) -- recorded as each version is made (a served
//! call's edits, the bridge's update, an undo or redo), not read back from the store's
//! steps, which keep only the undo window. An id a later version brings back (an undone
//! deletion) is no longer retired; one removed again (redone) is retired by the version that
//! removed it again, its step's caller and tool kept. Ids retired before the document was
//! saved and opened again are given by the host ([`Session::seed_retired`]), as their state
//! says them. [`Session::retired`] reads the record. The host tells the session the number
//! it gave each version it numbered ([`Session::number_version`]), so a refusal of a retired
//! id names the change as the host numbers it ([`crate::refused`] words it).
//!
//! Must not: number versions itself, or word a refusal.

use crate::{DocId, Session, SessionError};
use ooxml_word_edit::IdsDelta;
use ooxml_word_model::ids::HeldId;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

/// Who retired an id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetiredBy {
    /// A version this session made: its number, its caller and the tool that made it
    /// (`None`: an undo or redo, or the bridge's update without one).
    Step {
        /// The version that removed the id.
        seq: u64,
        /// Who made it.
        caller: Option<Arc<str>>,
        /// The tool.
        tool: Option<Arc<str>>,
    },
    /// A change made before the document was saved and opened again: the version it made as
    /// the host numbered it, and who made it in the host's words (`None`: not known).
    Earlier {
        /// The host's number of the version.
        version: u64,
        /// Who made it.
        said: Option<String>,
    },
}

/// One id's record: who retired it, and whether it is retired now (else brought back, kept
/// for a redo that removes it again).
#[derive(Debug, Clone)]
struct Record {
    by: RetiredBy,
    retired: bool,
    /// The version of the last change to the record: an older one changes nothing.
    seq: u64,
}

/// A Word document's retired ids.
#[derive(Debug, Default)]
pub(crate) struct Retired {
    records: Mutex<BTreeMap<HeldId, Record>>,
    /// The session's version -> the number the host gave it, for each version it numbered.
    numbers: Mutex<BTreeMap<u64, u64>>,
}

/// What made a version, as the record keeps it.
pub(crate) struct Made<'a> {
    pub(crate) seq: u64,
    pub(crate) caller: Option<&'a str>,
    pub(crate) tool: Option<&'a str>,
    /// An undo or redo: an id it removes again keeps the step that removed it first.
    pub(crate) history: bool,
}

impl Retired {
    /// Note what the version `made` made did to the ids.
    pub(crate) fn note(&self, made: &Made<'_>, delta: &IdsDelta) {
        if delta.is_empty() {
            return;
        }
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        for id in &delta.gone {
            let by = RetiredBy::Step {
                seq: made.seq,
                caller: made.caller.map(Arc::from),
                tool: made.tool.map(Arc::from),
            };
            match records.get_mut(id) {
                Some(record) if record.seq > made.seq => {}
                Some(record) => {
                    record.by = match (&record.by, made.history) {
                        (RetiredBy::Step { caller, tool, .. }, true) => RetiredBy::Step {
                            seq: made.seq,
                            caller: caller.clone(),
                            tool: tool.clone(),
                        },
                        _ => by,
                    };
                    record.retired = true;
                    record.seq = made.seq;
                }
                None => {
                    records.insert(
                        *id,
                        Record {
                            by,
                            retired: true,
                            seq: made.seq,
                        },
                    );
                }
            }
        }
        for id in &delta.came {
            if let Some(record) = records.get_mut(id).filter(|r| r.seq <= made.seq) {
                record.retired = false;
                record.seq = made.seq;
            }
        }
    }

    /// Take the ids a state retired before the document was opened again.
    fn seed(&self, given: Vec<(HeldId, u64, Option<String>)>) {
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        for (id, version, said) in given {
            records.entry(id).or_insert(Record {
                by: RetiredBy::Earlier { version, said },
                retired: true,
                seq: 0,
            });
        }
    }

    /// The host numbered the session's version `seq` as its version `number`.
    fn number(&self, seq: u64, number: u64) {
        self.numbers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(seq, number);
    }

    /// The number of the host's step a session version `seq` is part of: the first version
    /// numbered from it on (a call that made several versions is one step, numbered at its
    /// last); `None` before the host numbered it (its call still ending).
    pub(crate) fn number_of(&self, seq: u64) -> Option<u64> {
        let numbers = self.numbers.lock().unwrap_or_else(PoisonError::into_inner);
        numbers.range(seq..).next().map(|(_, number)| *number)
    }

    /// Who retired `id`, where it is retired now.
    pub(crate) fn by(&self, id: HeldId) -> Option<RetiredBy> {
        let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        records.get(&id).filter(|r| r.retired).map(|r| r.by.clone())
    }

    /// The ids retired now, in order.
    fn list(&self) -> Vec<(HeldId, RetiredBy)> {
        let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        records
            .iter()
            .filter(|(_, r)| r.retired)
            .map(|(id, r)| (*id, r.by.clone()))
            .collect()
    }
}

impl Session {
    /// The Word document's retired ids: each block id and drawing number one of its versions
    /// removed and none brought back since, with who removed it. Empty for another kind.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn retired(&self, id: &DocId) -> Result<Vec<(HeldId, RetiredBy)>, SessionError> {
        let document = self.document(id)?;
        Ok(document.word().map(|w| w.retired.list()).unwrap_or_default())
    }

    /// Give the Word document the ids a state saved beside its bytes says were retired
    /// before it was opened again: each with the version that removed it as the host
    /// numbered it, and who made that version in the host's words. An id already recorded
    /// keeps its record.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn seed_retired(&self, id: &DocId, given: Vec<(HeldId, u64, Option<String>)>) -> Result<(), SessionError> {
        let document = self.document(id)?;
        if let Some(word) = document.word() {
            word.retired.seed(given);
        }
        Ok(())
    }

    /// The host numbered the Word document's version `seq` (the session's) as its version
    /// `number`: a refusal of an id a version up to it retired names the change by it.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn number_version(&self, id: &DocId, seq: u64, number: u64) -> Result<(), SessionError> {
        let document = self.document(id)?;
        if let Some(word) = document.word() {
            word.retired.number(seq, number);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(gone: &[u32], came: &[u32]) -> IdsDelta {
        IdsDelta {
            gone: gone.iter().copied().map(HeldId::Block).collect(),
            came: came.iter().copied().map(HeldId::Block).collect(),
        }
    }

    fn made(seq: u64, tool: Option<&'static str>, history: bool) -> Made<'static> {
        Made {
            seq,
            caller: Some("{\"agent\":\"a\"}"),
            tool,
            history,
        }
    }

    #[test]
    fn an_undone_deletion_is_back_and_a_redone_one_keeps_its_tool() {
        let retired = Retired::default();
        retired.note(&made(2, Some("word_delete"), false), &delta(&[7], &[]));
        assert_eq!(retired.list().len(), 1);
        retired.note(&made(3, None, true), &delta(&[], &[7]));
        assert!(retired.list().is_empty());
        retired.note(&made(4, None, true), &delta(&[7], &[]));
        let list = retired.list();
        let Some((
            HeldId::Block(7),
            RetiredBy::Step {
                seq: 4,
                tool: Some(tool),
                ..
            },
        )) = list.first()
        else {
            panic!("{list:?}");
        };
        assert_eq!(&**tool, "word_delete");
    }

    #[test]
    fn an_older_version_changes_nothing_and_seeds_do_not_replace() {
        let retired = Retired::default();
        retired.note(&made(5, Some("word_delete"), false), &delta(&[1], &[]));
        retired.note(&made(4, None, true), &delta(&[], &[1]));
        assert_eq!(retired.list().len(), 1);
        retired.seed(vec![
            (HeldId::Block(1), 9, None),
            (HeldId::Drawing(3), 9, Some("x".into())),
        ]);
        let list = retired.list();
        assert!(matches!(list[0].1, RetiredBy::Step { seq: 5, .. }));
        assert_eq!(
            list[1].1,
            RetiredBy::Earlier {
                version: 9,
                said: Some("x".into())
            }
        );
    }
}
