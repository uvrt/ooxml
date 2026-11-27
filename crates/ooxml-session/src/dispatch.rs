//! Owns: the dispatch of a tool call ([`Session::call`], [`Session::edit`], a batch's op):
//! the arguments taken as meant, the tool and its document found, the handler run on the
//! versions the call's context names (else on the documents' heads), and what the call made
//! -- its versions ([`CallMade`]), the ids it retired, who made its step -- noted; `undo` and
//! `read_batch` handed to the session's own.
//!
//! Must not: hold a lock while a handler runs, or implement a tool.

use crate::batch::{MAX_READ_BATCH_CHARS, READ_BATCH};
use crate::context::{revision_date, CallCancel};
use crate::format::{Deferred, Finished, FormatRun, LockSpent};
use crate::session::Opened;
use crate::undo::UNDO;
use crate::{CallContext, DocId, DocKind, Session, SessionError};
use ooxml_base::Cancel;
use ooxml_store::OpTxn;
use ooxml_tools::{Accepted, Refusal, ToolError, ToolResult, ToolSpec, WayOut};
use serde_json::{Map, Value};
use std::any::Any;
use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

/// A call's tool and its arguments as meant.
type Found = (Arc<dyn ooxml_tools::Tool>, Map<String, Value>);

/// The versions a batch reads, by document.
pub(crate) type Pinned = BTreeMap<DocId, Opened>;

/// What one call gathers as it runs: the document version it read, its warnings, the
/// versions its batch reads, and where it may ask where blocks are laid out.
pub(crate) struct CallState<'p> {
    opened: Option<Opened>,
    warnings: Vec<String>,
    pinned: Option<&'p mut Pinned>,
    /// The other documents the call names, by the names it gives them, at the versions
    /// pinned for it.
    others: Vec<(String, Opened)>,
    /// The name the call gives its own document.
    named: String,
    /// What the call's edits made, as its document's format keeps it.
    made: Mutex<Option<Box<dyn Any + Send>>>,
    /// The date the call's writes carry.
    date: String,
    /// Whether the call's arguments were taken as meant already (by the host: design phase
    /// 3, one acceptance), so the session takes them as they are.
    accepted: bool,
    /// The batch's transaction the call's edits run in, where it is handed one.
    txn: Option<&'p dyn OpTxn>,
    /// What the call keeps once its batch is published.
    deferred: Mutex<Vec<Deferred>>,
}

impl<'p> CallState<'p> {
    /// A call's state as it starts: the versions its batch reads (`pinned`), its writes'
    /// `date`, whether its arguments were taken as meant already, and the batch's transaction
    /// its edits run in.
    fn new(pinned: Option<&'p mut Pinned>, date: String, accepted: bool, txn: Option<&'p dyn OpTxn>) -> Self {
        CallState {
            opened: None,
            warnings: Vec::new(),
            pinned,
            others: Vec::new(),
            named: String::new(),
            made: Mutex::new(None),
            date,
            accepted,
            txn,
            deferred: Mutex::new(Vec::new()),
        }
    }
}

/// The versions one changing call made ([`Session::edit`]): its first edit's number and its
/// last edit's version, held -- the call's own, whatever other calls committed meanwhile.
#[derive(Clone)]
pub struct CallMade {
    /// The number of the version its first edit made.
    pub first: u64,
    /// The version its first edit was applied to, held ([`Opened::number`] its state
    /// number): the state its step goes from, the call's own however many steps others
    /// commit before the host reads it (the session's history may have let it go by then).
    pub before: Opened,
    /// The version its last edit made.
    pub last: Opened,
    /// What its format says of the call beside ([`Finished::format`]: a Word call's touched,
    /// validation and reflow, `ooxml-session-word`'s `WordMade`).
    pub format: Option<Arc<dyn Any + Send + Sync>>,
    /// Where its commits' time went ([`Finished::lock`]).
    pub lock: LockSpent,
}

/// One op of a host's batch run in the batch's transaction ([`Session::edit_in`]).
pub struct InBatch {
    /// Its reply (its version the state number its batch's commit is expected to be given).
    pub result: ToolResult<Value>,
    /// The versions its edits made, the batch's own until it is published; `None` where it
    /// changed nothing.
    pub made: Option<CallMade>,
    /// What its format keeps once the batch is published, in order.
    pub deferred: Vec<Deferred>,
}

impl InBatch {
    /// The reply and the versions, as a call that is not a batch's op takes them.
    fn into_call(self) -> (ToolResult<Value>, Option<CallMade>) {
        (self.result, self.made)
    }
}

