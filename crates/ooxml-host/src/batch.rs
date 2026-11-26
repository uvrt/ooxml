//! Owns: `batch` in the host (design phase 3, section 4): the batch planned -- each op's
//! name read, its roles, the one acceptance (an op's errors fielded under its `arguments`),
//! the reads after its last change split off -- then run as one step: on one document,
//! optimistically in the document's store ([`ooxml_session::StoredDoc::run_batch`]: prepared
//! again from scratch on a conflict, behind the gate after the store's tries); on several, in
//! the transactions one owner holds open on each, in document order (decision D5). Each op
//! runs through the host's pipeline on the version the ops before it left; what it would keep
//! (its refs, its format's effects, its images) waits until the batch is published, and only
//! the attempt that published is kept. Then, once: the format's effects, the step, the refs,
//! the checks and what the batch touched; the reads after its last change on the version it
//! made; and the reply in today's words (ooxml-edit's `_run_partial` and `_run_batch`).
//!
//! Must not: keep anything of an attempt that did not publish, hold the session's stores while
//! an op runs, or run a session tool, `read_batch` or `undo` as an op.

use crate::catalogue::Catalogue;
use crate::pipeline::{caller_key, Going};
use crate::session::HostSession;
use ooxml_session::batch::{passed, seconds};
use ooxml_session::datasets::CallDatasets;
use ooxml_session::{
    AnyVersion, BatchOps, BatchRefused, CallContext, CallMade, CallRefs, Deferred, DocId, InBatch, OpRan, RefOwner,
    StepOwner,
};
use ooxml_store::{AttemptState, OpTxn, TxnOwner};
use ooxml_tools::roles::RoleSet;
use ooxml_tools::{Done, ErrorCode, Failure, OpEffects, Outcome, Refusal, ToolError, ToolResult, TryCall, WayOut};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// The tool's name.
const BATCH: &str = "batch";

/// The share of a batch's deadline a suggested batch size is to fill.
const FILL: f64 = 0.8;

/// One op of a batch as planned.
struct Planned {
    /// Its tool as the op gives it.
    given: String,
    /// Its tool as read.
    tool: String,
    /// Its arguments taken as meant (as given, where they were refused).
    arguments: Map<String, Value>,
    /// Its arguments as the op gives them.
    raw: Value,
    /// What was read as what.
    notes: Vec<String>,
    /// Why it cannot run (its field the op's own: `tool`, `arguments.x`).
    refused: Option<ToolError>,
    /// Whether it changes its document.
    changes: bool,
    /// The documents it names.
    docs: Vec<String>,
}

/// An op's failure: its error, and whether its field is the op's own already (a planning
/// refusal) or the tool's (fielded under `arguments` in the reply).
#[derive(Clone)]
struct Failed {
    error: ToolError,
    own: bool,
}

impl Failed {
    /// The error as the op's outcome names it (`arguments.x`).
    fn fielded(&self) -> ToolError {
        match (self.own, self.error.field()) {
            (false, Some(field)) => self.error.clone().at(format!("arguments.{field}")),
            _ => self.error.clone(),
        }
    }
}

/// What the ops of one attempt gathered.
#[derive(Default)]
struct Attempt {
    /// Each op's outcome as the reply lists it, by index.
    outcomes: BTreeMap<usize, Map<String, Value>>,
    /// The failed ops.
    errors: BTreeMap<usize, Failed>,
    /// The refs the ops defined so far, the failed ops' names.
    refs: Option<CallRefs>,
    /// What the applied ops said, merged.
    changed: Vec<String>,
    created: Vec<String>,
    removed: Vec<String>,
    renamed: Vec<(String, String)>,
    datasets: Vec<String>,
    warnings: Vec<String>,
    /// The images the applied ops show.
    images: usize,
    /// Each applied op's format effects, in op order.
    deferred: Vec<(usize, Vec<Deferred>)>,
    /// The datasets the ops named or defined (as one call's, today's batch's context): an
    /// op over the session's limit refused as it defines (`CallDatasets::define`), a failed
    /// op's taken back; kept once the batch is published.
    datasets_of: CallDatasets,
    /// What the applied ops made, by document: the first op's `before`, the last's version.
    made: BTreeMap<String, CallMade>,
    /// The first op the deadline stopped, and when.
    stopped: Option<(usize, f64)>,
}

/// A batch's ops as its documents' stores run them ([`BatchOps`]).
struct Run<'h> {
    host: &'h HostSession,
    planned: &'h [Planned],
    /// The ops before this run once each attempt (the reads after the last change after).
    split: usize,
    atomic: bool,
    caller: Option<&'h Map<String, Value>>,
    owner: Option<RefOwner>,
    /// When the batch began, and its deadline.
    started: Duration,
    timeout: Duration,
    attempt: Mutex<Attempt>,
    /// The documents the ops change, which run in a transaction.
    batched: Vec<String>,
    /// The values the application handed the call (written by reference; the rows of
    /// sources it resolved), for every op.
    values: ooxml_tools::written::CallValues,
}

