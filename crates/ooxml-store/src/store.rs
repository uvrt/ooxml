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
//! a state that never was, a new number. A reopen starts at 0 again. A step group is one
//! step and one number: its commits all carry one more than the highest number given when
//! it opened, which is given for good only when it closes having made a step; a revert that
//! takes all its commits back brings the number it opened on back.
//!
//! Must not: run anything inside the commit lock beyond the check, the rebase and replay of
//! a change set on a moved head and the publish, except a transaction that conflicted
//! [`StoreConfig::optimistic_tries`] times (design section 5); free old versions inside it.

use crate::changeset::ChangeSet;
use crate::history::{Garbage, GroupMark, History, Planned, Step};
use crate::ids::IdAllocator;
use crate::journal::Journal;
use crate::stats::Counts;
use crate::sync::{lock, HeadCell, Mutex};
use crate::txn::TxnParts;
use crate::{
    Clock, Command, Counters, CountersError, DocModel, Error, HistoryOutcome, KeySet, ReadPredicate, Seq, Snapshot,
    StepInfo, StoreError, StoreStats, Txn, Version,
};
use std::sync::Arc;
use std::time::Duration;

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
            rebase_window: 64,
        }
    }
}

/// Who runs a command; kept with its history step (scoped undo by writer).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Caller(Option<Arc<str>>);

impl Caller {
    /// No name.
    pub fn anonymous() -> Self {
        Self(None)
    }

    /// A named caller.
    pub fn named(name: &str) -> Self {
        Self(Some(Arc::from(name)))
    }