/// The versions `context` names, by document.
pub(crate) fn pinned_by(context: &CallContext) -> Pinned {
    context.at.iter().map(|o| (o.id.clone(), o.clone())).collect()
}

impl Session {
    /// Run the tool `name` with `arguments` (a JSON object, or its text) under `context`:
    /// the arguments taken as meant and validated, the document found, the handler run on
    /// the version `context` names for it, else on its head snapshot. Never panics and
    /// never fails: what went wrong is the result's error, with its way out -- a panic in
    /// the tool too, as an `internal` failure (counted in [`crate::SessionStats::panics`]).
    pub fn call(&self, name: &str, arguments: Value, context: &CallContext) -> ToolResult<Value> {
        if name == READ_BATCH {
            let retry = arguments.clone();
            return catch_unwind(AssertUnwindSafe(|| self.call_batch(arguments, context)))
                .unwrap_or_else(|panic| self.panicked(name, retry, &*panic, false));
        }
        if name == UNDO {
            self.counts.calls.fetch_add(1, Ordering::Relaxed);
            let retry = arguments.clone();
            let result = catch_unwind(AssertUnwindSafe(|| self.call_undo(arguments, context)))
                .unwrap_or_else(|panic| self.panicked(name, retry, &*panic, false));
            if let ooxml_tools::Outcome::Failed(error) = &result.outcome {
                self.counts.failed(name, error);
            }
            return result;
        }
        let mut pinned = pinned_by(context);
        self.call_one(name, arguments, context, "", Some(&mut pinned))
    }

    /// `read_batch`: its arguments taken as meant and validated, then its ops run.
    fn call_batch(&self, arguments: Value, context: &CallContext) -> ToolResult<Value> {
        self.counts.calls.fetch_add(1, Ordering::Relaxed);
        let spec = self.registry.batch_spec();
        let outcome = spec.accept(arguments).and_then(|Accepted { arguments, notes }| {
            self.read_batch(&Value::Object(arguments), context)
                .map(|done| (done, notes))
        });
        let mut result = match outcome {
            Ok((done, notes)) => {
                let mut result = ToolResult::done(done);
                result.warnings = notes;
                result
            }
            Err(error) => {
                self.counts.failed(READ_BATCH, &error);
                ToolResult::failed(error)
            }
        };
        result.app.tool = READ_BATCH.to_owned();
        result.app.budget = Some(MAX_READ_BATCH_CHARS);
        result
    }

    /// Run the editing tool `name` as [`Session::call`] does, and the versions its edits
    /// made (`None` where it changed nothing): the call's step, as a host that numbers the
    /// document's steps takes it while other calls commit to the document concurrently.
    pub fn edit(&self, name: &str, arguments: Value, context: &CallContext) -> (ToolResult<Value>, Option<CallMade>) {
        let mut pinned = pinned_by(context);
        self.run_one(name, arguments, context, "", Some(&mut pinned))
    }

    /// [`Session::edit`] of a call whose arguments its host took as meant already
    /// ([`ooxml_tools::ToolSpec::accept`], the one acceptance): they are validated arguments,
    /// run as they are -- no note is added twice, no alias read again.
    pub fn edit_accepted(
        &self,
        name: &str,
        arguments: Map<String, Value>,
        context: &CallContext,
    ) -> (ToolResult<Value>, Option<CallMade>) {
        let mut pinned = pinned_by(context);
        self.run(
            name,
            Value::Object(arguments),
            context,
            "",
            Some(&mut pinned),
            true,
            None,
        )
        .into_call()
    }

    /// [`Session::edit_accepted`] of one op of a host's batch of one document, its edits run in
    /// the batch's transaction `txn` (on the version the ops before it left): its step is the
    /// batch's, and what its format keeps waits for the batch's publish.
    pub fn edit_in(
        &self,
        name: &str,
        arguments: Map<String, Value>,
        context: &CallContext,
        txn: &dyn OpTxn,
    ) -> InBatch {
        let mut pinned = pinned_by(context);
        self.run(
            name,
            Value::Object(arguments),
            context,
            "",
            Some(&mut pinned),
            true,
            Some(txn),
        )
    }

    /// One call; `within` names the batch it runs in (`""`: none), whose refusals name it and
    /// whose op's argument errors are fielded under `arguments`; `pinned`, the versions the
    /// batch reads (each document at the version its first op read, as today's batch holds
    /// its documents for all its ops).
    pub(crate) fn call_one(
        &self,
        name: &str,
        arguments: Value,
        context: &CallContext,
        within: &str,
        pinned: Option<&mut Pinned>,
    ) -> ToolResult<Value> {
        self.run_one(name, arguments, context, within, pinned).0
    }