impl HostSession {
    /// `batch` with `arguments` taken as meant.
    pub(crate) fn batch(
        &self,
        arguments: &Map<String, Value>,
        roles: RoleSet,
        caller: Option<&Map<String, Value>>,
        going: &mut Going,
    ) -> ToolResult<Value> {
        let ops: Vec<Value> = arguments
            .get("ops")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let atomic = arguments.get("atomic").and_then(Value::as_bool).unwrap_or(false);
        let planned: Vec<Planned> = ops.iter().map(|op| self.plan(op, roles)).collect::<Vec<_>>();
        let open = |doc: &String| DocId::parse(doc).is_some_and(|id| self.session.kind_of(&id).is_ok());
        let mut named: Vec<String> = Vec::new();
        for doc in planned.iter().flat_map(|p| &p.docs) {
            if !named.contains(doc) && open(doc) {
                named.push(doc.clone());
            }
        }
        going.docs.clone_from(&named);
        let primary = named.first().cloned();
        let settings = &self.host.settings;
        if ops.len() > settings.max_batch_ops {
            let error = ToolError::failure(
                Failure::Limit,
                format!("{} ops; at most {} per batch", ops.len(), settings.max_batch_ops),
            )
            .at("ops");
            return self.at_primary(ToolResult::failed(error), primary.as_deref());
        }
        let notes: Vec<String> = planned
            .iter()
            .enumerate()
            .flat_map(|(i, p)| p.notes.iter().map(move |n| format!("ops[{i}]: {n}")))
            .collect();
        if let Some(error) = atomic.then(|| planning_refusal(&planned)).flatten() {
            // (refused before any op ran: what was read as what is not said)
            return self.at_primary(ToolResult::failed(error), primary.as_deref());
        }
        // the reads after the last change run once it is published
        let split = planned
            .iter()
            .rposition(|p| p.refused.is_none() && p.changes)
            .map_or(0, |last| last + 1);
        let mut batched: Vec<String> = Vec::new();
        for doc in planned.iter().take(split).filter(|p| p.changes).flat_map(|p| &p.docs) {
            if !batched.contains(doc) && open(doc) {
                batched.push(doc.clone());
            }
        }
        batched.sort_by_key(|doc| DocId::parse(doc));
        let clock = &self.session.context().clock;
        let run = Run {
            host: self,
            planned: &planned,
            split,
            atomic,
            caller,
            owner: caller.map(|c| Arc::from(caller_key(c).as_str())),
            started: clock.monotonic(),
            timeout: settings.batch_timeout,
            attempt: Mutex::new(Attempt::default()),
            batched: batched.clone(),
            values: going.values.clone(),
        };
        let published = self.run_ops(&run, &batched);
        let mut attempt = std::mem::take(&mut *run.lock());
        let published = match published {
            Ok(published) => published,
            Err(Stop::Refused(index)) => {
                let mut result = ToolResult::failed(atomic_refusal(&planned, &attempt, index));
                result.warnings = notes;
                return self.at_primary(result, primary.as_deref());
            }
            Err(Stop::Store(error)) => return ToolResult::failed(error),
        };
        for at in published.values() {
            going.lock.held += at.lock.held;
            going.lock.waited += at.lock.waited;
        }
        let result = self.published(&run, &mut attempt, &published, &named, notes);
        self.at_primary(result, primary.as_deref())
    }

    /// What a published batch answers: its effects kept, its checks and what it touched, the
    /// reads after its last change, and its ops' outcomes in today's words.
    fn published(
        &self,
        run: &Run<'_>,
        attempt: &mut Attempt,
        published: &BTreeMap<String, Published>,
        named: &[String],
        notes: Vec<String>,
    ) -> ToolResult<Value> {
        let context = self.context(run.caller);
        if let Err(error) = self.keep_published(run, attempt, published, &context) {
            return ToolResult::failed(error);
        }
        let mut done = Done::new(String::new());
        if run.split > 0 && !published.is_empty() {
            self.batch_checks(named, published, &context, &mut done);
        }
        let mut result = ToolResult::done(done);
        self.batch_touched(named, published, &mut result);
        let versions: BTreeMap<String, u64> = named
            .iter()
            .filter_map(|doc| {
                let id = DocId::parse(doc)?;
                Some((doc.clone(), self.session.snapshot(&id).ok()?.number()))
            })
            .collect();
        self.trailing(run, attempt, &versions);
        let mut warnings = notes;
        warnings.append(&mut attempt.warnings);
        result.warnings = warnings;
        match answer(run, attempt, run.planned, result) {
            Ok(result) => result,
            Err(error) => ToolResult::failed(error),
        }
    }

    /// The ops before the last change run: in no transaction where they change no document,
    /// optimistically on one, gated on several.
    fn run_ops(&self, run: &Run<'_>, batched: &[String]) -> Result<BTreeMap<String, Published>, Stop> {
        match batched {
            [] => {
                run.attempt();
                for index in 0..run.split {
                    run.run(index, None);
                }
                Ok(BTreeMap::new())
            }
            [doc] => self.optimistic(run, doc),
            _ => self.gated(run, batched),
        }
    }

    /// `result` naming the batch's first document at its head, where it names one.
    fn at_primary(&self, mut result: ToolResult<Value>, primary: Option<&str>) -> ToolResult<Value> {
        result.doc = primary.map(str::to_owned);
        result.version = primary
            .and_then(DocId::parse)
            .and_then(|id| self.session.snapshot(&id).ok())
            .map(|head| head.number());
        result
    }

