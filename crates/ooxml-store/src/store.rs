//! Owns: [`Store`], one document's versions: the lock-free head readers load, the commit
//! (conflict detection by the keys a transaction read and wrote and by its reads by
//! condition, checked on the head against the keys written since; redo, then a run under the
//! commit lock), history operations, the id allocator and the reopen state; the states'
//! numbers ([`Version::number`]).
//!
//! **Numbers.**  Each published version is numbered as it is published, in publish order,
//! as a tool layer numbers a document's states: the version opened is 0; a commit gets one
//! more than the highest number given; a plain undo whose result is the step's "before"
//! itself gets that version's number, a plain redo whose result is the step's "after" that
//! version's; any other undo or redo (a scoped one, or one moved past later changes) makes
//! a state that never was, a new number. A reopen starts at 0 again. A held transaction (a
//! host's batch, [`Store::open_txn`]) is one step and one number: the versions its owner
//! reads carry the number its commit will be given.
//!
//! **The commit lock** is held for the publish only: a commit checks what was written since
//! its snapshot, and rebases and replays its changes onto the head, outside it; under it,
//! that the head is still the one it replayed onto, the numbers, the head, the history and
//! the journal. A command that conflicted [`StoreConfig::optimistic_tries`] times (once, for
//! one that read the whole document) closes the per-document **gate** and prepares again on
//! the head: other commits wait for the gate before they publish -- not holding the lock --
//! while readers, undo, redo and the reads of the history go on (design section 5).
//!
//! Must not: prepare a command, or replay a change set, inside the commit lock (but for a
//! commit whose head moved [`MOVED_TRIES`] times, which replays under it); free old versions
//! inside it; wait for the gate while holding it.

use crate::changeset::ChangeSet;
use crate::history::{Garbage, History, Planned, Step};
use crate::ids::IdAllocator;
use crate::journal::{Journal, Since};
use crate::stats::Counts;
use crate::sync::{lock, wait, Condvar, HeadCell, Mutex, MutexGuard};
use crate::times::LockTimes;
use crate::txn::{Ids, TxnParts};
use crate::{
    Clock, Command, Counters, CountersError, DocModel, Error, HistoryOutcome, KeySet, ReadPredicate, Seq, Snapshot,
    StepInfo, StoreError, StoreStats, Txn, Version,
};
use std::sync::Arc;
use std::time::Duration;

mod receive;
pub use receive::{HandedBack, Prepared, Received};

/// A store's configuration, a value given when it opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreConfig {
    /// How many steps undo can reach (and scoped undo keeps for redo).
    pub history_depth: usize,
    /// How many times a transaction is prepared on a snapshot before it runs under the
    /// commit lock.
    pub optimistic_tries: u32,
    /// How many of the latest commits' change sets are kept for rebasing: a transaction
    /// whose snapshot is older than that, with commits since, is prepared again.
    pub rebase_window: usize,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            history_depth: 50,
            optimistic_tries: 3,
            // Measured (D2c, host_batch_bench): a host's batch of 20 edits prepares in 100-180 ms
            // beside six writers committing about 450 times a second, so 45-80 commits land
            // meanwhile. At 64, most such batches found the change sets they had to move past
            // let go and ended behind the gate; at 256 none did, on a short document and on a
            // 61-page one, for about 50 MB more peak memory there (of 2 GB) -- 512 and 1024 gave
            // nothing more. A batch that outlasts even this runs behind the gate, as it should.
            rebase_window: 256,
        }
    }
}

/// How many times a commit whose head moved between its check and its publish checks again
/// outside the commit lock before it checks under it.
const MOVED_TRIES: u32 = 3;

/// Who runs a command; kept with its history step (scoped undo by writer). A foreign caller
/// is another editor whose commit came in from outside the session (a co-authoring
/// refresh, [`Store::receive`]): no undo takes its step back, and a transaction whose keys
/// it changed meanwhile is refused rather than prepared again
/// ([`StoreError::ForeignChange`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Caller {
    name: Option<Arc<str>>,
    foreign: bool,
}

impl Caller {
    /// No name.
    pub fn anonymous() -> Self {
        Self::default()
    }

    /// A named caller.
    pub fn named(name: &str) -> Self {
        Self {
            name: Some(Arc::from(name)),
            foreign: false,
        }
    }

    /// Another editor, by name.
    pub fn foreign(editor: &str) -> Self {
        Self {
            name: Some(Arc::from(editor)),
            foreign: true,
        }
    }

    /// The name.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Whether it is another editor.
    pub fn is_foreign(&self) -> bool {
        self.foreign
    }

    /// The foreign editor's name, for the journal.
    fn foreign_name(&self) -> Option<Arc<str>> {
        self.name.clone().filter(|_| self.foreign)
    }
}

