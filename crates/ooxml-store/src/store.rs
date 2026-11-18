//! Owns: [`Store`], one document's versions: the lock-free head readers load, the commit
//! (conflict detection by the keys a transaction read and wrote, redo, then a run under the
//! commit lock), history operations, the id allocator and the reopen state.
//!
//! Must not: run anything inside the commit lock beyond the check, the rebase and replay of
//! a change set on a moved head and the publish, except a transaction that conflicted
//! [`StoreConfig::optimistic_tries`] times (design section 5); free old versions inside it.

use crate::history::{Garbage, History, Planned, Step};
use crate::ids::IdAllocator;
use crate::stats::Counts;
use crate::sync::{lock, HeadCell, Mutex};
use crate::txn::TxnParts;
use crate::{
    Clock, Command, Counters, CountersError, DocModel, Error, HistoryOutcome, KeySet, Seq, Snapshot, StepInfo,
    StoreStats, Txn, Version,
};
use std::collections::VecDeque;
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

/// The latest versions published, oldest first: each one's number and the change set that
/// made it from the one before (`None` for an undo or a redo, which cannot be rebased past).
type Log<D> = VecDeque<(Seq, Option<Arc<[<D as DocModel>::Change]>>)>;

/// The state under the commit lock.
struct State<D: DocModel> {
    last: Seq,
    history: History<D>,
    /// The latest versions published, within the rebase window.
    log: Log<D>,
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
                history: History::new(config.history_depth),
                log: VecDeque::new(),
            }),
            ids: IdAllocator::new(counters.cursors),
            config,
            clock,
            counts: Counts::new(),
        }
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

    /// The newest step `caller` made that undo can reach.
    pub fn latest_step_by(&self, caller: &str) -> Option<Seq> {
        lock(&self.state).history.latest_by(caller)
    }

    /// Run `command`: validate and prepare on the head's snapshot, commit if nothing it
    /// read or wrote was written since, otherwise prepare again on the new head; after
    /// [`StoreConfig::optimistic_tries`] conflicts, run it under the commit lock.
    pub fn run<C: Command<D>>(&self, caller: &Caller, command: &C) -> Result<Committed<D, C::Out>, C::Error> {
        let start = self.clock.monotonic();
        let mut attempts = 0;
        while attempts < self.config.optimistic_tries {
            attempts += 1;
            let base = self.snapshot();
            command.validate(base.doc())?;
            let mut tx = Txn::new(base, &self.ids);
            let out = command.prepare(&mut tx)?;
            if let Some(done) = self.try_commit(caller, tx.into_parts()) {
                return Ok(self.committed(done, out, attempts, false, start));
            }
            self.counts.conflict();
        }
        self.run_exclusive(caller, command, attempts + 1, start)
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
        // a key written only as a merge (made where the change is applied) is no conflict
        // with another writer of it
        let mut depends = parts.reads.clone();
        depends.extend(&parts.writes.without(&parts.merges));
        let mut state = lock(&self.state);
        let entered = self.clock.monotonic();
        let head = self.head.load();
        if head.written_since(parts.base.seq(), &depends) {
            return None;
        }
        let (doc, changes) = if Arc::ptr_eq(&head, &parts.base) {
            (parts.work, parts.changes.into())
        } else {
            // Disjoint from everything committed since: the changes, rebased past those
            // commits' (positions moved past theirs), apply to the head. A change that
            // cannot be rebased or applied after all is a conflict: prepared again.
            let rebased: Arc<[D::Change]> = rebase::<D>(&state.log, parts.base.seq(), parts.changes)?.into();
            let doc = replay(head.doc(), &rebased).ok()?;
            self.counts.rebased();
            (doc, rebased)
        };
        let body = Body {
            changes,
            reads: parts.reads,
            writes: parts.writes,
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
        attempts: u32,
        start: Duration,
    ) -> Result<Committed<D, C::Out>, C::Error> {
        self.counts.exclusive();
        let mut state = lock(&self.state);
        let entered = self.clock.monotonic();
        let base = self.head.load();
        command.validate(base.doc())?;
        let mut tx = Txn::new(Arc::clone(&base), &self.ids);
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
            reads: parts.reads,
            writes: parts.writes,
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
        let version = Arc::new(before.succeed(seq, doc, &body.writes));
        self.head.store(Arc::clone(&version));
        self.log(state, seq, Some(Arc::clone(&body.changes)));
        let writes = body.writes.clone();
        let step = Step {
            before,
            after: Arc::clone(&version),
            changes: body.changes,
            reads: body.reads,
            writes: body.writes,
            caller: body.caller,
        };
        let garbage = state.history.commit(step);
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
        self.history_op(|history, head| history.plan_undo(None, &KeySet::new(), head))
    }

    /// Redo the newest step plain undo took back.
    pub fn redo(&self) -> Result<HistoryOutcome<D>, Error<D>> {
        self.history_op(|history, head| history.plan_redo(None, head))
    }

    /// Scoped undo: take back the step that made version `step`, wherever it is in the
    /// history, leaving the later steps in place. Refused as
    /// [`crate::StoreError::Entangled`], changing nothing, when a later step read or wrote
    /// a key it wrote, or when it wrote a key of `shared` (keys every scope depends on).
    pub fn undo_step(&self, step: Seq, shared: &KeySet<D::Key>) -> Result<HistoryOutcome<D>, Error<D>> {
        self.history_op(|history, head| history.plan_undo(Some(step), shared, head))
    }

    /// Bring back the step that made version `step`, taken back by an undo, as a new
    /// step, named by the version it now makes. Refused as
    /// [`crate::StoreError::Entangled`] when what it read or wrote holds other content than
    /// its undo left.
    pub fn redo_step(&self, step: Seq) -> Result<HistoryOutcome<D>, Error<D>> {
        self.history_op(|history, head| history.plan_redo(Some(step), head))
    }

    fn history_op(
        &self,
        plan: impl FnOnce(&History<D>, &Snapshot<D>) -> Result<Planned<D>, Error<D>>,
    ) -> Result<HistoryOutcome<D>, Error<D>> {
        let mut state = lock(&self.state);
        let entered = self.clock.monotonic();
        let head = self.head.load();
        let planned = plan(&state.history, &head)?;
        let seq = state.last.next();
        state.last = seq;
        let version = Arc::new(head.succeed(seq, planned.doc, &planned.writes));
        self.head.store(Arc::clone(&version));
        self.log(&mut state, seq, None);
        let garbage = state.history.record(planned.target, head, Arc::clone(&version));
        let held = self.clock.monotonic().saturating_sub(entered);
        drop(state);
        drop(garbage);
        self.counts.committed(held);
        Ok(HistoryOutcome {
            version,
            step: planned.step,
            writes: planned.writes,
        })
    }
}