    /// One op planned: its name read, its roles, its arguments taken as meant.
    fn plan(&self, op: &Value, roles: RoleSet) -> Planned {
        let catalogue = &self.host.catalogue;
        let given = op.get("tool").and_then(Value::as_str).unwrap_or("").to_owned();
        let raw = op.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let retry = TryCall {
            tool: given.clone(),
            arguments: raw.as_object().cloned().unwrap_or_default(),
            fill: Vec::new(),
        };
        let mut notes = Vec::new();
        let tool = catalogue.tool_named(&given, &mut notes, true);
        let mut planned = Planned {
            given: given.clone(),
            tool: tool.clone(),
            arguments: retry.arguments.clone(),
            raw: raw.clone(),
            notes,
            refused: None,
            changes: false,
            docs: Vec::new(),
        };
        let Some(entry) = catalogue.get(&tool) else {
            planned.refused = Some(catalogue.no_tool(&given, roles, true, BATCH, &retry));
            return planned;
        };
        // (the documents it names, read from its arguments as given: a refused op's too)
        planned.docs = ooxml_tools::documents::named(&entry.spec.documents, &retry.arguments);
        if !entry.runs_in(BATCH) {
            let error = if entry.runs_in("read_batch") {
                ToolError::refusal(
                    Refusal::InvalidArguments,
                    format!(
                        "{tool} cannot run inside a batch; call it in read_batch (with other reads), or on its own"
                    ),
                    WayOut::names(["read_batch".to_owned(), tool.clone()])
                        .unwrap_or_else(|| WayOut::call(retry.clone())),
                )
            } else {
                ToolError::refusal(
                    Refusal::InvalidArguments,
                    format!("{tool} cannot run inside a batch; call it on its own"),
                    WayOut::call(retry.clone()),
                )
            };
            planned.refused = Some(error.at("tool"));
            return planned;
        }
        let checked = Catalogue::check_role(entry, None, roles, Some("tool"), &retry)
            .and_then(|()| {
                entry.spec.accept(raw).map_err(|error| {
                    let field = error
                        .field()
                        .map_or_else(|| "arguments".to_owned(), |f| format!("arguments.{f}"));
                    error.at(field)
                })
            })
            .and_then(|accepted| {
                Catalogue::check_role(entry, Some(&accepted.arguments), roles, None, &retry)
                    .map_err(|error| {
                        let field = error
                            .field()
                            .map_or_else(|| "arguments".to_owned(), |f| format!("arguments.{f}"));
                        error.at(field)
                    })
                    .map(|()| accepted)
            });
        match checked {
            Ok(accepted) => {
                planned.notes.extend(accepted.notes);
                planned.changes = entry.changes(&accepted.arguments);
                planned.docs = ooxml_tools::documents::named(&entry.spec.documents, &accepted.arguments);
                planned.arguments = accepted.arguments;
            }
            Err(error) => planned.refused = Some(error),
        }
        planned
    }

    /// A batch of one document run in its store, optimistically: the version it published.
    fn optimistic(&self, run: &Run<'_>, doc: &str) -> Result<BTreeMap<String, Published>, Stop> {
        let id = DocId::parse(doc).ok_or_else(|| Stop::Store(not_open(doc)))?;
        let format = self
            .session
            .format_doc(&id)
            .map_err(|e| Stop::Store(e.to_tool_error()))?;
        let caller = self.context(run.caller).caller;
        match format.stored().run_batch(&caller, run) {
            Ok(committed) => {
                let mut published = BTreeMap::new();
                published.insert(
                    doc.to_owned(),
                    Published {
                        version: committed.version,
                        previous: committed.previous,
                        lock: ooxml_session::LockSpent {
                            held: committed.timing.held,
                            waited: committed.timing.waited,
                        },
                    },
                );
                Ok(published)
            }
            Err(BatchRefused::Op(index)) => Err(Stop::Refused(index)),
            Err(BatchRefused::Store(error)) => Err(Stop::Store(error.to_tool_error())),
        }
    }

    /// A batch of several documents (decision D5): one owner holds a transaction open on each,
    /// in document order, so no other commit comes between; each op runs in its document's; all
    /// committed in document order, or none (an atomic batch's failed op, a panic).
    fn gated(&self, run: &Run<'_>, docs: &[String]) -> Result<BTreeMap<String, Published>, Stop> {
        let owner = self.host.owner();
        let clock = &self.session.context().clock;
        let mut waited = Duration::ZERO;
        let mut held = Held {
            owner,
            open: Vec::new(),
        };
        for doc in docs {
            let id = DocId::parse(doc).ok_or_else(|| Stop::Store(not_open(doc)))?;
            let format = self
                .session
                .format_doc(&id)
                .map_err(|e| Stop::Store(e.to_tool_error()))?;
            let before = format.stored().head();
            let asked = clock.monotonic();
            format
                .stored()
                .open_group(owner)
                .map_err(|e| Stop::Store(e.to_tool_error()))?;
            // (opening waits for the gate: another batch's, or a command's behind it)
            waited += clock.monotonic().saturating_sub(asked);
            held.open.push((doc.clone(), format, before, AttemptState::default()));
        }
        run.attempt();
        for index in 0..run.split {
            let doc = run.planned.get(index).and_then(|p| p.docs.first());
            let found = doc.and_then(|d| held.open.iter().find(|(o, ..)| o == d));
            let ran = match found {
                Some((_, format, _, state)) => format
                    .stored()
                    .in_group(owner, state, &mut |txn| run.run(index, Some(txn)))
                    .map_err(|e| Stop::Store(e.to_tool_error()))?,
                None => run.run(index, None),
            };
            if ran == OpRan::Failed && run.atomic {
                return Err(Stop::Refused(index));
            }
        }
        let caller = self.context(run.caller).caller;
        let mut published = BTreeMap::new();
        for (doc, format, before, _) in std::mem::take(&mut held.open) {
            let made = format
                .stored()
                .commit_group(owner, &caller)
                .map_err(|e| Stop::Store(e.to_tool_error()))?;
            let version = made
                .and_then(|(_, number)| format.stored().numbered(number))
                .unwrap_or_else(|| format.stored().head());
            published.insert(
                doc,
                Published {
                    previous: made.map(|_| before),
                    version,
                    lock: ooxml_session::LockSpent {
                        held: Duration::ZERO,
                        waited: std::mem::take(&mut waited),
                    },
                },
            );
        }
        Ok(published)
    }

