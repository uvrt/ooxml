//! Owns: the dispatch of a tool call ([`Session::call`], [`Session::edit`], a batch's op):
//! the arguments taken as meant, the tool and its document found, the handler run on the
//! versions the call's context names (else on the documents' heads), and what the call made
//! -- its versions ([`CallMade`]), the ids it retired, who made its step -- noted; `undo` and
//! `read_batch` handed to the session's own.
//!
//! Must not: hold a lock while a handler runs, or implement a tool.

use crate::batch::{MAX_READ_BATCH_CHARS, READ_BATCH};
use crate::call::Call;
use crate::context::{revision_date, CallCancel, LayoutCancel};
use crate::document::{At, Document};
use crate::session::Opened;
use crate::undo::UNDO;
use crate::{CallContext, DocId, DocKind, DocNames, Session, SessionError};
use ooxml_base::Cancel;
use ooxml_tools::{Accepted, Outcome, Refusal, ToolError, ToolResult, ToolSpec, WayOut};
use ooxml_tools_word::{Made, Placer};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

/// A call's tool and its arguments as meant.
type Found = (Arc<dyn crate::Tool>, Map<String, Value>);

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
    /// The host's names for the documents the call names.
    names: DocNames,
    /// The name the call gives its own document.
    named: String,
    placer: Option<Arc<dyn Placer>>,
    /// The layout of the call's document: the host's where it gives one, else the
    /// session's own engine of the document (set once the document is known).
    layout: Option<Arc<dyn ooxml_tools_word::WordLayout>>,
    /// The versions an editing tool's edits made.
    made: Mutex<Option<Made>>,
    /// The call's limit on laying a document out.
    layout_limit: Option<ooxml_tools_word::LayoutLimit<'p>>,
}

