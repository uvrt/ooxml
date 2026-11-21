//! Owns: [`Session`], documents by id with their stores, and the operations on them: open,
//! save, close, the bridge's update, snapshots, and dispatch of a tool call (on the
//! versions its context names, else on the documents' heads).
//!
//! Must not: hold a lock while a handler runs or a package is parsed or written (the table
//! lock is held only to find or change an entry), or implement a tool.

use crate::batch::{MAX_READ_BATCH_CHARS, READ_BATCH};
use crate::call::Call;
use crate::context::{revision_date, CallCancel, LayoutCancel};
use crate::document::{At, Document};
use crate::update::{HostIds, PartUpdate, Updated};
use crate::{CallContext, DocId, DocKind, DocNames, Registry, SessionError};
use ooxml_base::{Cancel, Context};
use ooxml_opc::Package;
use ooxml_store::{StoreConfig, StoreStats};
use ooxml_tools::{Accepted, Outcome, Refusal, ToolError, ToolResult, ToolSpec, WayOut};
use ooxml_tools_word::{Made, Placer, Version};
use ooxml_word_edit::EarlierIds;
use ooxml_word_model::docx::Versions;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

/// A session's configuration, a value given when it is made.
#[derive(Debug, Clone, Copy, Default)]
pub struct SessionConfig {
    /// Each document's store.
    pub store: StoreConfig,
}

/// A document at one version, as a call reads it: it stays readable, unchanged, while
/// others commit, and after the document is closed.
#[derive(Clone)]
pub struct Opened {
    id: DocId,
    kind: DocKind,
    name: Arc<str>,
    version: At,
    versions: Arc<Versions>,
    /// Where the call reading it may ask where blocks are laid out (its context's).
    placer: Option<Arc<dyn Placer>>,
}

impl Opened {
    /// The document's id.
    #[must_use]
    pub fn id(&self) -> &DocId {
        &self.id
    }

    /// Its kind.
    #[must_use]
    pub fn kind(&self) -> DocKind {
        self.kind
    }

    /// Its name, as opened.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The version's number.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version.seq()
    }

    /// The package at this version.
    #[must_use]
    pub fn package(&self) -> &Package {
        self.version.package()
    }

    /// The Word document at this version, when the session owns it as one.
    #[must_use]
    pub fn word(&self) -> Option<&ooxml_word_edit::WordDoc> {
        match &self.version {
            At::Word(version) => Some(version.doc()),
            At::Package(_) => None,
        }
    }

    /// This version as a Word reading tool is handed it.
    #[must_use]
    pub fn version_of(&self) -> Version<'_> {
        Version::new(&self.name, self.package(), self.version(), &self.versions)
            .with_placer(self.placer.as_deref())
            .with_word(self.word())
    }
}

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
    /// The version its last edit made.
    pub last: Opened,
    /// What the call's edit touched (`touched` entries), where it made one version (else
    /// `None`: [`Opened::touched_since`] the version before `first` says).
    pub touched: Option<Vec<Value>>,
    /// The validation delta of `last` against the problems the document opened with.
    pub validation: ooxml_word_edit::validate::Delta,
}

/// The versions `context` names, by document.
pub(crate) fn pinned_by(context: &CallContext) -> Pinned {
    context.at.iter().map(|o| (o.id.clone(), o.clone())).collect()
}

/// A copy of a session's counters (section 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SessionStats {
    /// Tool calls dispatched.
    pub calls: u64,
    /// Calls that failed or were refused.
    pub failed: u64,
    /// Calls and updates stopped by cancellation or a deadline.
    pub cancelled: u64,
    /// Bridge updates committed with a change.
    pub updates: u64,
    /// Documents open now.
    pub open: u64,
}

#[derive(Default)]
struct Counts {
    calls: AtomicU64,
    failed: AtomicU64,
    cancelled: AtomicU64,
    updates: AtomicU64,
    /// `bridge_calls{tool}` (section 13): unported tools' calls committed through the bridge.
    bridged: std::sync::Mutex<BTreeMap<String, u64>>,
}

/// Documents by id, their versions, and the tools that read and change them.
pub struct Session {
    config: SessionConfig,
    pub(crate) context: Context,
    registry: Registry,
    documents: RwLock<BTreeMap<DocId, Arc<Document>>>,
    last_id: AtomicU64,
    counts: Counts,
}