    /// What the published batch keeps, once: its ops' format effects (in op order), its step
    /// on each document it changed, its refs, its images.
    fn keep_published(
        &self,
        run: &Run<'_>,
        attempt: &mut Attempt,
        published: &BTreeMap<String, Published>,
        context: &CallContext,
    ) -> Result<(), ToolError> {
        // the datasets the ops named or defined, kept at publish as today's batch keeps them
        // (their staleness said then, of the attempt that published; an earlier attempt's
        // went with it): over the limit only where another call filled it meanwhile, which
        // today's batch refuses too, after its ops applied
        let stale = self.finish_datasets(std::mem::take(&mut attempt.datasets_of))?;
        attempt.warnings.extend(stale);
        for (index, deferred) in std::mem::take(&mut attempt.deferred) {
            let Some(doc) = run.planned.get(index).and_then(|p| p.docs.first()) else {
                continue;
            };
            let (Some(at), Some(format)) = (
                published.get(doc),
                DocId::parse(doc).and_then(|id| self.session.format_doc(&id).ok()),
            ) else {
                continue;
            };
            for keep in deferred {
                keep(&*format, &at.version);
            }
        }
        for (doc, at) in published {
            if at.previous.is_none() {
                continue;
            }
            let Some(format) = DocId::parse(doc).and_then(|id| self.session.format_doc(&id).ok()) else {
                continue;
            };
            let owner = StepOwner::of(BATCH, context);
            format
                .stored()
                .stepped(at.version.seq(), at.version.number(), Some(owner));
        }
        let mut stores = self.stores();
        if let Some(refs) = &attempt.refs {
            for (doc, at) in published {
                if let Some(table) = stores.refs.get_mut(doc) {
                    match refs.defined(doc) {
                        Some(own) => table.keep(own, run.owner.as_ref(), at.version.number()),
                        None if at.previous.is_some() => {
                            table.keep(&ooxml_session::RefMap::new(), run.owner.as_ref(), at.version.number());
                        }
                        None => {}
                    }
                }
            }
        }
        if attempt.images > 0 {
            // (the format refused an op past the budgets before it drew anything)
            let _ = stores.images.reserve(attempt.images);
        }
        Ok(())
    }

    /// The checks of each document the batch names, of the version it published: one
    /// document's as they are, several's by document.
    fn batch_checks(
        &self,
        named: &[String],
        published: &BTreeMap<String, Published>,
        context: &CallContext,
        done: &mut Done<Value>,
    ) {
        let ports = self.session.context();
        let cancel = ooxml_session::CallCancel::new(context, &*ports.cancel, &*ports.clock);
        let mut found: Vec<(String, Map<String, Value>)> = Vec::new();
        for doc in named {
            let Some(id) = DocId::parse(doc) else { continue };
            let Ok(format) = self.session.format_doc(&id) else {
                continue;
            };
            let (before, at) = match published.get(doc) {
                Some(p) => (p.previous.as_ref(), p.version.clone()),
                None => (None, format.stored().head()),
            };
            if let Some(checks) = format.batch_checks(before, &at, context, &cancel) {
                found.push((doc.clone(), checks));
            }
        }
        done.checks = match found.len() {
            1 => found.pop().map(|(_, c)| c).unwrap_or_default(),
            _ => found.into_iter().map(|(d, c)| (d, Value::Object(c))).collect(),
        };
    }

    /// What the batch touched on each document it changed, from the head it was published
    /// after to the version it published.
    fn batch_touched(&self, named: &[String], published: &BTreeMap<String, Published>, result: &mut ToolResult<Value>) {
        let mut touched = Vec::new();
        let mut parts = Vec::new();
        for doc in named {
            let Some(at) = published.get(doc) else { continue };
            let Some(previous) = &at.previous else { continue };
            let Some(format) = DocId::parse(doc).and_then(|id| self.session.format_doc(&id).ok()) else {
                continue;
            };
            let mut one = ToolResult::done(Done::new(String::new()));
            self.say_touched(doc, &*format, previous, &at.version, None, &mut one);
            touched.append(&mut one.app.touched);
            parts.append(&mut one.app.changed_parts);
        }
        result.app.touched = touched;
        result.app.changed_parts = parts;
    }

    /// The reads after the batch's last change, each as the same call alone would run, on the
    /// version the batch left (an op that finds its document moved on says its `version`).
    fn trailing(&self, run: &Run<'_>, attempt: &mut Attempt, versions: &BTreeMap<String, u64>) {
        let clock = &self.session.context().clock;
        for index in run.split..run.planned.len() {
            let Some(planned) = run.planned.get(index) else {
                continue;
            };
            let elapsed = clock.monotonic().saturating_sub(run.started);
            if attempt.stopped.is_some() || passed(elapsed, run.timeout) {
                attempt.stopped.get_or_insert((index, elapsed.as_secs_f64()));
                let error = ToolError::failure(
                    Failure::Timeout,
                    format!("not run: the batch passed its {} s deadline", seconds(run.timeout)),
                );
                fail(attempt, index, planned, Failed { error, own: true });
                continue;
            }
            if let Some(error) = &planned.refused {
                fail(
                    attempt,
                    index,
                    planned,
                    Failed {
                        error: error.clone(),
                        own: true,
                    },
                );
                continue;
            }
            let Some(entry) = self.host.catalogue.get(&planned.tool) else {
                continue;
            };
            let retry = TryCall {
                tool: planned.tool.clone(),
                arguments: planned.arguments.clone(),
                fill: Vec::new(),
            };
            let mut going = Going::new(&planned.tool, Value::Object(planned.arguments.clone()));
            let result = self.run_format(entry, planned.arguments.clone(), run.caller, &mut going, &retry);
            attempt.warnings.extend(
                going
                    .notes
                    .iter()
                    .chain(&result.warnings)
                    .map(|w| format!("ops[{index}]: {w}")),
            );
            match &result.outcome {
                // (a read after the last change answers as the same call alone would)
                Outcome::Failed(error) => {
                    fail(
                        attempt,
                        index,
                        planned,
                        Failed {
                            error: error.clone(),
                            own: true,
                        },
                    );
                }
                Outcome::Done(done) => {
                    for dataset in &done.datasets {
                        if !attempt.datasets.contains(dataset) {
                            attempt.datasets.push(dataset.clone());
                        }
                    }
                    let mut entry = outcome(index, &planned.tool, done);
                    entry.shift_remove("created");
                    let doc = going.docs.first();
                    if let (Some(doc), Some(version)) = (doc, result.version) {
                        if versions.get(doc).is_some_and(|v| *v != version) {
                            // another call's change came first
                            entry.insert("version".into(), Value::from(version));
                        }
                    }
                    attempt.outcomes.insert(index, entry);
                }
            }
        }
    }
}

