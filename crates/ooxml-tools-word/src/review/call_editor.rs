//! Owns: [`CallEditor`], a document's editor as one call's handler is handed it: each edit
//! runs through the editor as before (one command, one history step) -- or, in a host's
//! batch, in the batch's transaction: the one it is handed ([`CallEditor::in_txn`], a batch
//! of one document, optimistic: [`Editor::run_in`]) or the one it holds open
//! ([`CallEditor::with_owner`], the Python host's batch: [`Editor::run_held`]), a savepoint per
//! command, published with the batch's one commit -- and the version the call made is
//! kept ([`Made`]) for the host: the call's own, whatever other calls commit to the document
//! meanwhile (the head after the call may be theirs). The version a handler reads is the
//! one its edits would run on ([`CallEditor::current`]).
//!
//! Must not: change how an edit runs, or outlive the call.

use ooxml_store::{Caller, Command, OpTxn, Seq, Snapshot, Store, TxnOwner};
use ooxml_word_edit::touched::Touched;
use ooxml_word_edit::validate::{Delta, Problem};
use ooxml_word_edit::{EditError, Edited, Editor, IdsDelta, Setup, WordDoc};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

/// The versions one call made: its first edit's and its last's (the same for one edit),
/// with what its last edit touched and the validation delta of the version it made (the
/// editor works both out with each edit).
#[derive(Debug, Clone)]
pub struct Made {
    /// The version the call's first changing edit was applied to (the head it was published
    /// after): where the call's step begins.
    pub before: Snapshot<WordDoc>,
    /// The version the call's first changing edit made.
    pub first: Snapshot<WordDoc>,
    /// The version its last changing edit made: the call's step ends there.
    pub last: Snapshot<WordDoc>,
    /// What the last edit touched (from the version it was applied to).
    pub touched: Vec<Touched>,
    /// The validation delta of `last`.
    pub validation: Delta,
    /// The ids each changing edit removed and brought, by the version it made.
    pub ids: Vec<(Seq, IdsDelta)>,
    /// How many spaces the call's edits spaced for their text's language, all together
    /// ([`Edited::spaced`]).
    pub spaced: usize,
    /// Whether its edits ran in a host's batch: its versions are the batch's, read by the
    /// batch alone until it commits, and the step is the batch's.
    pub batch: bool,
    /// How long its edits held the document's commit lock, all together.
    pub held: Duration,
    /// How long they waited for the commit lock and the document's gate.
    pub waited: Duration,
}

/// The document's editor for one call: its edits, and the versions they made.
pub struct CallEditor<'a> {
    editor: &'a Editor,
    /// How the call's edits write ([`Setup`]: text as given, or spaced by its language).
    setup: Setup,
    /// The owner of the transaction the call's edits run in (a host's batch); `None`: each
    /// edit commits on its own.
    owner: Option<TxnOwner>,
    /// The transaction the call's edits run in, handed to it (a host's batch of one
    /// document); `None`: as `owner` says.
    txn: Option<&'a dyn OpTxn>,
    made: Mutex<Option<Made>>,
    /// The stand-ins its edits drew that were drawn as other values where they landed.
    moved: Mutex<Vec<(String, String)>>,
}

impl<'a> CallEditor<'a> {
    /// `editor`, for one call.
    #[must_use]
    pub fn new(editor: &'a Editor) -> Self {
        Self {
            editor,
            setup: Setup::default(),
            owner: None,
            txn: None,
            made: Mutex::new(None),
            moved: Mutex::new(Vec::new()),
        }
    }

    /// The same, its edits writing as `setup` says (the call's setting).
    #[must_use]
    pub fn with_setup(self, setup: Setup) -> Self {
        Self { setup, ..self }
    }

    /// The same, its edits run in the transaction `owner` holds open (a host's batch;
    /// `None`: each commits on its own).
    #[must_use]
    pub fn with_owner(self, owner: Option<TxnOwner>) -> Self {
        Self { owner, ..self }
    }

