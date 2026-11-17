//! Owns: [`Session`], documents by id with their stores, and the operations on them: open,
//! save, close, the bridge's update, snapshots, and dispatch of a tool call.
//!
//! Must not: hold a lock while a handler runs or a package is parsed or written (the table
//! lock is held only to find or change an entry), or implement a tool.

use crate::context::{CallCancel, StoreClock};
use crate::update::{PartUpdate, Updated};
use crate::{CallContext, DocId, DocKind, PackageDoc, Registry, SessionError};
use ooxml_base::{Cancel, Context};
use ooxml_opc::Package;
use ooxml_store::{Snapshot, Store, StoreConfig, StoreStats};
use ooxml_tools::{Accepted, Refusal, ToolError, ToolResult, ToolSpec, WayOut};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

/// A session's configuration, a value given when it is made.
#[derive(Debug, Clone, Copy, Default)]
pub struct SessionConfig {
    /// Each document's store.
    pub store: StoreConfig,
}

/// One open document.
struct Document {
    kind: DocKind,
    name: String,
    store: Store<PackageDoc>,
}

/// A document at one version, as a call reads it: it stays readable, unchanged, while
/// others commit, and after the document is closed.
#[derive(Clone)]
pub struct Opened {
    id: DocId,
    kind: DocKind,
    name: Arc<str>,
    version: Snapshot<PackageDoc>,
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
        self.version.seq().0
    }

    /// The package at this version.
    #[must_use]
    pub fn package(&self) -> &Package {
        self.version.doc().package()
    }
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
}

/// Documents by id, their versions, and the tools that read and change them.
pub struct Session {
    config: SessionConfig,
    context: Context,
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

    /// Open a package of `kind` from its bytes, as `name`. Nothing is inflated or parsed
    /// until read.
    ///
    /// # Errors
    /// [`SessionError::Package`] when the bytes are not a package within the limits.
    pub fn open(&self, bytes: Arc<[u8]>, kind: DocKind, name: &str) -> Result<DocId, SessionError> {
        let package = Package::open(bytes, &self.context.limits)?;
        let store = Store::new(
            PackageDoc::new(package),
            self.config.store,
            Arc::new(StoreClock(Arc::clone(&self.context.clock))),
        );
        let id = DocId::nth(self.last_id.fetch_add(1, Ordering::Relaxed) + 1);
        let document = Arc::new(Document {
            kind,
            name: name.to_owned(),
            store,
        });
        self.write_table().insert(id.clone(), document);
        Ok(id)
    }

    /// The document `id` at its head version. Never waits for a writer.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn snapshot(&self, id: &DocId) -> Result<Opened, SessionError> {
        let document = self.document(id)?;
        Ok(Opened {
            id: id.clone(),
            kind: document.kind,
            name: Arc::from(document.name.as_str()),
            version: document.store.snapshot(),
        })
    }

    /// The document's bytes at its head version: untouched entries copied raw.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`], or the package's error writing a part.
    pub fn save(&self, id: &DocId) -> Result<Vec<u8>, SessionError> {
        Ok(self.snapshot(id)?.package().save()?)
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
        let document = self.document(id)?;
        let cancel = CallCancel::new(context, &*self.context.cancel, &*self.context.clock);
        let update = PartUpdate::new(parts, self.context.limits, &cancel);
        let committed = document.store.run(&context.caller, &update).inspect_err(|e| {
            if matches!(e, SessionError::Base(ooxml_base::BaseError::Cancelled)) {
                self.counts.cancelled.fetch_add(1, Ordering::Relaxed);
            }
        })?;
        if committed.changed {
            self.counts.updates.fetch_add(1, Ordering::Relaxed);
        }
        Ok(Updated {
            version: committed.version.seq().0,
            changed: committed.changed,
            attempts: committed.attempts,
        })
    }

    /// The store's counters for the document (commits, conflicts, exclusive runs).
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn store_stats(&self, id: &DocId) -> Result<StoreStats, SessionError> {
        Ok(self.document(id)?.store.stats())
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
    /// its head snapshot. Never panics and never fails: what went wrong is the result's
    /// error, with its way out.
    pub fn call(&self, name: &str, arguments: Value, context: &CallContext) -> ToolResult<Value> {
        self.counts.calls.fetch_add(1, Ordering::Relaxed);
        let cancel = CallCancel::new(context, &*self.context.cancel, &*self.context.clock);
        let mut opened = None;
        let mut warnings = Vec::new();
        let outcome = self.dispatch(name, arguments, &cancel, &mut opened, &mut warnings);
        let mut result = match outcome {
            Ok(done) => ToolResult::done(done),
            Err(error) => {
                self.counts.failed.fetch_add(1, Ordering::Relaxed);
                if error.code().as_str() == "timeout" {
                    self.counts.cancelled.fetch_add(1, Ordering::Relaxed);
                }
                ToolResult::failed(error)
            }
        };
        if let Some(opened) = opened {
            result.doc = Some(opened.id.to_string());
            result.version = Some(opened.version());
        }
        result.warnings = warnings;
        result.app.tool = name.to_owned();
        result
    }

    fn dispatch(
        &self,
        name: &str,
        arguments: Value,
        cancel: &dyn Cancel,
        opened: &mut Option<Opened>,
        warnings: &mut Vec<String>,
    ) -> Result<ooxml_tools::Done<Value>, ToolError> {
        cancel.check().map_err(|e| SessionError::from(e).to_tool_error())?;
        let spec = self.spec_named(name)?;
        let Accepted { mut arguments, notes } = spec.accept(arguments)?;
        warnings.extend(notes);
        let doc = self.document_of(spec, &arguments)?;
        let tool = self
            .registry
            .get(name, doc.kind)
            .ok_or_else(|| self.not_served(name, doc.kind))?;
        warnings.extend(spec.ignored_for(Some(doc.kind.as_str()), &mut arguments));
        let doc = opened.insert(doc);
        tool.run(arguments, doc, cancel)
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

    /// The document a call names (its first document argument), at its head.
    fn document_of(&self, spec: &ToolSpec, arguments: &Map<String, Value>) -> Result<Opened, ToolError> {
        let given = spec
            .documents
            .iter()
            .find_map(|name| arguments.get(name).and_then(Value::as_str));
        let Some(given) = given else {
            return Err(ToolError::internal(
                format!("{} names no document; the session serves only tools that do", spec.name),
                "session:documents",
            ));
        };
        let id = DocId::parse(given).ok_or_else(|| self.not_open(given).to_tool_error())?;
        self.snapshot(&id).map_err(|e| e.to_tool_error())
    }

    fn not_served(&self, name: &str, kind: DocKind) -> ToolError {
        let open: Vec<String> = self
            .read_table()
            .iter()
            .filter(|(_, d)| self.registry.get(name, d.kind).is_some())
            .map(|(id, _)| id.to_string())
            .collect();
        let message = format!("{name} does not serve {kind} documents yet");
        match WayOut::names(open) {
            Some(way_out) => ToolError::refusal(Refusal::Refused, message, way_out).at("doc"),
            None => ToolError::failure(ooxml_tools::Failure::Internal, message).at("doc"),
        }
    }

    fn document(&self, id: &DocId) -> Result<Arc<Document>, SessionError> {
        self.read_table()
            .get(id)
            .cloned()
            .ok_or_else(|| self.not_open(&id.to_string()))
    }

    fn not_open(&self, id: &str) -> SessionError {
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