/// Where a commit's time went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommitTiming {
    /// Inside the commit lock, for the attempt that committed.
    pub held: Duration,
    /// Waiting: for the commit lock, and for the gate (another command's run behind it, or
    /// this one's turn to close it).
    pub waited: Duration,
    /// From the first validate to the publish, redo and waiting included.
    pub total: Duration,
}

/// A command's result after its commit.
#[derive(Debug)]
pub struct Committed<D: DocModel, O> {
    /// The version made, or the snapshot it ran on when it changed nothing.
    pub version: Snapshot<D>,
    /// The version it was applied to (the head it was published after, which may be
    /// newer than the snapshot it was prepared on); `None` when it changed nothing.
    pub previous: Option<Snapshot<D>>,
    /// What the command returned.
    pub out: O,
    /// Whether it changed the document (a no-op makes no version and no history step).
    pub changed: bool,
    /// The keys it wrote.
    pub writes: KeySet<D::Key>,
    /// How many times it was prepared.
    pub attempts: u32,
    /// Whether it ran behind the gate after conflicts.
    pub exclusive: bool,
    /// Where its time went.
    pub timing: CommitTiming,
}

/// What a commit section returns.
struct Done<D: DocModel> {
    version: Snapshot<D>,
    previous: Option<Snapshot<D>>,
    changed: bool,
    writes: KeySet<D::Key>,
    held: Duration,
    waited: Duration,
}

/// A transaction's work as it applies to a head: the version, and the changes that make it.
type Onto<D> = (D, Arc<[<D as DocModel>::Change]>);

/// Why a commit section published nothing: a conflict, and the other editor's commit in the
/// way where one is ([`Since::foreign_in_way`]).
struct Conflict<D: DocModel> {
    foreign: Option<crate::journal::ForeignHit<D::Key>>,
}

impl<D: DocModel> Conflict<D> {
    /// The conflict of `parts` (depending on `depends`) with the entries `since` its base,
    /// up to `head`.
    fn of(since: &Since<D>, depends: &KeySet<D::Key>, parts: &TxnParts<D>, head: &Snapshot<D>) -> Self {
        Conflict {
            foreign: since.foreign_in_way(depends, &parts.predicates, head.doc()),
        }
    }
}

/// A committed transaction's record, before it has versions around it.
struct Body<D: DocModel> {
    changes: Arc<[D::Change]>,
    reads: KeySet<D::Key>,
    writes: KeySet<D::Key>,
    caller: Caller,
}

/// A store's history as [`Store::listed`] reads it, in one go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed<K: Ord> {
    /// The highest state number given: the next step's is one more.
    pub highest: u64,
    /// The steps undo can reach, oldest first.
    pub steps: Vec<StepInfo<K>>,
    /// The steps plain redo can bring back, the next one last.
    pub redo: Vec<StepInfo<K>>,
}

/// The state under the commit lock.
pub(crate) struct State<D: DocModel> {
    /// The highest state number given ([`Version::number`]).
    pub(crate) numbered: u64,
    pub(crate) history: History<D>,
    /// The latest versions' change sets: within the rebase window, and back to the oldest
    /// version the history may still move a change set from.
    journal: Journal<D>,
    /// How long this lock was held and waited for.
    times: LockTimes,
    /// Since when a command plans and commits behind the gate (`None`: open): another
    /// commit publishes only once it is open again ([`Store::enter_gate`]).
    pub(crate) gated: Option<Duration>,
    /// Whether an owner holds a transaction open ([`Store::open_txn`]): it holds the gate
    /// too, and undo and redo are refused until it is committed or dropped.
    pub(crate) held: bool,
    /// How many commits found the gate closed and have not published (or given up) since:
    /// the gate is closed again only once they have, so a batch, or a command behind the
    /// gate, never jumps the writers that waited for the one before it.
    queued: usize,
}

/// The state, locked: when it is let go, how long it was held is counted (the wait is
/// counted as it is taken).
struct Locked<'s, D: DocModel> {
    guard: MutexGuard<'s, State<D>>,
    clock: &'s dyn Clock,
    entered: Duration,
    /// How long it was waited for (the gate too, where it was).
    waited: Duration,
}

impl<D: DocModel> std::ops::Deref for Locked<'_, D> {
    type Target = State<D>;
    fn deref(&self) -> &State<D> {
        &self.guard
    }
}

impl<D: DocModel> std::ops::DerefMut for Locked<'_, D> {
    fn deref_mut(&mut self) -> &mut State<D> {
        &mut self.guard
    }
}

impl<D: DocModel> Drop for Locked<'_, D> {
    fn drop(&mut self) {
        let held = self.clock.monotonic().saturating_sub(self.entered);
        self.guard.times.held.record(held);
    }
}

/// The gate, closed by a command that plans and commits behind it ([`Store::enter_gate`]);
/// opened again when dropped, on every path.
struct Gate<'s, D: DocModel> {
    store: &'s Store<D>,
}