impl Session {
    /// A session with `context`'s ports and configuration and the standard tools.
    #[must_use]
    pub fn new(config: SessionConfig, context: Context) -> Self {
        Self::with_registry(config, context, Registry::standard())
    }

    /// A session dispatching `registry`'s tools.
    #[must_use]
    pub fn with_registry(config: SessionConfig, context: Context, registry: Registry) -> Self {
        Self {
            config,
            context,
            registry,
            documents: RwLock::new(BTreeMap::new()),
            last_id: AtomicU64::new(0),
            counts: Counts::default(),
        }
    }

    /// The tools.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Open a package of `kind` from its bytes, as `name`. A Word document is read (the
    /// session owns it: its ids, the problems it opened with); another kind's parts are not
    /// inflated or parsed until read.
    ///
    /// # Errors
    /// [`SessionError::Package`] when the bytes are not a package within the limits;
    /// [`SessionError::Word`] when a Word document cannot be read.
    pub fn open(&self, bytes: Arc<[u8]>, kind: DocKind, name: &str) -> Result<DocId, SessionError> {
        self.open_with(bytes, kind, name, &EarlierIds::default())
    }

    /// [`Session::open`], a Word document opened again with the ids it had before it was
    /// saved (the host's state of it, [`EarlierIds`]): it never gives a retired id again.
    ///
    /// # Errors
    /// As [`Session::open`].
    pub fn open_with(
        &self,
        bytes: Arc<[u8]>,
        kind: DocKind,
        name: &str,
        earlier: &EarlierIds,
    ) -> Result<DocId, SessionError> {
        let package = Package::open(bytes, &self.context.limits)?;
        let document = Arc::new(Document::open(
            package,
            kind,
            name,
            self.config.store,
            &self.context,
            earlier,
        )?);
        let id = DocId::nth(self.last_id.fetch_add(1, Ordering::Relaxed) + 1);
        self.write_table().insert(id.clone(), document);
        Ok(id)
    }