/// What an op's run through the pipeline gave: its reply, what the session gave beside (its
/// versions, its format's effects), and the ref names it keeps folded (`(kept, given)`).
struct OpRun {
    result: ToolResult<Value>,
    inner: Option<InBatch>,
    folded: Vec<(String, String)>,
}

/// What a batch published on one document.
struct Published {
    /// The version published (the head it ran on, where it changed nothing).
    version: AnyVersion,
    /// The version it was published after; `None` where it changed nothing.
    previous: Option<AnyVersion>,
    /// How long its commit held the commit lock and waited for it and the gate.
    lock: ooxml_session::LockSpent,
}

/// Why a batch published nothing.
enum Stop {
    /// An atomic batch's op failed.
    Refused(usize),
    /// The store refused.
    Store(ToolError),
}

/// The transactions one owner holds open, let go uncommitted on every path that does not
/// commit them (an error, a panic).
struct Held {
    owner: TxnOwner,
    open: Vec<(String, Arc<dyn ooxml_session::FormatDoc>, AnyVersion, AttemptState)>,
}

impl Drop for Held {
    fn drop(&mut self) {
        for (_, format, _, _) in &self.open {
            format.stored().drop_group(self.owner);
        }
    }
}

impl BatchOps for Run<'_> {
    fn attempt(&self) {
        let mut attempt = self.lock();
        *attempt = Attempt {
            refs: Some(CallRefs::new(self.owner.clone())),
            ..Attempt::default()
        };
    }

    fn count(&self) -> usize {
        self.split
    }

    fn atomic(&self) -> bool {
        self.atomic
    }

    fn run(&self, index: usize, txn: &dyn OpTxn) -> OpRan {
        Run::run(self, index, Some(txn))
    }
}