    /// [`Session::call_one`], and the versions the call's edits made.
    fn run_one(
        &self,
        name: &str,
        arguments: Value,
        context: &CallContext,
        within: &str,
        pinned: Option<&mut Pinned>,
    ) -> (ToolResult<Value>, Option<CallMade>) {
        self.run(name, arguments, context, within, pinned, false, None)
            .into_call()
    }

    /// [`Session::run_one`]; `accepted`: the arguments were taken as meant by the host;
    /// `txn`: the batch's transaction its edits run in.
    #[allow(clippy::too_many_arguments)]
    fn run(
        &self,
        name: &str,
        arguments: Value,
        context: &CallContext,
        within: &str,
        pinned: Option<&mut Pinned>,
        accepted: bool,
        txn: Option<&dyn OpTxn>,
    ) -> InBatch {
        self.counts.calls.fetch_add(1, Ordering::Relaxed);
        let cancel = CallCancel::new(context, &*self.context.cancel, &*self.context.clock);
        let date = context
            .date
            .clone()
            .unwrap_or_else(|| revision_date(self.context.clock.now()));
        let mut state = CallState::new(pinned, date, accepted, txn);
        // a panic in the handler is the call's internal failure (crate::panics), never the host's
        let retry = arguments.clone();
        let run = AssertUnwindSafe(|| {
            if within.is_empty() {
                self.dispatch(name, arguments, context, &cancel, &mut state)
            } else {
                self.dispatch_within(name, arguments, context, &cancel, &mut state, within)
            }
        });
        let mut result = catch_unwind(run).unwrap_or_else(|panic| {
            let changed = state.made.get_mut().unwrap_or_else(PoisonError::into_inner).is_some();
            self.panicked(name, retry, &*panic, changed)
        });
        if let ooxml_tools::Outcome::Failed(error) = &result.outcome {
            self.counts.failed(name, error);
        }
        let made = state.made.get_mut().unwrap_or_else(PoisonError::into_inner).take();
        let mut call_made = None;
        if let Some(opened) = state.opened.take() {
            result.doc = Some(state.named.clone());
            // a changing call's version is the last it made (else the head it left); a
            // reading call's, the one read
            let mutates = self.registry.named(name).is_some_and(|t| t.spec().flags.mutates);
            let document = self.document(&opened.id).ok();
            let finished = made.zip(document.as_ref()).and_then(|(made, document)| {
                let none = Map::new();
                let run = self.format_run(name, &none, &opened, &state, context, &cancel);
                let finished = document.doc.finish(made, &run)?;
                // its step and who made it, unless it ran in a batch (whose close makes the step)
                if !finished.batch {
                    document.stepped(finished.last.seq(), finished.last.number(), Some(name), context);
                }
                Some(finished)
            });
            call_made = finished.zip(document.as_ref()).map(|(finished, document)| {
                let Finished {
                    first,
                    before,
                    last,
                    format,
                    lock,
                    ..
                } = finished;
                CallMade {
                    first,
                    before: Self::opened(&opened.id, document, before),
                    last: Self::opened(&opened.id, document, last),
                    format,
                    lock,
                }
            });
            // (state numbers, as the tool layer numbers the document's states)
            let head = || document.as_ref().map(|d| d.head().number());
            let left = call_made.as_ref().map(|m| m.last.number());
            result.version = Some(
                left.or_else(|| if mutates { head() } else { None })
                    .unwrap_or_else(|| opened.number()),
            );
        }
        let mut warnings = state.warnings;
        warnings.append(&mut result.warnings);
        result.warnings = warnings;
        result.app.tool = name.to_owned();
        InBatch {
            result,
            made: call_made,
            deferred: std::mem::take(state.deferred.get_mut().unwrap_or_else(PoisonError::into_inner)),
        }
    }