    /// The name.
    pub fn name(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

/// Where a commit's time went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommitTiming {
    /// Inside the commit lock, for the attempt that committed.
    pub held: Duration,
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
    /// Whether it ran under the commit lock after conflicts.
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
}

/// A committed transaction's record, before it has versions around it.
struct Body<D: DocModel> {
    changes: Arc<[D::Change]>,
    reads: KeySet<D::Key>,
    writes: KeySet<D::Key>,
    caller: Option<Arc<str>>,
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
struct State<D: DocModel> {
    last: Seq,
    /// The highest state number given ([`Version::number`]).
    numbered: u64,
    history: History<D>,
    /// The latest versions' change sets: within the rebase window, and back to the oldest
    /// version the history may still move a change set from.
    journal: Journal<D>,
}

/// One document's versions. Readers load the head without a lock; writers prepare in
/// parallel and commit in a short critical section.
pub struct Store<D: DocModel> {
    head: HeadCell<Version<D>>,
    state: Mutex<State<D>>,
    ids: IdAllocator,
    config: StoreConfig,
    clock: Arc<dyn Clock>,
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
                last: counters.last,
                numbered: 0,
                history: History::new(config.history_depth),
                journal: Journal::new(),
            }),
            ids: IdAllocator::new(counters.cursors),
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
    /// store made a step or a group is open.
    pub fn continue_numbers(&self, state: u64, highest: u64) -> Result<(), CountersError> {
        if state > highest {
            return Err(CountersError::StateAboveHighest { state, highest });
        }
        let mut locked = lock(&self.state);
        let head = self.head.load();
        if locked.numbered != 0 || head.number() != 0 || locked.history.grouped() || !locked.history.steps().is_empty()
        {
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

    /// The counters to save beside the bytes of `at`.
    pub fn counters(&self, at: &Snapshot<D>) -> Counters {
        let last = lock(&self.state).last;
        Counters {
            version: at.seq(),
            last,
            cursors: self.ids.cursors(),
        }
    }

    /// The store's counters.
    pub fn stats(&self) -> StoreStats {
        self.counts.read()
    }

    /// The steps undo can reach, oldest first.
    pub fn steps(&self) -> Vec<StepInfo<D::Key>> {
        lock(&self.state).history.steps()
    }

    /// The steps plain redo can bring back, the next one last.
    pub fn redoable(&self) -> Vec<StepInfo<D::Key>> {
        lock(&self.state).history.redoable()
    }

    /// The highest state number given ([`Version::number`]): the next step's is one more.
    pub fn highest(&self) -> u64 {
        lock(&self.state).numbered
    }

    /// The highest state number given ([`Version::number`]), the steps undo can reach
    /// (oldest first) and those plain redo can bring back (the next one last), read at once.
    pub fn listed(&self) -> Listed<D::Key> {
        let state = lock(&self.state);
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
        lock(&self.state).history.numbered(number)
    }

    /// The newest step `caller` made that undo can reach.
    pub fn latest_step_by(&self, caller: &str) -> Option<Seq> {
        lock(&self.state).history.latest_by(caller)
    }

    /// Run `command`: validate and prepare on the head's snapshot, commit if nothing it
    /// read or wrote was written since, otherwise prepare again on the new head; after
    /// [`StoreConfig::optimistic_tries`] conflicts (after one, for a command that read the
    /// whole document), run it under the commit lock.
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
        let start = self.clock.monotonic();
        let mut attempts = 0;
        while attempts < self.config.optimistic_tries {
            attempts += 1;
            let base = self.snapshot();
            command.validate(base.doc())?;
            let mut tx = Txn::new(base, &self.ids, setup);
            let out = command.prepare(&mut tx)?;
            let whole = tx.reads().keys().is_none();
            if let Some(done) = self.try_commit(caller, tx.into_parts()) {
                return Ok(self.committed(done, out, attempts, false, start));
            }
            self.counts.conflict();
            if whole {
                // it read the whole document: any commit since conflicts with it again, so
                // it is prepared again under the commit lock at once
                break;
            }
        }
        self.run_exclusive(caller, command, setup, attempts + 1, start)
    }

    /// The commit section: `None` on a conflict.
    fn try_commit(&self, caller: &Caller, parts: TxnParts<D>) -> Option<Done<D>> {
        if parts.changes.is_empty() {
            return Some(Done {
                version: parts.base,
                previous: None,
                changed: false,
                writes: KeySet::new(),
                held: Duration::ZERO,
            });
        }
        // a key written only as a merge (made where the change is applied), or a derived one
        // (`DocModel::derived`), is no conflict with another writer of it; the keys it creates
        // under stand-ins are kept apart (`parts.created`) and none with anything: no other
        // transaction can have read them, and its own reads of them are of its own work
        let mut depends = parts.reads.without(&parts.created);
        depends.extend(&parts.writes.without(&parts.merges).without_where(D::derived));
        let mut state = lock(&self.state);
        let entered = self.clock.monotonic();
        let head = self.head.load();
        if head.written_since(parts.base.seq(), &depends) {
            return None;
        }
        if !parts.predicates.is_empty() && head.seq() > parts.base.seq() {
            // read by condition: a key written since that meets it on the head conflicts
            let written = state.journal.written_since(parts.base.seq())?;
            let keys = written.keys()?;
            if keys
                .iter()
                .any(|key| parts.predicates.iter().any(|p| p.depends(head.doc(), key)))
            {
                return None;
            }
        }
        let (doc, changes) = if Arc::ptr_eq(&head, &parts.base) {
            (parts.work, parts.changes.into())
        } else {
            // Disjoint from everything committed since: the changes, rebased past those
            // commits' (positions moved past theirs), apply to the head. A change that
            // cannot be rebased or applied after all is a conflict: prepared again.
            let rebased: Arc<[D::Change]> = state.journal.rebase(parts.base.seq(), parts.changes)?.into();
            let doc = replay(head.doc(), &rebased).ok()?;
            self.counts.rebased();
            (doc, rebased)
        };
        let body = Body {
            changes,
            reads: history_reads(parts.reads.without(&parts.created), &parts.predicates),
            writes: resolved_writes(&doc, parts.writes, &parts.created),
            caller: caller.0.clone(),
        };
        let (version, writes, garbage) = self.publish(&mut state, Arc::clone(&head), doc, body);
        let held = self.clock.monotonic().saturating_sub(entered);
        drop(state);
        drop(garbage);
        Some(Done {
            version,
            previous: Some(head),
            changed: true,
            writes,
            held,
        })
    }

    /// Prepare and commit under the commit lock, so nothing can come between.
    fn run_exclusive<C: Command<D>>(
        &self,
        caller: &Caller,
        command: &C,
        setup: &D::Setup,
        attempts: u32,
        start: Duration,
    ) -> Result<Committed<D, C::Out>, C::Error> {
        self.counts.exclusive();
        let mut state = lock(&self.state);
        let entered = self.clock.monotonic();
        let base = self.head.load();
        command.validate(base.doc())?;
        let mut tx = Txn::new(Arc::clone(&base), &self.ids, setup);
        let out = command.prepare(&mut tx)?;
        let parts = tx.into_parts();
        if parts.changes.is_empty() {
            drop(state);
            let done = Done {
                version: base,
                previous: None,
                changed: false,
                writes: KeySet::new(),
                held: Duration::ZERO,
            };
            return Ok(self.committed(done, out, attempts, true, start));
        }
        let body = Body {
            changes: parts.changes.into(),
            reads: history_reads(parts.reads.without(&parts.created), &parts.predicates),
            writes: resolved_writes(&parts.work, parts.writes, &parts.created),
            caller: caller.0.clone(),
        };
        let (version, writes, garbage) = self.publish(&mut state, Arc::clone(&base), parts.work, body);
        let held = self.clock.monotonic().saturating_sub(entered);
        drop(state);
        drop(garbage);
        Ok(self.committed(
            Done {
                version,
                previous: Some(base),
                changed: true,
                writes,
                held,
            },
            out,
            attempts,
            true,
            start,
        ))
    }

    /// Publish `doc` as the next version after `before` and record the step in the history.
    fn publish(
        &self,
        state: &mut State<D>,
        before: Snapshot<D>,
        doc: D,
        body: Body<D>,
    ) -> (Snapshot<D>, KeySet<D::Key>, Garbage<D>) {
        let seq = state.last.next();
        state.last = seq;
        // within a group, the group's number: given for good when it closes
        let number = state.history.group_number().unwrap_or_else(|| {
            state.numbered += 1;
            state.numbered
        });
        doc.settle();
        let version = Arc::new(before.succeed(seq, number, doc, &body.writes));
        self.head.store(Arc::clone(&version));
        let changes = ChangeSet::made(body.changes);
        let writes = body.writes.clone();
        let step = Step {
            before,
            after: Arc::clone(&version),
            changes: changes.clone(),
            reads: body.reads,
            writes: body.writes,
            caller: body.caller,
        };
        let garbage = state.history.commit(step);
        self.journal(state, seq, changes, writes.clone());
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
            timing: CommitTiming { held: done.held, total },
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
        let mut state = lock(&self.state);
        if state.history.grouped() {
            return Err(StoreError::GroupOpen);
        }
        let entered = self.clock.monotonic();
        let head = self.head.load();
        let mut planned = plan(&state.history, &head, &state.journal)?;
        let seq = state.last.next();
        state.last = seq;
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
        self.journal(&mut state, seq, changes, planned.writes.clone());
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

/// Step groups: several commits as one history step (a host's batch of calls).
///
/// While a group is open its commits go on the undo list as they are made, plain redo is
/// held aside and the depth waits; [`Store::close_group`] makes them one step, and
/// [`Store::revert_group`] takes the ones after a mark back as a new version that no redo
/// brings back. A group opened within an open one joins it. The host holds the document
/// alone while a group is open: another writer's commit meanwhile would become part of the
/// group's step. Undo and redo are refused then.
impl<D: DocModel> Store<D> {
    /// Open a step group, or join the one open: a call's group within a batch's is part
    /// of the batch's step, and its close only ends the joining. Where the group is now (a
    /// joined group's revert to it takes back only what was committed since it joined).
    /// Cheap: the commit lock for a moment, no allocation.
    ///
    /// # Errors
    /// None today; the store's own, should a group refuse.
    pub fn open_group(&self) -> Result<GroupMark, Error<D>> {
        let mut state = lock(&self.state);
        let start = self.head.load().number();
        let number = state.numbered.saturating_add(1);
        state.history.open_group(start, number);
        state.history.group_mark().ok_or(StoreError::NoGroup)
    }

    /// Where the open group is now, for [`Store::revert_group`]; `None` without one.
    pub fn group_mark(&self) -> Option<GroupMark> {
        lock(&self.state).history.group_mark()
    }

    /// Take back the open group's commits after `mark`: the content before them published
    /// as a new version, which is returned (`None`: none was made since `mark`).
    ///
    /// # Errors
    /// [`StoreError::NoGroup`] without an open group (or with a mark of none of its).
    pub fn revert_group(&self, mark: GroupMark) -> Result<Option<Snapshot<D>>, Error<D>> {
        let mut state = lock(&self.state);
        let entered = self.clock.monotonic();
        let Some(revert) = state.history.plan_revert(mark)? else {
            return Ok(None);
        };
        let head = self.head.load();
        let seq = state.last.next();
        state.last = seq;
        let number = state.history.revert_number(mark).unwrap_or(head.number());
        let mut doc = revert.doc;
        doc.carry(head.doc());
        doc.settle();
        let version = Arc::new(head.succeed(seq, number, doc, &revert.writes));
        self.head.store(Arc::clone(&version));
        let garbage = state.history.reverted(mark);
        self.journal(&mut state, seq, revert.changes, revert.writes);
        let held = self.clock.monotonic().saturating_sub(entered);
        drop(state);
        drop(garbage);
        self.counts.committed(held);
        Ok(Some(version))
    }

    /// Close the open group: its commits one step of `caller`'s, whose version is returned
    /// (`None`: it made none, and redo is as it was when it opened). A joined group's close
    /// only ends the joining (`None`).
    ///
    /// # Errors
    /// [`StoreError::NoGroup`] without an open group.
    pub fn close_group(&self, caller: &Caller) -> Result<Option<Seq>, Error<D>> {
        let mut state = lock(&self.state);
        let head = self.head.load();
        let (made, garbage) = state.history.close_group(caller.0.clone(), &head)?;
        if let Some((_, number)) = made {
            state.numbered = state.numbered.max(number);
        }
        drop(state);
        drop(garbage);
        Ok(made.map(|(seq, _)| seq))
    }
}

impl<D: DocModel> Store<D> {
    /// Keep the change set that made version `seq`: within the rebase window, and back to
    /// the oldest version the history may still move a change set from.
    fn journal(&self, state: &mut State<D>, seq: Seq, changes: ChangeSet<D>, writes: KeySet<D::Key>) {
        state.journal.push(seq, changes, writes);
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