impl<D: DocModel> Store<D> {
    /// Keep the change set that made version `seq`, within the rebase window.
    fn log(&self, state: &mut State<D>, seq: Seq, changes: Option<Arc<[D::Change]>>) {
        state.log.push_back((seq, changes));
        while state.log.len() > self.config.rebase_window {
            state.log.pop_front();
        }
    }
}

/// `changes`, made on version `base`, rebased past every change set published since, in
/// order; `None` when one of them is not in the log (too old, or an undo or redo) or a
/// change cannot be moved past one of theirs.
fn rebase<D: DocModel>(log: &Log<D>, base: Seq, mut changes: Vec<D::Change>) -> Option<Vec<D::Change>> {
    let since = log.iter().position(|(seq, _)| *seq > base)?;
    if since == 0 && log.front().is_some_and(|(seq, _)| seq.0 != base.0 + 1) {
        return None; // the commits right after the snapshot are no longer kept
    }
    for (_, past) in log.iter().skip(since) {
        for past in past.as_deref()? {
            for change in &mut changes {
                *change = D::rebase(change, past).ok()?;
            }
        }
    }
    Some(changes)
}

/// `changes` applied in order to `doc`.
fn replay<D: DocModel>(doc: &D, changes: &[D::Change]) -> Result<D, D::Error> {
    let mut doc = doc.clone();
    for change in changes {
        doc.apply(change)?;
    }
    Ok(doc)
}