impl Run<'_> {
    fn lock(&self) -> std::sync::MutexGuard<'_, Attempt> {
        self.attempt.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Op `index` of this attempt, in `txn` where its document is the batch's.
    fn run(&self, index: usize, txn: Option<&dyn OpTxn>) -> OpRan {
        let host = self.host;
        let Some(planned) = self.planned.get(index) else {
            return OpRan::NotRun;
        };
        {
            let mut attempt = self.lock();
            if let Some(ran) = self.past_deadline(&mut attempt, index, planned) {
                return ran;
            }
            if let Some(error) = &planned.refused {
                fail(
                    &mut attempt,
                    index,
                    planned,
                    Failed {
                        error: error.clone(),
                        own: true,
                    },
                );
                return OpRan::Failed;
            }
        }
        let Some(entry) = host.host.catalogue.get(&planned.tool) else {
            return OpRan::Failed;
        };
        let retry = TryCall {
            tool: planned.tool.clone(),
            arguments: planned.arguments.clone(),
            fill: Vec::new(),
        };
        let doc = planned.docs.first().cloned().unwrap_or_default();
        let id = DocId::parse(&doc);
        // a document not open, or opened after the batch began
        let opened = id.as_ref().and_then(|id| host.session.kind_of(id).ok()).is_some();
        if !opened {
            let error = id
                .and_then(|id| host.session.snapshot(&id).err())
                .map_or_else(|| not_open(&doc), |e| e.to_tool_error().at("doc"));
            fail(&mut self.lock(), index, planned, Failed { error, own: false });
            return OpRan::Failed;
        }
        let mut refs = self
            .lock()
            .refs
            .take()
            .unwrap_or_else(|| CallRefs::new(self.owner.clone()));
        let mark = refs.mark();
        let mut datasets = std::mem::take(&mut self.lock().datasets_of);
        datasets.start_op(index);
        let datasets_mark = datasets.mark();
        let ran = self.run_op(planned, entry, &doc, &mut refs, &mut datasets, txn, &retry);
        let mut attempt = self.lock();
        let ran = match ran {
            Ok(op) => self.keep_op(&mut attempt, index, planned, &doc, &mut refs, op, &retry),
            Err(failed) => Err(failed),
        };
        let ran = match ran {
            Ok(()) => {
                if planned.changes {
                    datasets.changed(&doc);
                }
                OpRan::Applied
            }
            Err(failed) => {
                datasets.reset(datasets_mark);
                refs.reset(mark);
                refs.failed(index, &planned.raw);
                fail(&mut attempt, index, planned, failed);
                OpRan::Failed
            }
        };
        attempt.refs = Some(refs);
        attempt.datasets_of = datasets;
        ran
    }

    /// Op `index` not run where the batch is past its deadline (an atomic batch refused).
    fn past_deadline(&self, attempt: &mut Attempt, index: usize, planned: &Planned) -> Option<OpRan> {
        let clock = &self.host.session.context().clock;
        let elapsed = clock.monotonic().saturating_sub(self.started);
        if attempt.stopped.is_none() && !passed(elapsed, self.timeout) {
            return None;
        }
        let at = *attempt.stopped.get_or_insert((index, elapsed.as_secs_f64()));
        let error = if self.atomic {
            let (note, not_run) = not_run(self.timeout, at.0, self.split, at.1);
            ToolError::failure(
                Failure::Timeout,
                format!("the batch passed its {} s deadline: {note}", seconds(self.timeout)),
            )
            .with_detail("not_run", not_run)
        } else {
            ToolError::failure(
                Failure::Timeout,
                format!("not run: the batch passed its {} s deadline", seconds(self.timeout)),
            )
        };
        let own = !self.atomic;
        fail(attempt, index, planned, Failed { error, own });
        Some(if self.atomic { OpRan::Failed } else { OpRan::NotRun })
    }

    /// The op run through the host's pipeline: its refs resolved, its handler run in `txn`
    /// (on the version the ops before it left) or alone.
    #[allow(clippy::too_many_arguments)]
    fn run_op(
        &self,
        planned: &Planned,
        entry: &crate::catalogue::Entry,
        doc: &str,
        refs: &mut CallRefs,
        datasets: &mut CallDatasets,
        txn: Option<&dyn OpTxn>,
        retry: &TryCall,
    ) -> Result<OpRun, Failed> {
        let host = self.host;
        let mut arguments = planned.arguments.clone();
        let kind = DocId::parse(doc).and_then(|id| host.session.kind_of(&id).ok());
        let mut ignored = entry.spec.ignored_for(kind.map(|k| k.as_str()), &mut arguments);
        let with = host
            .with_refs(entry, refs, doc, arguments, planned.changes, retry)
            .map_err(|error| Failed { error, own: false })?;
        // 6.-8. its sources, the datasets it names, its references (as the call alone)
        let valued = host
            .with_values(entry, doc, with.arguments, retry, datasets)
            .map_err(|refused| Failed {
                error: refused.error,
                own: false,
            })?;
        let mut context = host.context(self.caller);
        context.values = valued.values;
        context.values.merge(self.values.clone());
        let txn = txn.filter(|_| self.batched.iter().any(|d| d == doc));
        let id = DocId::parse(doc);
        if let (Some(txn), Some(id)) = (txn, &id) {
            let at = host
                .session
                .format_doc(id)
                .ok()
                .and_then(|format| format.stored().version_in(txn))
                .and_then(|version| host.session.opened_at(id, version).ok());
            context.at.extend(at);
        }
        let mut run = match txn {
            Some(txn) => host.session.edit_in(entry.name(), valued.arguments, &context, txn),
            None => {
                let (result, made) = host.session.edit_accepted(entry.name(), valued.arguments, &context);
                InBatch {
                    result,
                    made,
                    deferred: Vec::new(),
                }
            }
        };
        // what the host adds to the tool's reply (check's validation), of the version it read
        let read = txn.zip(id.as_ref()).and_then(|(txn, id)| {
            let format = host.session.format_doc(id).ok()?;
            let at = format.stored().version_in(txn)?;
            Some((format, at))
        });
        match read {
            Some((format, at)) => host.host_added_at(entry.name(), doc, &*format, &at, &mut run.result),
            None => host.host_added(entry.name(), doc, &mut run.result),
        }
        let mut warnings = with.notes;
        warnings.append(&mut ignored);
        warnings.append(&mut run.result.warnings);
        run.result.warnings = warnings;
        let result = std::mem::replace(&mut run.result, ToolResult::failed(ToolError::internal("", "")));
        Ok(OpRun {
            result,
            inner: Some(run),
            folded: with.folded,
        })
    }

    /// An op's result kept for the attempt: its refs defined (pending until the batch is
    /// published), what it said merged, its format effects and versions kept.
    #[allow(clippy::too_many_arguments)]
    fn keep_op(
        &self,
        attempt: &mut Attempt,
        index: usize,
        planned: &Planned,
        doc: &str,
        refs: &mut CallRefs,
        OpRun {
            mut result,
            inner,
            folded,
        }: OpRun,
        retry: &TryCall,
    ) -> Result<(), Failed> {
        let mut effects = OpEffects::default();
        effects.take_from(&mut result);
        let done = match &mut result.outcome {
            Outcome::Failed(error) => {
                return Err(Failed {
                    error: error.clone(),
                    own: false,
                })
            }
            Outcome::Done(done) => done,
        };
        {
            let stores = self.host.stores();
            let table = stores.refs.get(doc);
            for (name, address) in &effects.refs {
                refs.define(name, address, doc, table, retry)
                    .map_err(|error| Failed { error, own: false })?;
            }
        }
        // a ref name the op gave that it keeps folded, first among its warnings
        let mut warnings: Vec<String> = folded
            .iter()
            .filter(|(kept, _)| effects.refs.iter().any(|(n, _)| n == kept))
            .map(|(kept, given)| ooxml_tools::refs::folded_warning(given, kept))
            .collect();
        warnings.append(&mut result.warnings);
        for changed in &done.changed {
            if !attempt.changed.contains(changed) {
                attempt.changed.push(changed.clone());
            }
        }
        attempt.created.extend(done.created.iter().cloned());
        attempt.removed.extend(done.removed.iter().cloned());
        for (from, to) in &done.renamed {
            match attempt.renamed.iter_mut().find(|(f, _)| f == from) {
                Some(kept) => kept.1.clone_from(to),
                None => attempt.renamed.push((from.clone(), to.clone())),
            }
        }
        for dataset in effects.datasets.iter().chain(&done.datasets) {
            if !attempt.datasets.contains(dataset) {
                attempt.datasets.push(dataset.clone());
            }
        }
        attempt.images += effects.images;
        attempt
            .warnings
            .extend(warnings.iter().map(|w| format!("ops[{index}]: {w}")));
        attempt.outcomes.insert(index, outcome(index, &planned.tool, done));
        if let Some(inner) = inner {
            if !inner.deferred.is_empty() {
                attempt.deferred.push((index, inner.deferred));
            }
            if let Some(made) = inner.made {
                attempt
                    .made
                    .entry(doc.to_owned())
                    .and_modify(|m| {
                        m.last = made.last.clone();
                        m.format.clone_from(&made.format);
                    })
                    .or_insert(made);
            }
        }
        Ok(())
    }
}

/// An op's outcome as the reply lists it: its summary, the ids it created, its data.
fn outcome(index: usize, tool: &str, done: &Done<Value>) -> Map<String, Value> {
    let mut entry = Map::new();
    entry.insert("op".into(), Value::from(index));
    entry.insert("tool".into(), Value::from(tool));
    entry.insert("summary".into(), Value::from(done.summary.as_str()));
    if !done.created.is_empty() {
        entry.insert("created".into(), Value::from(done.created.clone()));
    }
    let empty = match &done.data {
        None | Some(Value::Null) => true,
        Some(Value::Object(map)) => map.is_empty(),
        Some(Value::Array(items)) => items.is_empty(),
        Some(_) => false,
    };
    if !empty {
        entry.insert("data".into(), done.data.clone().unwrap_or(Value::Null));
    }
    entry
}