    /// The same, its edits run in `txn` (a host's batch of one document, handed the batch's
    /// transaction; `None`: as [`CallEditor::with_owner`] says).
    #[must_use]
    pub fn in_txn(self, txn: Option<&'a dyn OpTxn>) -> Self {
        Self { txn, ..self }
    }

    /// Whether the call's edits run in a host's batch: what an edit would lay the whole
    /// document out for waits for the host, which measures it once after the batch's last op.
    #[must_use]
    pub fn in_batch(&self) -> bool {
        self.owner.is_some() || self.txn.is_some()
    }

    /// The version the call's next edit runs on: the head, or in a host's batch the version
    /// its ops left.
    #[must_use]
    pub fn current(&self) -> Snapshot<WordDoc> {
        let store = self.editor.store();
        if let Some(current) = self.txn.and_then(|txn| txn.current::<WordDoc>()) {
            return current;
        }
        self.owner
            .and_then(|owner| store.txn_current(owner))
            .unwrap_or_else(|| store.snapshot())
    }

    /// Run `command` as `caller` through the editor ([`Editor::run_with`], as the call's
    /// setup says; in a batch [`Editor::run_held`], the batch's caller making its step); a
    /// version it made is the call's.
    ///
    /// # Errors
    /// The edit's.
    pub fn run<C>(&self, caller: &Caller, command: &C) -> Result<Edited<C::Out>, EditError>
    where
        C: Command<WordDoc, Error = EditError>,
    {
        let edited = match (self.txn, self.owner) {
            (Some(txn), _) => txn
                .with::<WordDoc, _>(|tx, current| {
                    let edited = self.editor.run_in(tx, current, command, &self.setup)?;
                    *current = Snapshot::clone(&edited.version);
                    Ok(edited)
                })
                .unwrap_or_else(|| Err(EditError::from(ooxml_store::Error::<WordDoc>::NotHeld)))?,
            (None, Some(owner)) => self.editor.run_held(owner, command, &self.setup)?,
            (None, None) => self.editor.run_with(caller, command, &self.setup)?,
        };
        if edited.drawn.moved().next().is_some() {
            let mut moved = self.moved.lock().unwrap_or_else(PoisonError::into_inner);
            moved.extend(edited.drawn.moved().map(|(s, v)| (s.to_owned(), v.to_owned())));
        }
        if edited.changed {
            let mut made = self.made.lock().unwrap_or_else(PoisonError::into_inner);
            let (before, first, mut ids, spaced, held, waited) = made.take().map_or_else(
                || {
                    let before = edited.previous.clone().unwrap_or_else(|| edited.version.clone());
                    (
                        before,
                        edited.version.clone(),
                        Vec::new(),
                        0,
                        Duration::ZERO,
                        Duration::ZERO,
                    )
                },
                |m| (m.before, m.first, m.ids, m.spaced, m.held, m.waited),
            );
            ids.push((edited.version.seq(), edited.ids.clone()));
            *made = Some(Made {
                before,
                first,
                last: edited.version.clone(),
                touched: edited.touched.clone(),
                validation: edited.validation.clone(),
                ids,
                spaced: spaced + edited.spaced,
                batch: self.in_batch(),
                held: held + edited.timing.commit.held,
                waited: waited + edited.timing.commit.waited,
            });
        }
        Ok(edited)
    }

    /// The document's versions (what a handler reads is [`CallEditor::current`]).
    #[must_use]
    pub fn store(&self) -> &Store<WordDoc> {
        self.editor.store()
    }

    /// The problems the document opened with.
    #[must_use]
    pub fn baseline(&self) -> &[Problem] {
        self.editor.baseline()
    }

    /// The stand-ins the call's edits drew that landed as other values: `(stand-in, value)`
    /// ([`crate::resolve_drawn`] names them so in the reply).
    #[must_use]
    pub fn moved(&self) -> Vec<(String, String)> {
        self.moved.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The versions the call's edits made so far; `None` while none changed the document.
    #[must_use]
    pub fn made(&self) -> Option<Made> {
        self.made.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}