/// The versions one changing call made ([`Session::edit`]): its first edit's number and its
/// last edit's version, held -- the call's own, whatever other calls committed meanwhile.
#[derive(Clone)]
pub struct CallMade {
    /// The number of the version its first edit made.
    pub first: u64,
    /// The state number of the version its first edit was applied to ([`Opened::number`]):
    /// the state its step goes from.
    pub before: u64,
    /// The version its last edit made.
    pub last: Opened,
    /// What the call's edit touched (`touched` entries), where it made one version (else
    /// `None`: [`Opened::touched_since`] the version before `first` says).
    pub touched: Option<Vec<Value>>,
    /// The validation delta of `last` against the problems the document opened with.
    pub validation: ooxml_word_edit::validate::Delta,
    /// The reflow its checks report (docx-agent's `reflow_json` form, or `"stale: call
    /// check"`, or a `"timeout: …"` where laying `last` out passed the call's limit), where
    /// the call had the host's layout and the version its first edit was applied to was laid
    /// out (a check saw it, or the engine laid it out last); else `None`.
    pub reflow: Option<Value>,
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
            if result.is_error() {
                self.counts.failed.fetch_add(1, Ordering::Relaxed);
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
                self.counts.failed.fetch_add(1, Ordering::Relaxed);
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
        self.counts.calls.fetch_add(1, Ordering::Relaxed);
        let cancel = CallCancel::new(context, &*self.context.cancel, &*self.context.clock);
        let layout_cancel = cancel.layout();
        let mut state = CallState {
            opened: None,
            warnings: Vec::new(),
            pinned,
            others: Vec::new(),
            names: context.names.clone(),
            named: String::new(),
            placer: context.placer.clone(),
            layout: context.layout.clone(),
            made: Mutex::new(None),
            layout_limit: layout_cancel.as_ref().map(LayoutCancel::limit),
        };
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
            self.counts.failed.fetch_add(1, Ordering::Relaxed);
            if error.code().as_str() == "timeout" {
                self.counts.cancelled.fetch_add(1, Ordering::Relaxed);
            }
        }
        let CallState {
            opened,
            mut warnings,
            made,
            named,
            layout_limit,
            ..
        } = state;
        let made = made.into_inner().unwrap_or_else(PoisonError::into_inner);
        let mut call_made = None;
        if let Some(opened) = opened {
            result.doc = Some(named);
            // a changing call's version is the last it made (else the head it left); a
            // reading call's, the one read
            let mutates = self.registry.named(name).is_some_and(|t| t.spec().flags.mutates);
            let document = self.document(&opened.id).ok();
            if let (Some(made), Some(document)) = (&made, &document) {
                Self::noted(made, document, name, context);
            }
            call_made = made.zip(document.as_ref()).map(|(made, document)| {
                let one = made.first.seq() == made.last.seq();
                // (the host's layout only: a call it gives none is not measured after its
                // edits, as before the session kept engines of its own)
                let reflow = context
                    .layout
                    .as_deref()
                    .and_then(|layout| crate::reflowed::after_edit(layout, document, &made, (layout_limit, &cancel)));
                CallMade {
                    reflow,
                    first: made.first.seq().0,
                    before: made.before.number(),
                    touched: one.then(|| made.touched.iter().map(crate::changes::touched_json).collect()),
                    validation: made.validation,
                    last: Self::opened(&opened.id, document, At::Word(made.last)),
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
        warnings.append(&mut result.warnings);
        result.warnings = warnings;
        result.app.tool = name.to_owned();
        (result, call_made)
    }

    /// What `name`'s edits `made` to `document` noted: the ids each version retired, by
    /// whom, and the call's step and who made it (unless a step group holds it).
    fn noted(made: &Made, document: &Document, name: &str, context: &CallContext) {
        if let Some(word) = document.word() {
            for (seq, delta) in &made.ids {
                let by = crate::retired::Made {
                    seq: seq.0,
                    caller: context.caller.name(),
                    tool: Some(name),
                    history: false,
                };
                word.retired.note(&by, delta);
            }
        }
        if !document.grouped() {
            document.stepped(made.last.seq().0, made.last.number(), Some(name), context);
        }
    }

    fn dispatch(
        &self,
        name: &str,
        arguments: Value,
        context: &CallContext,
        cancel: &dyn Cancel,
        state: &mut CallState<'_>,
    ) -> ToolResult<Value> {
        let found = self.found(name, arguments, cancel, state);
        let (tool, arguments) = match found {
            Ok(found) => found,
            Err(error) => return ToolResult::failed(error),
        };
        let given = arguments.clone();
        let Some(doc) = state.opened.as_ref() else {
            return ToolResult::failed(ToolError::internal("no document", "session:dispatch"));
        };
        let document = self.document(&doc.id).ok();
        let call = Call {
            doc,
            name: &state.named,
            word: document.as_deref().and_then(Document::word),
            caller: &context.caller,
            cancel,
            date: context
                .date
                .clone()
                .unwrap_or_else(|| revision_date(self.context.clock.now())),
            setup: ooxml_word_edit::Setup {
                as_given: context.as_given,
            },
            locale: context.locale.as_ref().or(self.config.locale.as_ref()),
            made: &state.made,
            layout: state.layout.as_deref(),
            layout_limit: state.layout_limit,
            raster: context.raster.as_deref(),
            image_budgets: &context.image_budgets,
            blobs: context.blobs.as_deref(),
            written: &context.written,
            others: &state.others,
            tables: context.tables.as_deref(),
            sources: &context.sources,
            counts: context.counts,
            limits: context.limits,
        };
        let result = tool.run(arguments, &call);
        // An id a version removed, refused as one the document lacks: said as such.
        match (&result.outcome, call.word) {
            (Outcome::Failed(error), Some(word)) => {
                let replaced = call.doc.word().map(|w| w.ids().replaced());
                let none = BTreeMap::new();
                crate::refused::removed(&word.retired, replaced.unwrap_or(&none), &given, error)
                    .map_or(result, ToolResult::failed)
            }
            _ => result,
        }
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
        let Accepted { mut arguments, notes } = spec.accept(arguments)?;
        state.warnings.extend(notes);
        let (named, doc) = self.document_of(spec, &arguments, &state.names, state.pinned.as_deref_mut())?;
        state.others = self.others_of(spec, &arguments, &doc.id, &state.names, state.pinned.as_deref_mut())?;
        state.named = named;
        let tool = self
            .registry
            .get(name, doc.kind)
            .ok_or_else(|| self.not_served(name, doc.kind, &state.names))?;
        state
            .warnings
            .extend(spec.ignored_for(Some(doc.kind.as_str()), &mut arguments));
        if state.layout.is_none() {
            state.layout = self.own_pages(&doc.id);
        }
        state.opened = Some(doc.with_placer(state.placer.clone()));
        Ok((Arc::clone(tool), arguments))
    }

    /// One op of a batch `within`: a tool that may run there, its argument errors fielded
    /// under `arguments` as today's batches field them.
    fn dispatch_within(
        &self,
        name: &str,
        arguments: Value,
        context: &CallContext,
        cancel: &dyn Cancel,
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

    /// `name` refused for a `kind` document, with the documents it serves, as the call
    /// names documents (`names`).
    fn not_served(&self, name: &str, kind: DocKind, names: &DocNames) -> ToolError {
        let table = self.read_table();
        let open = names.of(table
            .iter()
            .filter(|(_, d)| self.registry.get(name, d.kind).is_some())
            .map(|(id, _)| id));
        drop(table);
        let message = format!("{name} does not serve {kind} documents yet");
        match WayOut::names(open) {
            Some(way_out) => ToolError::refusal(Refusal::Refused, message, way_out).at("doc"),
            None => ToolError::failure(ooxml_tools::Failure::Internal, message).at("doc"),
        }
    }
}