/// Op `index` failed with `failed`: its outcome the error, in brief.
fn fail(attempt: &mut Attempt, index: usize, planned: &Planned, failed: Failed) {
    // (an op refused as it was planned is named as it gives its tool)
    let name = if planned.refused.is_some() {
        planned.given.clone()
    } else {
        planned.tool.clone()
    };
    let mut entry = Map::new();
    entry.insert("op".into(), Value::from(index));
    entry.insert("tool".into(), Value::from(name));
    entry.insert("error".into(), brief(&failed.fielded()));
    attempt.outcomes.insert(index, entry);
    attempt.errors.insert(index, failed);
}

/// An op's error in its outcome: code and message, and what else it has.
fn brief(error: &ToolError) -> Value {
    let mut brief = Map::new();
    brief.insert("code".into(), Value::from(error.code().as_str()));
    brief.insert("message".into(), Value::from(error.message()));
    if let Some(field) = error.field() {
        brief.insert("field".into(), Value::from(field));
    }
    let options = error.way_out().map_or(&[][..], WayOut::valid_options);
    if !options.is_empty() {
        brief.insert("valid_options".into(), Value::Array(options.to_vec()));
    }
    if !error.details().is_empty() {
        brief.insert("details".into(), Value::Object(error.details().clone()));
    }
    Value::Object(brief)
}

/// An atomic batch's refusal of its first op that cannot run, before any runs.
fn planning_refusal(planned: &[Planned]) -> Option<ToolError> {
    let (index, p, error) = planned
        .iter()
        .enumerate()
        .find_map(|(i, p)| p.refused.as_ref().map(|e| (i, p, e)))?;
    let said = if error.field() == Some("tool") {
        format!("ops[{index}]")
    } else if p.given == p.tool {
        format!("ops[{index}] ({})", p.given)
    } else {
        format!("ops[{index}] ({}, read as {})", p.given, p.tool)
    };
    let field = error
        .field()
        .map_or_else(|| format!("ops[{index}]"), |f| format!("ops[{index}].{f}"));
    Some(recoded(error, format!("{said}: {}", error.message())).at(field))
}

/// `error`'s code, way out and details with `message`.
fn recoded(error: &ToolError, message: String) -> ToolError {
    let mut out = error.clone().with_message(message);
    if let ErrorCode::Failure(_) = error.code() {
        if let Some(way_out) = error.way_out() {
            out = out.with_way_out(way_out.clone());
        }
    }
    out
}

/// An atomic batch's refusal: op `index` failed, so nothing was applied.
fn atomic_refusal(planned: &[Planned], attempt: &Attempt, index: usize) -> ToolError {
    let tool = planned.get(index).map_or("?", |p| p.tool.as_str());
    let Some(failed) = attempt.errors.get(&index) else {
        return ToolError::internal(format!("ops[{index}] failed"), "host:batch");
    };
    let error = &failed.error;
    let message = format!(
        "ops[{index}] ({tool}) failed, so nothing in the batch was applied: {}",
        error.message()
    );
    let field = match error.field() {
        Some(field) => format!("ops[{index}].arguments.{field}"),
        None => format!("ops[{index}]"),
    };
    let mut out = recoded(error, message).at(field);
    let mut details = Map::new();
    details.insert("op".into(), Value::from(index));
    details.insert("tool".into(), Value::from(tool));
    details.insert("error".into(), error.to_json());
    details.insert("completed_before".into(), Value::from(index));
    if let Some(not_run) = error.details().get("not_run") {
        details.insert("not_run".into(), not_run.clone());
    }
    for (key, value) in details {
        out = out.with_detail(&key, value);
    }
    out
}

/// The batch's reply: its ops' outcomes in order, what failed, what the deadline stopped.
fn answer(
    run: &Run<'_>,
    attempt: &Attempt,
    planned: &[Planned],
    mut result: ToolResult<Value>,
) -> Result<ToolResult<Value>, ToolError> {
    let outcomes: Vec<&Map<String, Value>> = attempt.outcomes.values().collect();
    let not_run = attempt
        .stopped
        .map(|(first, elapsed)| not_run(run.timeout, first, planned.len(), elapsed));
    let errors: Vec<(usize, ToolError)> = attempt.errors.iter().map(|(i, f)| (*i, f.fielded())).collect();
    if !errors.is_empty() && errors.len() == outcomes.len() {
        return Err(none_applied(&outcomes, &errors, not_run.as_ref()));
    }
    let done_ops: Vec<&Map<String, Value>> = outcomes.iter().copied().filter(|o| !o.contains_key("error")).collect();
    let listed = Value::Array(outcomes.iter().map(|o| Value::Object((*o).clone())).collect());
    let lead = if run.split > 0 {
        format!("Ran {} op(s) as one step: ", done_ops.len())
    } else {
        "Read: ".to_owned()
    };
    let mut data = Map::new();
    let summary = if errors.is_empty() {
        data.insert("ops".into(), listed);
        format!("{lead}{}", summaries(&done_ops))
    } else {
        let stopped = attempt.stopped.map(|(first, _)| first);
        let failures: Vec<(usize, ToolError)> = errors
            .iter()
            .filter(|(i, _)| stopped.is_none_or(|first| *i < first))
            .cloned()
            .collect();
        let (mut said, applied) = if run.atomic {
            // (an atomic batch's ops all applied: only a read after them failed)
            let mut said = vec![format!("{lead}{}", summaries(&done_ops))];
            if !failures.is_empty() {
                said.push(format!(
                    "Not answered, the batch applied -- resend only these on their own: {}",
                    failures_said(&outcomes, &failures)
                ));
            }
            (said, run.split)
        } else {
            let mut said = vec![format!(
                "Ran {} of {} ops as one step: {}",
                done_ops.len(),
                outcomes.len(),
                summaries(&done_ops)
            )];
            if !failures.is_empty() {
                said.push(format!(
                    "Not applied -- fix and resend only these: {}",
                    failures_said(&outcomes, &failures)
                ));
            }
            (said, done_ops.len())
        };
        data.insert("applied".into(), Value::from(applied));
        data.insert(
            "failed".into(),
            Value::Array(errors.iter().map(|(i, _)| Value::from(*i)).collect()),
        );
        data.insert("ops".into(), listed);
        if let Some((note, not_run)) = &not_run {
            said.push(format!("Not run: {note}"));
            data.insert("not_run".into(), not_run.clone());
        }
        said.join(". ")
    };
    if let Outcome::Done(done) = &mut result.outcome {
        done.summary = summary;
        done.data = Some(Value::Object(data));
        merged_into(done, attempt);
    }
    Ok(result)
}