impl<D: DocModel> Drop for Gate<'_, D> {
    fn drop(&mut self) {
        self.store.open_gate(|_| ());
    }
}

/// A commit's place among those that found the gate closed ([`State::queued`]): left as it
/// publishes, or when it gives up (dropped).
struct Queue<'s, D: DocModel> {
    store: &'s Store<D>,
    queued: bool,
}

impl<D: DocModel> Queue<'_, D> {
    /// Leave the queue, the commit lock held (`state`); the last to leave lets a closer of
    /// the gate go on.
    fn leave(&mut self, state: &mut State<D>) {
        if std::mem::take(&mut self.queued) {
            state.queued -= 1;
            if state.queued == 0 {
                self.store.gate.notify_all();
            }
        }
    }
}

impl<D: DocModel> Drop for Queue<'_, D> {
    fn drop(&mut self) {
        if self.queued {
            let mut state = lock(&self.store.state);
            self.leave(&mut state);
        }
    }
}

/// One document's versions. Readers load the head without a lock; writers prepare in
/// parallel and commit in a short critical section.
pub struct Store<D: DocModel> {
    head: HeadCell<Version<D>>,
    state: Mutex<State<D>>,
    /// Signalled when the gate opens.
    gate: Condvar,
    ids: Arc<IdAllocator>,
    /// The transaction an owner holds open across calls, if any ([`Store::open_txn`]).
    pub(crate) holder: Mutex<Option<crate::held::HeldTxn<D>>>,
    /// Signalled when a held transaction is let go.
    pub(crate) released: Condvar,
    config: StoreConfig,
    pub(crate) clock: Arc<dyn Clock>,
    counts: Counts,
}

impl<D: DocModel> Store<D> {
    /// A store whose first version is `doc`, numbered 0.
    pub fn new(doc: D, config: StoreConfig, clock: Arc<dyn Clock>) -> Self {
        Self::open(doc, config, clock, Counters::default())
    }

    /// A store for a document opened again with the counters saved beside it: it opens at
    /// the saved version's number, the next change gets one more than the last number
    /// given, and every id sequence goes on where it got.
    pub fn reopen(
        doc: D,
        config: StoreConfig,
        clock: Arc<dyn Clock>,
        counters: &Counters,
    ) -> Result<Self, CountersError> {
        counters.check()?;
        Ok(Self::open(doc, config, clock, counters.clone()))
    }

    fn open(doc: D, config: StoreConfig, clock: Arc<dyn Clock>, counters: Counters) -> Self {
        Self {
            head: HeadCell::new(Arc::new(Version::first(counters.version, doc))),
            state: Mutex::new(State {
                numbered: 0,
                history: History::new(config.history_depth),
                journal: Journal::new(),
                times: LockTimes::default(),
                gated: None,
                held: false,
                queued: 0,
            }),
            gate: Condvar::new(),
            holder: Mutex::new(None),
            released: Condvar::new(),
            ids: Arc::new(IdAllocator::new(counters.cursors, counters.last)),
            config,
            clock,
            counts: Counts::new(),
        }
    }

    /// Go on with the numbers a tool layer gave the document's states before it was opened
    /// again ([`Version::number`]): the head is the state numbered `state`, the next step's
    /// number one more than `highest`. Only before the store numbered anything itself.
    ///
    /// # Errors
    /// [`CountersError::StateAboveHighest`]; [`CountersError::AlreadyNumbered`] once the
    /// store made a step or a transaction is held open.
    pub fn continue_numbers(&self, state: u64, highest: u64) -> Result<(), CountersError> {
        if state > highest {
            return Err(CountersError::StateAboveHighest { state, highest });
        }
        let mut locked = self.lock_state();
        let head = self.head.load();
        if locked.numbered != 0 || head.number() != 0 || locked.held || !locked.history.steps().is_empty() {
            return Err(CountersError::AlreadyNumbered);
        }
        locked.numbered = highest;
        self.head.store(Arc::new(head.renumbered(state)));
        Ok(())
    }

    /// The head. Never waits for a writer.
    pub fn snapshot(&self) -> Snapshot<D> {
        self.head.load()
    }

    /// How many commits wait for the gate now (or waited for it and have not published yet).
    pub fn gate_queue(&self) -> usize {
        self.lock_state().queued
    }

    /// How long the commit lock was held and waited for, each time it was taken so far.
    pub fn lock_times(&self) -> LockTimes {
        self.lock_state().times.clone()
    }

