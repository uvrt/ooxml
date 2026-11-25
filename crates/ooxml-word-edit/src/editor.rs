//! Owns: [`Editor`], one Word document's versions in an `ooxml-store` store with the
//! problems it opened with, and the run of a command through it: validate and prepare on a
//! snapshot, commit (rebased past concurrent commits, prepared again on a conflict), then
//! the result from the change and the version after it ([`Edited`]): what it touched and
//! the validation delta (design section 5, "Results"); every command followed by the
//! records of placed values kept in the document ([`crate::provenance`]).
//!
//! Must not: hold anything but the store and the baseline; read the clock but through the
//! store's port; run a command's effects outside its transaction.

use crate::doc::{Setup, WordDoc};
use crate::ids_delta::{ids_delta, IdsDelta};
use crate::pool::EarlierIds;
use crate::touched::{touched, Touched};
use crate::validate::{problems, Delta, Problem};
use crate::{EditError, Result};
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, Command, CommitTiming, Snapshot, Store, StoreConfig};
use ooxml_word_model::ids::LoadedState;
use std::sync::Arc;
use std::time::Duration;

/// One Word document's versions, with the problems it opened with.
pub struct Editor {
    store: Store<WordDoc>,
    baseline: Vec<Problem>,
    clock: Arc<dyn Clock>,
}

/// Where a run's time went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EditTiming {
    /// Validate, prepare and commit (the store's measure, redo and waiting included).
    pub commit: CommitTiming,
    /// Working out what the change touched and the ids it retired.
    pub touched: Duration,
    /// The validation delta.
    pub validation: Duration,
}

/// A command's result after its commit.
#[derive(Debug)]
pub struct Edited<O> {
    /// What the command returned.
    pub out: O,
    /// The version made (the one it ran on when it changed nothing).
    pub version: Snapshot<WordDoc>,
    /// The version it was applied to (the head it was published after); `None` when it
    /// changed nothing.
    pub previous: Option<Snapshot<WordDoc>>,
    /// Whether it changed the document.
    pub changed: bool,
    /// What its stand-ins were drawn as where it was applied: ids the command returned that
    /// it drew are named through it ([`crate::Drawn::resolve`]). Empty when it changed
    /// nothing.
    pub drawn: crate::Drawn,
    /// How many spaces its language spacing wrote or made U+00A0 ([`crate::Change::Spaced`]);
    /// 0 when it changed nothing.
    pub spaced: usize,
    /// What it touched, per part.
    pub touched: Vec<Touched>,
    /// The block ids and drawing numbers it removed and brought.
    pub ids: IdsDelta,
    /// Its version's problems against those the document opened with.
    pub validation: Delta,
    /// How many times it was prepared.
    pub attempts: u32,
    /// Whether it ran behind the document's gate after conflicts.
    pub exclusive: bool,
    /// Where its time went.
    pub timing: EditTiming,
}

impl Editor {
    /// The document whose first version is `package`, its caches within `budget`.
    ///
    /// # Errors
    /// When the package is not a Word document a reading can open.
    pub fn open(
        package: Package,
        config: StoreConfig,
        clock: Arc<dyn Clock>,
        budget: Arc<dyn CacheBudget>,
    ) -> Result<Editor> {
        Editor::open_with(package, config, clock, budget, &EarlierIds::default())
    }

    /// The document whose first version is `package`, opened again with the ids it had
    /// before it was saved ([`EarlierIds`]).
    ///
    /// # Errors
    /// As [`Editor::open`].
    pub fn open_with(
        package: Package,
        config: StoreConfig,
        clock: Arc<dyn Clock>,
        budget: Arc<dyn CacheBudget>,
        earlier: &EarlierIds,
    ) -> Result<Editor> {
        let doc = WordDoc::open_with(package, budget, earlier)?;
        Editor::of_doc(doc, config, clock)
    }

    /// The document whose first version is `package`, opened again with the id state saved
    /// beside its bytes ([`WordDoc::open_with_state`]); and what the state recorded.
    ///
    /// # Errors
    /// As [`WordDoc::open_with_state`].
    pub fn open_with_state(
        package: Package,
        config: StoreConfig,
        clock: Arc<dyn Clock>,
        budget: Arc<dyn CacheBudget>,
        state: &str,
    ) -> Result<(Editor, LoadedState)> {
        let (doc, loaded) = WordDoc::open_with_state(package, budget, state)?;
        Ok((Editor::of_doc(doc, config, clock)?, loaded))
    }

    /// The editor of `doc` as Word opens it ([`crate::controls::opened_store`]: a bound
    /// control shows its node).
    fn of_doc(doc: WordDoc, config: StoreConfig, clock: Arc<dyn Clock>) -> Result<Editor> {
        let store = crate::controls::opened_store(doc, config, Arc::clone(&clock))?;
        let baseline = problems(store.snapshot().doc())?;
        Ok(Editor { store, baseline, clock })
    }

    /// The store.
    #[must_use]
    pub fn store(&self) -> &Store<WordDoc> {
        &self.store
    }

    /// The problems the document opened with.
    #[must_use]
    pub fn baseline(&self) -> &[Problem] {
        &self.baseline
    }

    /// Run `command` as `caller`: commit it, then report what it touched and the
    /// validation delta of the version it made.
    ///
    /// # Errors
    /// The command's refusal; nothing changed.
    pub fn run<C>(&self, caller: &Caller, command: &C) -> Result<Edited<C::Out>>
    where
        C: Command<WordDoc, Error = EditError>,
    {
        self.run_with(caller, command, &Setup::default())
    }

    /// [`Editor::run`], its transactions writing as `setup` says (text as given, or spaced
    /// by its language).
    ///
    /// # Errors
    /// The command's refusal; nothing changed.
    pub fn run_with<C>(&self, caller: &Caller, command: &C, setup: &Setup) -> Result<Edited<C::Out>>
    where
        C: Command<WordDoc, Error = EditError>,
    {
        // docx-agent's records of placed values follow every edit, in its own step; then
        // every block the edit left without a durable id is stamped
        let followed = crate::provenance::Followed(command);
        let committed = self
            .store
            .run_with(caller, &crate::stamping::Stamped(&followed), setup)?;
        let started = self.clock.monotonic();
        let (touched, ids) = match &committed.previous {
            Some(previous) if committed.changed => (
                touched(previous.doc(), committed.version.doc())?,
                ids_delta(previous.doc(), committed.version.doc())?,
            ),
            _ => (Vec::new(), IdsDelta::default()),
        };
        let found = self.clock.monotonic();
        let validation = Delta::of(&self.baseline, &problems(committed.version.doc())?);
        let validated = self.clock.monotonic();
        let (drawn, spaced) = if committed.changed {
            let doc = committed.version.doc();
            (doc.drawn().clone(), doc.spaced())
        } else {
            (crate::Drawn::default(), 0)
        };
        Ok(Edited {
            out: committed.out,
            changed: committed.changed,
            drawn,
            spaced,
            version: committed.version,
            previous: committed.previous,
            touched,
            ids,
            validation,
            attempts: committed.attempts,
            exclusive: committed.exclusive,
            timing: EditTiming {
                commit: committed.timing,
                touched: found.saturating_sub(started),
                validation: validated.saturating_sub(found),
            },
        })
    }
}