    /// The document `id` at its head version. Never waits for a writer.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn snapshot(&self, id: &DocId) -> Result<Opened, SessionError> {
        let document = self.document(id)?;
        Ok(Self::opened(id, &document, document.head()))
    }

    pub(crate) fn opened(id: &DocId, document: &Document, version: At) -> Opened {
        Opened {
            id: id.clone(),
            kind: document.kind,
            name: Arc::from(document.name.as_str()),
            version,
            versions: Arc::clone(&document.versions),
            placer: None,
        }
    }

    /// The document's bytes at its head version: untouched entries copied raw.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`], or the package's error writing a part.
    pub fn save(&self, id: &DocId) -> Result<Vec<u8>, SessionError> {
        Ok(self.snapshot(id)?.package().save()?)
    }

    /// The save gate (section 13, P2): the document's bytes at its head version, refused
    /// when a Word document has validation problems it did not open with.
    ///
    /// # Errors
    /// [`SessionError::NewProblems`], with every new problem; as [`Session::save`].
    pub fn save_checked(&self, id: &DocId) -> Result<Vec<u8>, SessionError> {
        let document = self.document(id)?;
        let head = document.head();
        if let (Some(word), At::Word(version)) = (document.word(), &head) {
            let delta = word.validation(version)?;
            if !delta.new.is_empty() {
                return Err(SessionError::NewProblems { new: delta.new });
            }
        }
        Ok(head.package().save()?)
    }

    /// What the save gate compares for a Word document at its head: its problems now
    /// against those it opened with (`None` for another kind).
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; the edit core's when a version cannot be read.
    pub fn validation(&self, id: &DocId) -> Result<Option<ooxml_word_edit::validate::Delta>, SessionError> {
        let document = self.document(id)?;
        match (document.word(), document.head()) {
            (Some(word), At::Word(version)) => Ok(Some(word.validation(&version)?)),
            _ => Ok(None),
        }
    }

    /// What the Word document's last check with `reflow` saw (`since_last_check` is
    /// measured from it), read and kept by a host's own checks too; `None` for another kind.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn last_check(&self, id: &DocId) -> Result<Option<Arc<ooxml_tools_word::LastCheck>>, SessionError> {
        Ok(self.document(id)?.word().map(|word| Arc::clone(&word.last_check)))
    }

    /// [`Session::validation`] of the version `at` (one a call made, say) instead of the head.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; the edit core's when the version cannot be read.
    pub fn validation_at(&self, at: &Opened) -> Result<Option<ooxml_word_edit::validate::Delta>, SessionError> {
        let document = self.document(&at.id)?;
        match (document.word(), &at.version) {
            (Some(word), At::Word(version)) => Ok(Some(word.validation(version)?)),
            _ => Ok(None),
        }
    }

    /// Close the document: the session lets go of it and its history at once; a snapshot
    /// still held by a reader stays valid until it is dropped.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn close(&self, id: &DocId) -> Result<(), SessionError> {
        let removed = self.write_table().remove(id);
        removed.map(drop).ok_or_else(|| self.not_open(&id.to_string()))
    }

    /// The ids of the open documents, in the order they were opened.
    #[must_use]
    pub fn documents(&self) -> Vec<DocId> {
        self.read_table().keys().cloned().collect()
    }

    /// Commit `parts` (bytes to put, `None` to remove) to the document as one step, made
    /// by `context.caller`: the bridge's way in for a version whose owner is elsewhere
    /// (section 13). XML parts are parsed here, on a snapshot, without any lock; the
    /// commit is the store's.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; a part that does not parse or a removal of a part that
    /// is not there ([`SessionError::Package`]); cancellation ([`SessionError::Base`]).
    pub fn update(
        &self,
        id: &DocId,
        parts: Vec<(String, Option<Arc<[u8]>>)>,
        context: &CallContext,
    ) -> Result<Updated, SessionError> {
        self.commit(id, parts, context).map(|(updated, _)| updated)
    }

    /// The bridge (section 13): commit the parts an unported tool `tool` changed, run by the
    /// host on the head's bytes, as one step made by `context.caller`, and count it in
    /// `bridge_calls{tool}`; `host`: what the host's ledger holds beside the parts (its
    /// tool's draws of new ids and renames, [`HostIds`]). The version it leaves, held.
    ///
    /// # Errors
    /// As [`Session::update`]; a conflict with a commit since the head the tool ran on is
    /// prepared again on the newer head (the update reads the whole document).
    pub fn bridge(
        &self,
        id: &DocId,
        tool: &str,
        parts: Vec<(String, Option<Arc<[u8]>>)>,
        host: HostIds,
        context: &CallContext,
    ) -> Result<(Updated, Opened), SessionError> {
        let done = self.commit_by(id, Some(tool), parts, host, context)?;
        let mut counts = self.counts.bridged.lock().unwrap_or_else(PoisonError::into_inner);
        *counts.entry(tool.to_owned()).or_default() += 1;
        Ok(done)
    }

    /// `bridge_calls{tool}`: how many calls of each unported tool went through the bridge.
    #[must_use]
    pub fn bridge_calls(&self) -> BTreeMap<String, u64> {
        self.counts
            .bridged
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// [`Session::update`], and the version it leaves (the one it ran on where it changed
    /// nothing), held: a call whose context names it reads that version, whatever was
    /// committed since (the host that owns the document hands each version over and reads
    /// it so).
    ///
    /// # Errors
    /// As [`Session::update`].
    pub fn commit(
        &self,
        id: &DocId,
        parts: Vec<(String, Option<Arc<[u8]>>)>,
        context: &CallContext,
    ) -> Result<(Updated, Opened), SessionError> {
        self.commit_by(id, None, parts, HostIds::default(), context)
    }

    /// [`Session::commit`] of the parts `tool` changed (`None`: no tool named).
    fn commit_by(
        &self,
        id: &DocId,
        tool: Option<&str>,
        parts: Vec<(String, Option<Arc<[u8]>>)>,
        host: HostIds,
        context: &CallContext,
    ) -> Result<(Updated, Opened), SessionError> {
        let document = self.document(id)?;
        let cancel = CallCancel::new(context, &*self.context.cancel, &*self.context.clock);
        let update = PartUpdate::new(parts, self.context.limits, &cancel).with_host(host);
        let (version, changed, attempts) = document.update(&context.caller, tool, &update).inspect_err(|e| {
            if matches!(e, SessionError::Base(ooxml_base::BaseError::Cancelled)) {
                self.counts.cancelled.fetch_add(1, Ordering::Relaxed);
            }
        })?;
        if changed {
            self.counts.updates.fetch_add(1, Ordering::Relaxed);
        }
        let updated = Updated {
            version: version.seq(),
            changed,
            attempts,
        };
        Ok((updated, Self::opened(id, &document, version)))
    }

    /// The store's counters for the document (commits, conflicts, exclusive runs).
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn store_stats(&self, id: &DocId) -> Result<StoreStats, SessionError> {
        Ok(self.document(id)?.stats())
    }

    /// The session's counters.
    #[must_use]
    pub fn stats(&self) -> SessionStats {
        SessionStats {
            calls: self.counts.calls.load(Ordering::Relaxed),
            failed: self.counts.failed.load(Ordering::Relaxed),
            cancelled: self.counts.cancelled.load(Ordering::Relaxed),
            updates: self.counts.updates.load(Ordering::Relaxed),
            open: self.read_table().len() as u64,
        }
    }

    /// Run the tool `name` with `arguments` (a JSON object, or its text) under `context`:
    /// the arguments taken as meant and validated, the document found, the handler run on
    /// the version `context` names for it, else on its head snapshot. Never panics and
    /// never fails: what went wrong is the result's error, with its way out.
    pub fn call(&self, name: &str, arguments: Value, context: &CallContext) -> ToolResult<Value> {
        if name == READ_BATCH {
            return self.call_batch(arguments, context);
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
            made: Mutex::new(None),
            layout_limit: layout_cancel.as_ref().map(LayoutCancel::limit),
        };
        let mut result = if within.is_empty() {
            self.dispatch(name, arguments, context, &cancel, &mut state)
        } else {
            self.dispatch_within(name, arguments, context, &cancel, &mut state, within)
        };
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
            if let (Some(made), Some(word)) = (&made, document.as_deref().and_then(Document::word)) {
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
            call_made = made.zip(document.as_ref()).map(|(made, document)| {
                let one = made.first.seq() == made.last.seq();
                CallMade {
                    first: made.first.seq().0,
                    touched: one.then(|| made.touched.iter().map(crate::changes::touched_json).collect()),
                    validation: made.validation,
                    last: Self::opened(&opened.id, document, At::Word(made.last)),
                }
            });
            let head = || document.as_ref().map(|d| d.head().seq());
            let left = call_made.as_ref().map(|m| m.last.version());
            result.version = Some(
                left.or_else(|| if mutates { head() } else { None })
                    .unwrap_or_else(|| opened.version()),
            );
        }
        warnings.append(&mut result.warnings);
        result.warnings = warnings;
        result.app.tool = name.to_owned();
        (result, call_made)
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
            made: &state.made,
            layout: context.layout.as_deref(),
            layout_limit: state.layout_limit,
            blobs: context.blobs.as_deref(),
            written: &context.written,
            others: &state.others,
            limits: context.limits,
        };
        let result = tool.run(arguments, &call);
        // An id a version removed, refused as one the document lacks: said as such.
        match (&result.outcome, call.word) {
            (Outcome::Failed(error), Some(word)) => {
                let replaced = call.doc.word().map(|w| w.ids().replaced());
                let none = std::collections::BTreeMap::new();
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
        state.opened = Some(Opened {
            placer: state.placer.clone(),
            ..doc
        });
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
        if name == READ_BATCH || runs.is_some_and(|f| !(f.batchable || f.read_batchable)) {
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

    pub(crate) fn document(&self, id: &DocId) -> Result<Arc<Document>, SessionError> {
        self.read_table()
            .get(id)
            .cloned()
            .ok_or_else(|| self.not_open(&id.to_string()))
    }

    pub(crate) fn not_open(&self, id: &str) -> SessionError {
        SessionError::NotOpen {
            id: id.to_owned(),
            open: self.read_table().keys().map(ToString::to_string).collect(),
        }
    }

    // A poisoned table is still consistent: every change to it is one insert or remove.
    fn read_table(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<DocId, Arc<Document>>> {
        self.documents.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write_table(&self) -> std::sync::RwLockWriteGuard<'_, BTreeMap<DocId, Arc<Document>>> {
        self.documents.write().unwrap_or_else(PoisonError::into_inner)
    }
}