    /// The run of `name` with `arguments` on `opened`, as its format is handed it.
    fn format_run<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a Map<String, Value>,
        opened: &'a Opened,
        state: &'a CallState<'_>,
        context: &'a CallContext,
        cancel: &'a CallCancel<'a>,
    ) -> FormatRun<'a> {
        FormatRun {
            tool: name,
            arguments,
            doc: opened,
            named: &state.named,
            others: &state.others,
            context,
            cancel,
            date: &state.date,
            locale: context.locale.as_ref().or(self.config.locale.as_ref()),
            made: &state.made,
            txn: state.txn,
            deferred: &state.deferred,
        }
    }

    fn dispatch(
        &self,
        name: &str,
        arguments: Value,
        context: &CallContext,
        cancel: &CallCancel<'_>,
        state: &mut CallState<'_>,
    ) -> ToolResult<Value> {
        let found = self.found(name, arguments, cancel, state);
        let (tool, arguments) = match found {
            Ok(found) => found,
            Err(error) => return ToolResult::failed(error),
        };
        let Some(doc) = state.opened.as_ref() else {
            return ToolResult::failed(ToolError::internal("no document", "session:dispatch"));
        };
        let document = match self.document(&doc.id) {
            Ok(document) => document,
            Err(error) => return ToolResult::failed(error.to_tool_error()),
        };
        let run = self.format_run(name, &arguments, doc, state, context, cancel);
        document.doc.run(&*tool, &run)
    }

    /// The tool a call names, its arguments as meant, and the document it reads (kept in
    /// `state`).
    fn found(
        &self,
        name: &str,
        arguments: Value,
        cancel: &dyn Cancel,
        state: &mut CallState<'_>,
    ) -> Result<Found, ToolError> {
        cancel.check().map_err(|e| SessionError::from(e).to_tool_error())?;
        let spec = self.spec_named(name)?;
        let Accepted { mut arguments, notes } = match arguments {
            Value::Object(arguments) if state.accepted => Accepted {
                arguments,
                notes: Vec::new(),
            },
            arguments => spec.accept(arguments)?,
        };
        state.warnings.extend(notes);
        let (named, doc) = self.document_of(spec, &arguments, state.pinned.as_deref_mut())?;
        state.others = self.others_of(spec, &arguments, &doc.id, state.pinned.as_deref_mut())?;
        state.named = named;
        let tool = self
            .registry
            .get(name, doc.kind)
            .ok_or_else(|| self.not_served(name, doc.kind))?;
        state
            .warnings
            .extend(spec.ignored_for(Some(doc.kind.as_str()), &mut arguments));
        state.opened = Some(doc);
        Ok((Arc::clone(tool), arguments))
    }

    /// One op of a batch `within`: a tool that may run there, its argument errors fielded
    /// under `arguments` as today's batches field them.
    fn dispatch_within(
        &self,
        name: &str,
        arguments: Value,
        context: &CallContext,
        cancel: &CallCancel<'_>,
        state: &mut CallState<'_>,
        within: &str,
    ) -> ToolResult<Value> {
        let runs = self.registry.named(name).map(|t| t.spec().flags);
        if name == READ_BATCH || name == UNDO || runs.is_some_and(|f| !(f.batchable || f.read_batchable)) {
            let way_out = match WayOut::names([name]) {
                Some(way_out) => way_out,
                None => return ToolResult::failed(ToolError::internal("no name", "session:batch")),
            };
            return ToolResult::failed(
                ToolError::refusal(
                    Refusal::InvalidArguments,
                    format!("{name} cannot run inside a {within}; call it on its own"),
                    way_out,
                )
                .at("tool"),
            );
        }
        if runs.is_none() {
            return match self.spec_named(name) {
                Ok(_) => ToolResult::done(ooxml_tools::Done::new("")),
                Err(e) => ToolResult::failed(e.at("tool")),
            };
        }
        let mut result = self.dispatch(name, arguments, context, cancel, state);
        if let ooxml_tools::Outcome::Failed(error) = &mut result.outcome {
            let field = error.field().map(|f| format!("arguments.{f}"));
            if let Some(field) = field.filter(|_| error.code().as_str() == "invalid_arguments") {
                *error = error.clone().at(field);
            }
        }
        result
    }

    /// The definition of `name` (the first registered: tools of one name share it).
    fn spec_named(&self, name: &str) -> Result<&ToolSpec, ToolError> {
        if let Some(tool) = self.registry.named(name) {
            return Ok(tool.spec());
        }
        let names = self.registry.names();
        let message = format!("no tool {}", ooxml_tools::pyfmt::repr_str(name));
        let way_out =
            WayOut::names(names).ok_or_else(|| ToolError::internal("the session has no tools", "session:registry"))?;
        Err(ToolError::refusal(Refusal::InvalidArguments, message, way_out))
    }

    /// `name` refused for a `kind` document, with the documents it serves.
    fn not_served(&self, name: &str, kind: DocKind) -> ToolError {
        let table = self.read_table();
        let open: Vec<String> = table
            .iter()
            .filter(|(_, d)| self.registry.get(name, d.kind).is_some())
            .map(|(id, _)| id.to_string())
            .collect();
        drop(table);
        let message = format!("{name} does not serve {kind} documents yet");
        match WayOut::names(open) {
            Some(way_out) => ToolError::refusal(Refusal::Refused, message, way_out).at("doc"),
            None => ToolError::failure(ooxml_tools::Failure::Internal, message).at("doc"),
        }
    }
}
