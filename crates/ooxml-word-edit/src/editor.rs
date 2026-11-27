//! Owns: [`Editor`], one Word document's versions in an `ooxml-store` store with the
//! problems it opened with, and the run of a command through it: validate and prepare on a
//! snapshot, commit (rebased past concurrent commits, prepared again on a conflict), then
//! the result from the change and the version after it ([`Edited`]): what it touched and
//! the validation delta (design section 5, "Results"); every command followed by the
//! records of placed values kept in the document ([`crate::provenance`]). A command may also
//! run in a transaction a host's batch is handed ([`Editor::run_in`]): it then makes a
//! version only the batch reads, published with the transaction's commit.
//!
//! Must not: hold anything but the store and the baseline; read the clock but through the
//! store's port; run a command's effects outside its transaction.

use crate::doc::{Setup, WordDoc};
use crate::ids_delta::{ids_delta_of, IdsDelta};
use crate::pool::EarlierIds;
use crate::touched::{touched_of, Touched};
use crate::validate::{problems, Delta, Problem};
use crate::{EditError, Result};
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_store::{Caller, Clock, Command, CommitTiming, Snapshot, Store, StoreConfig, Txn};
use ooxml_word_model::ids::{LoadedState, VersionDelta};
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
    /// What its version changed against the one before ([`Edited::previous`]), by block
    /// identity: the store's change set of the version, which `touched` and `ids` are
    /// derived from. Empty when it changed nothing.
    pub changes: Arc<VersionDelta>,
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

impl<O> Edited<O> {
    /// The same result with what the command returned turned into `f`'s.
    pub fn map<U>(self, f: impl FnOnce(O) -> U) -> Edited<U> {
        let (out, rest) = self.split();
        rest.with(f(out))
    }

    /// What the command returned, and the rest.
    fn split(self) -> (O, Edited<()>) {
        let rest = Edited {
            out: (),
            version: self.version,
            previous: self.previous,
            changed: self.changed,
            drawn: self.drawn,
            spaced: self.spaced,
            changes: self.changes,
            touched: self.touched,
            ids: self.ids,
            validation: self.validation,
            attempts: self.attempts,
            exclusive: self.exclusive,
            timing: self.timing,
        };
        (self.out, rest)
    }
}

impl Edited<()> {
    /// The result with `out` as what the command returned.
    fn with<U>(self, out: U) -> Edited<U> {
        Edited {
            out,
            version: self.version,
            previous: self.previous,
            changed: self.changed,
            drawn: self.drawn,
            spaced: self.spaced,
            changes: self.changes,
            touched: self.touched,
            ids: self.ids,
            validation: self.validation,
            attempts: self.attempts,
            exclusive: self.exclusive,
            timing: self.timing,
        }
    }
}

impl<O, E> Edited<std::result::Result<O, E>> {
    /// The command's own refusal, returned as its out (nothing changed); else the result
    /// with what it returned.
    ///
    /// # Errors
    /// The refusal the command returned.
    pub fn transpose(self) -> std::result::Result<Edited<O>, E> {
        let (out, rest) = self.split();
        out.map(|out| rest.with(out))
    }
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
        let spaced = if committed.changed {
            committed.version.doc().spaced()
        } else {
            0
        };
        let ran = Ran {
            version: committed.version,
            previous: committed.previous.filter(|_| committed.changed),
            spaced,
            attempts: committed.attempts,
            exclusive: committed.exclusive,
            timing: committed.timing,
        };
        self.edited(committed.out, ran)
    }

    /// Run `command` as [`Editor::run_with`] does, in `tx` (a host's batch's transaction,
    /// committed by whoever runs it): validated and prepared on its working version, its
    /// changes taken back where it refuses. The version it makes ([`Txn::version_here`]) is
    /// the transaction's alone, its changes against `previous` (the version read before it,
    /// the batch's ops so far left).
    ///
    /// # Errors
    /// The command's refusal, nothing changed.
    pub fn run_in<C>(
        &self,
        tx: &mut Txn<'_, WordDoc>,
        previous: &Snapshot<WordDoc>,
        command: &C,
        setup: &Setup,
    ) -> Result<Edited<C::Out>>
    where
        C: Command<WordDoc, Error = EditError>,
    {
        let started = self.clock.monotonic();
        let before = tx.len();
        let (out, spaced) = prepared(tx, command, setup)?;
        let changed = tx.len() != before;
        let ran = Ran {
            previous: changed.then(|| Arc::clone(previous)),
            version: if changed {
                tx.version_here(previous)
            } else {
                Arc::clone(previous)
            },
            spaced: if changed { spaced } else { 0 },
            attempts: 1,
            exclusive: false,
            timing: CommitTiming {
                held: Duration::ZERO,
                waited: Duration::ZERO,
                total: self.clock.monotonic().saturating_sub(started),
            },
        };
        self.edited(out, ran)
    }

    /// A run's result: what it touched, the ids it removed and brought and the validation
    /// delta of the version it made.
    fn edited<O>(&self, out: O, ran: Ran) -> Result<Edited<O>> {
        let started = self.clock.monotonic();
        let (changes, touched, ids) = match &ran.previous {
            Some(previous) => {
                // the store's change set of the version: made against the version it was
                // published after, which is `previous`
                let changes = ran
                    .version
                    .changes()
                    .unwrap_or_else(|| Arc::new(ran.version.doc().delta_from(previous.doc())));
                let (before, after) = (previous.doc(), ran.version.doc());
                let touched = touched_of(before, after, &changes)?;
                let ids = ids_delta_of(before, after, &changes)?;
                (changes, touched, ids)
            }
            None => (Arc::default(), Vec::new(), IdsDelta::default()),
        };
        let found = self.clock.monotonic();
        let validation = Delta::of(&self.baseline, &problems(ran.version.doc())?);
        let validated = self.clock.monotonic();
        let changed = ran.previous.is_some();
        let drawn = if changed {
            ran.version.doc().drawn().clone()
        } else {
            crate::Drawn::default()
        };
        Ok(Edited {
            out,
            changed,
            drawn,
            spaced: ran.spaced,
            version: ran.version,
            previous: ran.previous,
            changes,
            touched,
            ids,
            validation,
            attempts: ran.attempts,
            exclusive: ran.exclusive,
            timing: EditTiming {
                commit: ran.timing,
                touched: found.saturating_sub(started),
                validation: validated.saturating_sub(found),
            },
        })
    }
}

/// `command` validated and prepared in `tx` as `setup` says, followed by the records of
/// placed values and the stamping of new blocks; its changes taken back where it refuses,
/// the transaction depending on what the refusal rests on ([`Txn::run_op`]: an op of a
/// batch or a held transaction, which goes on). What it returned and the spaces its language
/// spacing wrote.
fn prepared<C>(tx: &mut Txn<'_, WordDoc>, command: &C, setup: &Setup) -> Result<(C::Out, usize)>
where
    C: Command<WordDoc, Error = EditError>,
{
    let followed = crate::provenance::Followed(command);
    let stamped = crate::stamping::Stamped(&followed);
    tx.set_up(setup);
    let spaced = tx.doc().spaced();
    let out = tx.run_op(&stamped)?;
    Ok((out, tx.doc().spaced().saturating_sub(spaced)))
}

/// What a run made, before what it touched is worked out.
struct Ran {
    version: Snapshot<WordDoc>,
    /// The version it was applied to, where it changed the document.
    previous: Option<Snapshot<WordDoc>>,
    spaced: usize,
    attempts: u32,
    exclusive: bool,
    timing: CommitTiming,
}