    /// The state under the commit lock; the wait counted.
    fn lock_state(&self) -> Locked<'_, D> {
        let asked = self.clock.monotonic();
        let mut guard = lock(&self.state);
        let entered = self.clock.monotonic();
        let waited = entered.saturating_sub(asked);
        guard.times.waited.record(waited);
        Locked {
            guard,
            clock: &*self.clock,
            entered,
            waited,
        }
    }

    /// The state under the commit lock with the gate open, or held by the caller
    /// (`gated`): a commit waits here, the lock let go meanwhile, while a command plans
    /// behind the gate -- queued (`queue`) from its first wait until it publishes or gives
    /// up, so the gate is handed to it before anyone closes it again.
    fn lock_ungated<'s>(&'s self, gated: bool, queue: &mut Queue<'s, D>) -> Locked<'s, D> {
        let locked = self.lock_state();
        if gated || locked.gated.is_none() {
            return locked;
        }
        drop(locked);
        let asked = self.clock.monotonic();
        let mut guard = lock(&self.state);
        if guard.gated.is_some() && !queue.queued {
            guard.queued += 1;
            queue.queued = true;
        }
        while guard.gated.is_some() {
            guard = wait(&self.gate, guard);
        }
        let entered = self.clock.monotonic();
        let waited = entered.saturating_sub(asked);
        guard.times.waited.record(waited);
        guard.times.gate_waited.record(waited);
        Locked {
            guard,
            clock: &*self.clock,
            entered,
            waited,
        }
    }

    /// Close the gate, once it is open: until the returned guard is dropped, no other
    /// commit publishes (each waits in [`Store::lock_ungated`]); undo, redo and the reads of
    /// the history go on. The commit lock is not held meanwhile.
    fn enter_gate(&self) -> Gate<'_, D> {
        drop(self.close_gate());
        Gate { store: self }
    }

    /// Close the gate once it is open (waiting meanwhile, the commit lock let go), and the
    /// commit lock, still held, for what the closer does as it closes it.
    pub(crate) fn close_gate(&self) -> MutexGuard<'_, State<D>> {
        let asked = self.clock.monotonic();
        let mut guard = lock(&self.state);
        // the commits that waited for the gate before go first ([`State::queued`])
        while guard.gated.is_some() || guard.queued > 0 {
            guard = wait(&self.gate, guard);
        }
        let entered = self.clock.monotonic();
        guard.times.gate_waited.record(entered.saturating_sub(asked));
        guard.gated = Some(entered);
        guard
    }

    /// Open the gate again, `also` done under the same hold of the commit lock; the commits
    /// waiting for it go on.
    pub(crate) fn open_gate(&self, also: impl FnOnce(&mut State<D>)) {
        let mut guard = lock(&self.state);
        if let Some(since) = guard.gated.take() {
            let held = self.clock.monotonic().saturating_sub(since);
            guard.times.gated.record(held);
        }
        also(&mut guard);
        drop(guard);
        self.gate.notify_all();
    }

    /// A version number never given before, for a version that is read but not published
    /// (a held transaction's, [`Store::in_txn`]).
    pub(crate) fn draw_seq(&self) -> Seq {
        self.ids.next_seq()
    }

    /// A transaction on the head, to be committed later by [`Store::commit`]: it shares the
    /// store's id allocator, so it may be kept past the call that began it.
    pub fn begin(&self, setup: &D::Setup) -> Txn<'static, D> {
        Txn::new(self.snapshot(), Ids::Shared(Arc::clone(&self.ids)), setup)
    }

    /// Commit `tx` as `caller`, as [`Store::run`] commits a prepared transaction: `None` on
    /// a conflict (nothing changed; the caller prepares again on a newer head).
    pub fn commit(&self, caller: &Caller, tx: Txn<'_, D>) -> Option<Committed<D, ()>> {
        let start = self.clock.monotonic();
        let done = self.try_commit(caller, tx.into_parts(), false, None).ok();
        if done.is_none() {
            self.counts.conflict();
        }
        done.map(|done| self.committed(done, (), 1, false, start))
    }

    /// [`Store::commit`] by the holder of the gate (a held transaction's), published as
    /// version `at` where given (a number drawn for the same content, never published).
    pub(crate) fn commit_gated(
        &self,
        caller: &Caller,
        tx: Txn<'_, D>,
        at: Option<Seq>,
        start: Duration,
    ) -> Option<Committed<D, ()>> {
        let done = self.try_commit(caller, tx.into_parts(), true, at).ok()?;
        Some(self.committed(done, (), 1, true, start))
    }

    /// The counters to save beside the bytes of `at`.
    pub fn counters(&self, at: &Snapshot<D>) -> Counters {
        Counters {
            version: at.seq(),
            last: self.ids.last_seq(),
            cursors: self.ids.cursors(),
        }
    }

    /// The store's counters.
    pub fn stats(&self) -> StoreStats {
        self.counts.read()
    }

    /// The steps undo can reach, oldest first.
    pub fn steps(&self) -> Vec<StepInfo<D::Key>> {
        self.lock_state().history.steps()
    }

    /// The steps plain redo can bring back, the next one last.
    pub fn redoable(&self) -> Vec<StepInfo<D::Key>> {
        self.lock_state().history.redoable()
    }

    /// The highest state number given ([`Version::number`]): the next step's is one more.
    pub fn highest(&self) -> u64 {
        self.lock_state().numbered
    }

    /// The highest state number given ([`Version::number`]), the steps undo can reach
    /// (oldest first) and those plain redo can bring back (the next one last), read at once.
    pub fn listed(&self) -> Listed<D::Key> {
        let state = self.lock_state();
        Listed {
            highest: state.numbered,
            steps: state.history.steps(),
            redo: state.history.redoable(),
        }
    }

    /// The version numbered `number` ([`Version::number`]) the head reaches: the head, else
    /// the newest such state undo or redo can bring back (numbers repeat after an undo: the
    /// head-most one); `None` when none is kept.
    pub fn numbered(&self, number: u64) -> Option<Snapshot<D>> {
        let head = self.head.load();
        if head.number() == number {
            return Some(head);
        }
        self.lock_state().history.numbered(number)
    }

    /// The newest step `caller` made that undo can reach.
    pub fn latest_step_by(&self, caller: &str) -> Option<Seq> {
        self.lock_state().history.latest_by(caller)
    }

    /// Run `command`: validate and prepare on the head's snapshot, commit if nothing it
    /// read or wrote was written since, otherwise prepare again on the new head; after
    /// [`StoreConfig::optimistic_tries`] conflicts (after one, for a command that read the
    /// whole document), run it behind the gate.
    pub fn run<C: Command<D>>(&self, caller: &Caller, command: &C) -> Result<Committed<D, C::Out>, C::Error> {
        self.run_with(caller, command, &D::Setup::default())
    }

    /// [`Store::run`], each transaction's working version set up with `setup`
    /// ([`DocModel::set_up`]).
    pub fn run_with<C: Command<D>>(
        &self,
        caller: &Caller,
        command: &C,
        setup: &D::Setup,
    ) -> Result<Committed<D, C::Out>, C::Error> {
        let begin = |base: Snapshot<D>| Txn::new(base, Ids::Borrowed(&self.ids), setup);
        self.attempts(caller, begin, |tx| {
            command.validate(tx.base().doc())?;
            command.prepare(tx)
        })
    }

    /// [`Store::run_with`] of a command given as its `prepare` alone (it validates what it
    /// needs as it goes), handed a transaction that shares the store's id allocator, as
    /// [`Store::begin`]'s does: one it may pass on, erased, to code that names the model (a
    /// host's batch hands it to each op's format). Its versions ([`Txn::version_here`]) are
    /// numbered as its commit is expected to be: the store's next number as it is prepared.
    ///
    /// # Errors
    /// What `prepare` returned (nothing published; a refusal is not prepared again).
    pub fn run_txn<O, E: From<Error<D>>>(
        &self,
        caller: &Caller,
        setup: &D::Setup,
        prepare: impl Fn(&mut Txn<'static, D>) -> Result<O, E>,
    ) -> Result<Committed<D, O>, E> {
        let begin = |base: Snapshot<D>| {
            let next = self.lock_state().numbered.saturating_add(1);
            Txn::new(base, Ids::Shared(Arc::clone(&self.ids)), setup).numbered(next)
        };
        self.attempts(caller, begin, prepare)
    }

    /// Prepare (`prepare`, on a transaction `begin` makes on the head's snapshot) and commit
    /// if nothing it read or wrote was written since, otherwise prepare again on the new head;
    /// after [`StoreConfig::optimistic_tries`] conflicts (after one, for a transaction that
    /// read the whole document), prepare and commit behind the gate, so that no other commit
    /// comes between: prepared on the head outside the commit lock, as every command is (an
    /// undo meanwhile, which the gate lets through, only has it prepared again).
    ///
    /// A conflict with another editor's commit ([`Caller::foreign`]) that changed what the
    /// transaction depends on is not prepared again: it is refused as
    /// [`StoreError::ForeignChange`], so a person's edit is never overwritten by a blind
    /// re-run (co-authoring design: the conflict goes back to the agent).
    fn attempts<'t, O, E: From<Error<D>>>(
        &self,
        caller: &Caller,
        begin: impl Fn(Snapshot<D>) -> Txn<'t, D>,
        prepare: impl Fn(&mut Txn<'t, D>) -> Result<O, E>,
    ) -> Result<Committed<D, O>, E> {
        let start = self.clock.monotonic();
        let mut attempts = 0;
        while attempts < self.config.optimistic_tries {
            attempts += 1;
            let mut tx = begin(self.snapshot());
            let out = prepare(&mut tx)?;
            let whole = tx.reads().keys().is_none();
            match self.try_commit(caller, tx.into_parts(), false, None) {
                Ok(done) => return Ok(self.committed(done, out, attempts, false, start)),
                Err(Conflict {
                    foreign: Some((step, editor, overlap)),
                }) => {
                    self.counts.conflict();
                    return Err(StoreError::ForeignChange { step, editor, overlap }.into());
                }
                Err(Conflict { foreign: None }) => {}
            }
            self.counts.conflict();
            if whole {
                // it read the whole document: any commit since conflicts with it again, so
                // it is prepared again behind the gate at once
                break;
            }
        }
        self.counts.exclusive();
        let asked = self.clock.monotonic();
        let _gate = self.enter_gate();
        let closing = self.clock.monotonic().saturating_sub(asked);
        loop {
            attempts += 1;
            let mut tx = begin(self.snapshot()).behind_gate();
            let out = prepare(&mut tx)?;
            match self.try_commit(caller, tx.into_parts(), true, None) {
                Ok(mut done) => {
                    done.waited += closing;
                    return Ok(self.committed(done, out, attempts, true, start));
                }
                Err(Conflict {
                    foreign: Some((step, editor, overlap)),
                }) => {
                    self.counts.conflict();
                    return Err(StoreError::ForeignChange { step, editor, overlap }.into());
                }
                Err(Conflict { foreign: None }) => {}
            }
            self.counts.conflict();
        }
    }

    /// The commit: `None` on a conflict. What was written since the transaction's snapshot
    /// is checked, and its changes rebased and replayed onto the head, outside the commit
    /// lock; under it, only that the head is still the one replayed onto, and the publish.
    /// Where the head moved meanwhile it is checked again from the newer head; after
    /// [`MOVED_TRIES`] such moves, under the lock. `gated`: the caller holds the gate; `at`:
    /// the number to publish under (drawn already, for this content), else the next.
    fn try_commit(
        &self,
        caller: &Caller,
        mut parts: TxnParts<D>,
        gated: bool,
        at: Option<Seq>,
    ) -> Result<Done<D>, Conflict<D>> {
        if parts.changes.is_empty() {
            return Ok(Done {
                version: parts.base,
                previous: None,
                changed: false,
                writes: KeySet::new(),
                held: Duration::ZERO,
                waited: Duration::ZERO,
            });
        }
        // a key written only as a merge (made where the change is applied), or a derived one
        // (`DocModel::derived`), is no conflict with another writer of it; the keys it creates
        // under stand-ins are kept apart (`parts.created`) and none with anything: no other
        // transaction can have read them, and its own reads of them are of its own work
        let mut depends = parts.reads.without(&parts.created);
        depends.extend(&parts.writes.without(&parts.merges).without_where(D::derived));
        let reads = history_reads(parts.reads.without(&parts.created), &parts.predicates);
        let changes: Arc<[D::Change]> = std::mem::take(&mut parts.changes).into();
        let mut moved = 0;
        let mut waited = Duration::ZERO;
        let mut queue = Queue {
            store: self,
            queued: false,
        };
        loop {
            let last = moved >= MOVED_TRIES;
            let mut early = None;
            if !last {
                let head = self.head.load();
                let since = if Arc::ptr_eq(&head, &parts.base) {
                    Some((head, Since::none()))
                } else {
                    // the head and the entries that led to it, at once
                    let state = self.lock_state();
                    waited += state.waited;
                    let head = self.head.load();
                    state.journal.since_base(parts.base.seq()).map(|since| (head, since))
                };
                let (head, since) = since.ok_or(Conflict { foreign: None })?;
                early = Some((
                    Arc::clone(&head),
                    self.onto((&parts, &changes), &depends, &head, &since)?,
                ));
            }
            let mut state = self.lock_ungated(gated, &mut queue);
            waited += state.waited;
            let entered = self.clock.monotonic();
            let head = self.head.load();
            let (doc, changes) = match early {
                Some((onto, planned)) if Arc::ptr_eq(&onto, &head) => planned,
                Some(_) => {
                    // published to meanwhile: checked again from the newer head
                    drop(state);
                    moved += 1;
                    continue;
                }
                None => {
                    let since = if Arc::ptr_eq(&head, &parts.base) {
                        Since::none()
                    } else {
                        let since = state.journal.since_base(parts.base.seq());
                        since.ok_or(Conflict { foreign: None })?
                    };
                    self.onto((&parts, &changes), &depends, &head, &since)?
                }
            };
            let body = Body {
                changes,
                reads,
                writes: resolved_writes(&doc, parts.writes, &parts.created),
                caller: caller.clone(),
            };
            let (version, writes, garbage) = self.publish(&mut state, Arc::clone(&head), doc, body, at);
            let held = self.clock.monotonic().saturating_sub(entered);
            queue.leave(&mut state);
            drop(state);
            drop(garbage);
            return Ok(Done {
                version,
                previous: Some(head),
                changed: true,
                writes,
                held,
                waited,
            });
        }
    }

    /// The transaction's work as it applies to `head`, `since` the entries published after
    /// its snapshot up to `head`: its own when nothing was published since, else its changes
    /// rebased past theirs and replayed onto `head`; `None` when it conflicts -- a key it
    /// depends on written since, a key written since that meets a condition it read by, or
    /// a change that cannot be rebased or applied.
    fn onto(
        &self,
        work: (&TxnParts<D>, &Arc<[D::Change]>),
        depends: &KeySet<D::Key>,
        head: &Snapshot<D>,
        since: &Since<D>,
    ) -> Result<Onto<D>, Conflict<D>> {
        self.onto_head(work, depends, head, since)
            .ok_or_else(|| Conflict::of(since, depends, work.0, head))
    }

    /// [`Store::onto`], `None` on a conflict.
    fn onto_head(
        &self,
        (parts, changes): (&TxnParts<D>, &Arc<[D::Change]>),
        depends: &KeySet<D::Key>,
        head: &Snapshot<D>,
        since: &Since<D>,
    ) -> Option<Onto<D>> {
        if Arc::ptr_eq(head, &parts.base) {
            return Some((parts.work.clone(), Arc::clone(changes)));
        }
        if head.written_since(parts.base.seq(), depends) {
            return None;
        }
        if !parts.predicates.is_empty() {
            // read by condition: a key written since that meets it on the head conflicts
            let written = since.written();
            let keys = written.keys()?;
            if keys
                .iter()
                .any(|key| parts.predicates.iter().any(|p| p.depends(head.doc(), key)))
            {
                return None;
            }
        }
        // Disjoint from everything committed since: the changes, rebased past those
        // commits' (positions moved past theirs), apply to the head. A change that cannot be
        // rebased or applied after all is a conflict: prepared again.
        let rebased: Arc<[D::Change]> = since.rebase(changes.to_vec())?.into();
        let doc = replay(head.doc(), &rebased).ok()?;
        self.counts.rebased();
        Some((doc, rebased))
    }

    /// Publish `doc` as the next version after `before` -- numbered `at` where given (drawn
    /// for it already), else the next number -- and record the step in the history.
    fn publish(
        &self,
        state: &mut State<D>,
        before: Snapshot<D>,
        doc: D,
        body: Body<D>,
        at: Option<Seq>,
    ) -> (Snapshot<D>, KeySet<D::Key>, Garbage<D>) {
        // (a number drawn already is never above the highest given)
        let seq = at
            .filter(|at| *at > before.seq())
            .unwrap_or_else(|| self.ids.next_seq());
        state.numbered += 1;
        let number = state.numbered;
        doc.settle();
        let version = Arc::new(before.succeed(seq, number, doc, &body.writes));
        self.head.store(Arc::clone(&version));
        let changes = ChangeSet::made(body.changes);
        let writes = body.writes.clone();
        let foreign = body.caller.foreign_name();
        let step = Step {
            before,
            after: Arc::clone(&version),
            changes: changes.clone(),
            reads: body.reads,
            writes: body.writes,
            caller: body.caller.name,
            foreign: body.caller.foreign,
        };
        let garbage = state.history.commit(step);
        self.journal(state, seq, changes, writes.clone(), foreign);
        (version, writes, garbage)
    }

    fn committed<O>(&self, done: Done<D>, out: O, attempts: u32, exclusive: bool, start: Duration) -> Committed<D, O> {
        if done.changed {
            self.counts.committed(done.held);
        }
        let total = self.clock.monotonic().saturating_sub(start);
        Committed {
            version: done.version,
            previous: done.previous,
            out,
            changed: done.changed,
            writes: done.writes,
            attempts,
            exclusive,
            timing: CommitTiming {
                held: done.held,
                waited: done.waited,
                total,
            },
        }
    }

    /// Undo the newest step undo can reach (a scoped undo may have taken out newer ones'
    /// predecessors; this is the newest left).
    pub fn undo(&self) -> Result<HistoryOutcome<D>, Error<D>> {
        self.history_op(|history, head, journal| history.plan_undo(None, &KeySet::new(), head, journal))
    }

    /// Redo the newest step plain undo took back.
    pub fn redo(&self) -> Result<HistoryOutcome<D>, Error<D>> {
        self.history_op(|history, head, journal| history.plan_redo(None, head, journal))
    }

    /// Undo the newest step when it is the one that made version `step`: a check and the
    /// undo in one, so no commit lands between them. Refused as
    /// [`crate::StoreError::NotNext`], changing nothing, when another step is newest.
    pub fn undo_newest(&self, step: Seq) -> Result<HistoryOutcome<D>, Error<D>> {
        self.history_op(|history, head, journal| {
            history.newest_is(step)?;
            history.plan_undo(None, &KeySet::new(), head, journal)
        })
    }

    /// Redo the step plain redo would bring back next when it is the one that made version
    /// `step` (as [`Store::undo_newest`]); [`crate::StoreError::NotNext`] otherwise.
    pub fn redo_next(&self, step: Seq) -> Result<HistoryOutcome<D>, Error<D>> {
        self.history_op(|history, head, journal| {
            history.next_redo_is(step)?;
            history.plan_redo(None, head, journal)
        })
    }

    /// Scoped undo: take back the step that made version `step`, wherever it is in the
    /// history, leaving the later steps in place. Refused as
    /// [`crate::StoreError::Entangled`], changing nothing, when a later step read or wrote
    /// a key it wrote, or when it wrote a key of `shared` (keys every scope depends on).
    pub fn undo_step(&self, step: Seq, shared: &KeySet<D::Key>) -> Result<HistoryOutcome<D>, Error<D>> {
        self.history_op(|history, head, journal| history.plan_undo(Some(step), shared, head, journal))
    }

    /// Bring back the step that made version `step`, taken back by an undo, as a new
    /// step, named by the version it now makes. Refused as
    /// [`crate::StoreError::Entangled`] when what it read or wrote holds other content than
    /// its undo left.
    pub fn redo_step(&self, step: Seq) -> Result<HistoryOutcome<D>, Error<D>> {
        self.history_op(|history, head, journal| history.plan_redo(Some(step), head, journal))
    }

    fn history_op(
        &self,
        plan: impl FnOnce(&History<D>, &Snapshot<D>, &Journal<D>) -> Result<Planned<D>, Error<D>>,
    ) -> Result<HistoryOutcome<D>, Error<D>> {
        let mut state = self.lock_state();
        if state.held {
            return Err(StoreError::GroupOpen);
        }
        let entered = self.clock.monotonic();
        let head = self.head.load();
        let mut planned = plan(&state.history, &head, &state.journal)?;
        let seq = self.ids.next_seq();
        planned.doc.carry(head.doc());
        planned.doc.settle();
        let number = planned.number.unwrap_or_else(|| {
            state.numbered += 1;
            state.numbered
        });
        let version = Arc::new(head.succeed(seq, number, planned.doc, &planned.writes));
        let previous = Arc::clone(&head);
        self.head.store(Arc::clone(&version));
        let changes = planned.changes;
        let garbage = state
            .history
            .record(planned.target, head, Arc::clone(&version), changes.clone());
        self.journal(&mut state, seq, changes, planned.writes.clone(), None);
        let held = self.clock.monotonic().saturating_sub(entered);
        drop(state);
        drop(garbage);
        self.counts.committed(held);
        Ok(HistoryOutcome {
            version,
            previous,
            step: planned.step,
            writes: planned.writes,
        })
    }
}