/// What the applied ops said, merged, into the batch's reply: what they changed, created,
/// removed and renamed, their datasets and the refs they defined.
fn merged_into(done: &mut Done<Value>, attempt: &Attempt) {
    done.changed.clone_from(&attempt.changed);
    done.created.clone_from(&attempt.created);
    done.removed.clone_from(&attempt.removed);
    done.renamed.clone_from(&attempt.renamed);
    done.datasets.clone_from(&attempt.datasets);
    done.refs = attempt
        .refs
        .iter()
        .flat_map(|own| {
            own.defined_docs()
                .filter_map(|doc| own.defined(doc))
                .flat_map(|map| map.iter().map(|(n, a)| (n.to_owned(), a.to_owned())))
                .collect::<Vec<_>>()
        })
        .collect();
}

/// The first few ops' summaries.
fn summaries(done: &[&Map<String, Value>]) -> String {
    let said: Vec<&str> = done
        .iter()
        .take(5)
        .filter_map(|o| o.get("summary").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .collect();
    let more = if done.len() > 5 {
        format!("; and {} more", done.len() - 5)
    } else {
        String::new()
    };
    format!("{}{more}", said.join("; "))
}

/// The failed ops for a message: `ops[1] (tool): message`, the first few.
fn failures_said(outcomes: &[&Map<String, Value>], errors: &[(usize, ToolError)]) -> String {
    let tool_of = |index: usize| {
        outcomes
            .iter()
            .find(|o| o.get("op").and_then(Value::as_u64) == Some(index as u64))
            .and_then(|o| o.get("tool").and_then(Value::as_str))
            .filter(|t| !t.is_empty())
            .unwrap_or("?")
            .to_owned()
    };
    let said: Vec<String> = errors
        .iter()
        .take(5)
        .map(|(i, e)| format!("ops[{i}] ({}): {}", tool_of(*i), e.message()))
        .collect();
    let more = errors.len().saturating_sub(5);
    let tail = if more > 0 {
        format!("; and {more} more (data.ops)")
    } else {
        String::new()
    };
    format!("{}{tail}", said.join("; "))
}

/// A batch none of whose ops applied: the first failure's code, field and options, and every
/// op's outcome in `details`.
fn none_applied(
    outcomes: &[&Map<String, Value>],
    errors: &[(usize, ToolError)],
    not_run: Option<&(String, Value)>,
) -> ToolError {
    let Some((index, first)) = errors.first() else {
        return ToolError::internal("no failure", "host:batch");
    };
    let tool = failures_said(outcomes, errors.get(..1).unwrap_or_default());
    let mut lead = if errors.len() == 1 {
        let tool = tool.split_once(": ").map_or(tool.as_str(), |(t, _)| t).to_owned();
        format!("{tool} failed, so nothing was applied: {}", first.message())
    } else {
        format!(
            "none of the {} ops applied: {}",
            errors.len(),
            failures_said(outcomes, errors)
        )
    };
    let mut details = Map::new();
    details.insert("applied".into(), Value::from(0));
    details.insert(
        "failed".into(),
        Value::Array(errors.iter().map(|(i, _)| Value::from(*i)).collect()),
    );
    details.insert(
        "ops".into(),
        Value::Array(outcomes.iter().map(|o| Value::Object((*o).clone())).collect()),
    );
    if let Some((note, not_run)) = not_run {
        lead.push_str(&format!(". Not run: {note}"));
        details.insert("not_run".into(), not_run.clone());
    }
    let field = match first.field() {
        Some(field) => format!("ops[{index}].{field}"),
        None => format!("ops[{index}]"),
    };
    let mut out = recoded(first, lead).at(field);
    // (the first failure's details are the ops'; its way out stays)
    for (key, value) in details {
        out = out.with_detail(&key, value);
    }
    out
}

/// What a batch's deadline stopped -- ops `first` to `total - 1`, the batch having run for
/// `elapsed` s -- as a sentence and as `data.not_run`.
fn not_run(timeout: Duration, first: usize, total: usize, elapsed: f64) -> (String, Value) {
    let count = total.saturating_sub(first);
    let last = total.saturating_sub(1);
    let rounded = (elapsed * 10.0).round() / 10.0;
    let mut data = Map::new();
    data.insert("count".into(), Value::from(count));
    data.insert("first".into(), Value::from(first));
    data.insert("last".into(), Value::from(last));
    data.insert("ran".into(), Value::from(first));
    data.insert("seconds".into(), Value::from(rounded));
    let span = if count == 1 {
        format!("ops[{first}]")
    } else {
        format!("ops[{first}]..ops[{last}]")
    };
    let mut text = format!(
        "{count} op(s), {span}, after the batch passed its {} s deadline ({first} op(s) ran in {elapsed:.1} s)",
        seconds(timeout)
    );
    if first > 0 && elapsed > 0.0 {
        let each = elapsed / first as f64;
        let size = ((timeout.as_secs_f64() * FILL / each) as usize).max(1);
        data.insert("batch_size".into(), Value::from(size));
        text.push_str(&format!(
            "; resend them as batches of at most {size} ops (about {each:.2} s an op so far)"
        ));
    } else {
        text.push_str("; resend them as smaller batches");
    }
    (text, Value::Object(data))
}

/// The refusal of a document that is not open.
fn not_open(doc: &str) -> ToolError {
    ToolError::failure(Failure::Internal, format!("no open document {doc}")).at("doc")
}