impl<D: DocModel> Store<D> {
    /// Keep the change set that made version `seq`: within the rebase window, and back to
    /// the oldest version the history may still move a change set from.
    fn journal(
        &self,
        state: &mut State<D>,
        seq: Seq,
        changes: ChangeSet<D>,
        writes: KeySet<D::Key>,
        foreign: Option<Arc<str>>,
    ) {
        state.journal.push(seq, changes, writes, foreign);
        let needed = state.history.oldest_reach();
        state.journal.trim(self.config.rebase_window, needed);
    }
}

/// The reads a step keeps in the history: a read by condition counts there as a read of the
/// keys it names ([`ReadPredicate::history_keys`]), or of every key when it names none, so
/// undo and redo find a step entangled at least as they did before such reads (a condition
/// is checked against the head only at commit).
fn history_reads<D: DocModel>(mut reads: KeySet<D::Key>, predicates: &[Arc<dyn ReadPredicate<D>>]) -> KeySet<D::Key> {
    for predicate in predicates {
        match predicate.history_keys() {
            Some(keys) => reads.extend(&KeySet::of(keys)),
            None => return KeySet::Everything,
        }
    }
    reads
}

/// The keys a commit wrote, as `doc` (the version it made) has them: its written keys and
/// the keys it created under stand-ins, resolved ([`DocModel::resolve`]).
fn resolved_writes<D: DocModel>(doc: &D, mut writes: KeySet<D::Key>, created: &KeySet<D::Key>) -> KeySet<D::Key> {
    if !created.is_empty() {
        writes.extend(&doc.resolve(created));
    }
    writes
}

/// `changes` applied in order to `doc`, as a transaction of its own ([`DocModel::begin`]).
fn replay<D: DocModel>(doc: &D, changes: &[D::Change]) -> Result<D, D::Error> {
    let mut doc = doc.clone();
    doc.begin();
    for change in changes {
        doc.apply(change)?;
    }
    Ok(doc)
}
