//! Owns: [`Session`], documents by id with their stores, and the operations on them: open,
//! save, close, an update of parts given as bytes, snapshots and counters. A tool call's dispatch is
//! [`crate::dispatch`]'s.
//!
//! Must not: hold a lock while a handler runs or a package is parsed or written (the table
//! lock is held only to find or change an entry), or implement a tool.

use crate::context::CallCancel;
use crate::document::Document;
use crate::format::AnyVersion;
use crate::format::{Format, FormatDoc, OpenEnv};
use crate::update::{PartUpdate, Updated};
use crate::{CallContext, DocId, DocKind, Registry, SessionError};
use ooxml_base::Context;
use ooxml_opc::Package;
use ooxml_store::{StoreConfig, StoreStats};
use std::any::Any;
use std::collections::hash_map::RandomState;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

/// A session's configuration, a value given when it is made.
#[derive(Debug, Clone, Default)]
pub struct SessionConfig {
    /// Each document's store.
    pub store: StoreConfig,
    /// The locale the host set for the session's calls (a call's own, in its
    /// [`CallContext::locale`], comes first); `None`: the host set none, and a call that names
    /// none reads each document in its own language.
    pub locale: Option<ooxml_numfmt::Locale>,
}

/// A document at one version, as a call reads it: it stays readable, unchanged, while
/// others commit, and after the document is closed.
#[derive(Clone)]
pub struct Opened {
    pub(crate) id: DocId,
    pub(crate) kind: DocKind,
    name: Arc<str>,
    version: AnyVersion,
    /// What its format keeps for reading its versions ([`FormatDoc::readings`]).
    readings: Arc<dyn Any + Send + Sync>,
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

    /// The state's number, as the tool layer numbers the document's states: a step a new
    /// number, an undo or redo the number of the state it brings back.
    #[must_use]
    pub fn number(&self) -> u64 {
        self.version.number()
    }

    /// The package at this version.
    #[must_use]
    pub fn package(&self) -> &Package {
        self.version.package()
    }

    /// The version itself, erased: its format's code reads it as its model.
    #[must_use]
    pub fn any(&self) -> &AnyVersion {
        &self.version
    }

    /// What the document's format keeps for reading its versions.
    #[must_use]
    pub fn readings(&self) -> &(dyn Any + Send + Sync) {
        &*self.readings
    }
}

/// A copy of a session's counters (section 10).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionStats {
    /// Tool calls dispatched.
    pub calls: u64,
    /// Calls that failed or were refused.
    pub failed: u64,
    /// Calls and updates stopped by cancellation or a deadline.
    pub cancelled: u64,
    /// Documents open now.
    pub open: u64,
    /// `panics{tool}`: the calls of each tool that panicked, each answered as an `internal`
    /// failure ([`crate::panics`]).
    pub panics: BTreeMap<String, u64>,
    /// `internal{tool}`: the calls of each tool answered as an `internal` failure -- a bug,
    /// never the caller's input (the edit core's `EditError::Internal` among them; a panic
    /// too, counted in `panics` as well).
    pub internal: BTreeMap<String, u64>,
}

#[derive(Default)]
pub(crate) struct Counts {
    pub(crate) calls: AtomicU64,
    pub(crate) failed: AtomicU64,
    pub(crate) cancelled: AtomicU64,
    /// `panics{tool}`.
    pub(crate) panics: std::sync::Mutex<BTreeMap<String, u64>>,
    /// `internal{tool}`.
    pub(crate) internal: std::sync::Mutex<BTreeMap<String, u64>>,
}

impl Counts {
    /// A call of `tool` that failed with `error`: counted as failed, cancelled when it timed
    /// out, internal when it is a bug.
    pub(crate) fn failed(&self, tool: &str, error: &ooxml_tools::ToolError) {
        self.failed.fetch_add(1, Ordering::Relaxed);
        match error.code().as_str() {
            "timeout" => {
                self.cancelled.fetch_add(1, Ordering::Relaxed);
            }
            "internal" => {
                let mut internal = self.internal.lock().unwrap_or_else(PoisonError::into_inner);
                *internal.entry(tool.to_owned()).or_insert(0) += 1;
            }
            _ => {}
        }
    }
}

/// Documents by id, their versions, and the tools that read and change them.
pub struct Session {
    pub(crate) config: SessionConfig,
    pub(crate) context: Context,
    pub(crate) registry: Registry,
    /// The kinds it opens, each its format.
    formats: Vec<Arc<dyn Format>>,
    documents: RwLock<BTreeMap<DocId, Arc<Document>>>,
    last_id: AtomicU64,
    /// The ids the application gave documents it opened: the counter never gives one.
    given: std::sync::Mutex<BTreeSet<DocId>>,
    pub(crate) counts: Counts,
    /// What tells this session's live-view tokens from another's (one made at the same
    /// moment, one before a restart): sixteen hex digits, its own.
    pub(crate) nonce: String,
}

impl Session {
    /// A session with `context`'s ports and configuration opening the kinds `formats` hold,
    /// with their tools.
    #[must_use]
    pub fn new(config: SessionConfig, context: Context, formats: Vec<Arc<dyn Format>>) -> Self {
        let registry = Registry::of(&formats);
        Self::with_registry(config, context, formats, registry)
    }

    /// A session opening the kinds `formats` hold and dispatching `registry`'s tools.
    #[must_use]
    pub fn with_registry(
        config: SessionConfig,
        context: Context,
        formats: Vec<Arc<dyn Format>>,
        registry: Registry,
    ) -> Self {
        Self {
            config,
            registry,
            formats,
            documents: RwLock::new(BTreeMap::new()),
            last_id: AtomicU64::new(0),
            given: std::sync::Mutex::new(BTreeSet::new()),
            counts: Counts::default(),
            nonce: nonce(&context),
            context,
        }
    }

    /// The tools.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The formats it opens, each a kind.
    #[must_use]
    pub fn formats(&self) -> &[Arc<dyn Format>] {
        &self.formats
    }

    /// The session's configuration.
    #[must_use]
    pub fn config(&self) -> &SessionConfig {
        &self.config
    }

    /// The session's ports.
    #[must_use]
    pub fn context(&self) -> &Context {
        &self.context
    }

    /// What tells this session's live-view tokens from another's: sixteen hex digits, its own.
    #[must_use]
    pub fn nonce(&self) -> &str {
        &self.nonce
    }

    /// What a document of `kind` opens with in this session.
    #[must_use]
    pub fn open_env(&self) -> OpenEnv<'_> {
        OpenEnv {
            store: self.config.store,
            context: &self.context,
        }
    }

    /// Open a package of `kind` from its bytes, as `name`, as its format opens it (a Word
    /// document is read: the session owns it, its ids, the problems it opened with; another
    /// kind's parts are not inflated or parsed until read).
    ///
    /// # Errors
    /// [`SessionError::Kind`] for a kind the session holds no format of;
    /// [`SessionError::Package`] when the bytes are not a package within the limits;
    /// [`SessionError::Format`] when the format cannot read it.
    pub fn open(&self, bytes: Arc<[u8]>, kind: DocKind, name: &str) -> Result<DocId, SessionError> {
        let format = self
            .formats
            .iter()
            .find(|f| f.kind() == kind)
            .ok_or_else(|| SessionError::Kind(kind.as_str().to_owned()))?;
        let package = Package::open(bytes, &self.context.limits)?;
        let doc = format.open(package, &self.open_env())?;
        Ok(self.insert(kind, name, doc))
    }

    /// Open a package of `kind` from its bytes, as `name`, as the application asks: under
    /// the id `id` it gives (the one a resumed run's saved conversation names) instead of the
    /// next one counted, and with the `state` saved beside the bytes
    /// ([`Format::open_with_state`]), its history's numbers going on from the state's.
    ///
    /// # Errors
    /// [`SessionError::IdTaken`] where a document has `id`; [`SessionError::StateRefused`];
    /// as [`Session::open`].
    pub fn open_as(
        &self,
        bytes: Arc<[u8]>,
        kind: DocKind,
        name: &str,
        id: Option<&DocId>,
        state: Option<&str>,
    ) -> Result<DocId, SessionError> {
        if let Some(id) = id {
            if self.read_table().contains_key(id) {
                return Err(SessionError::IdTaken(id.to_string()));
            }
        }
        let format = self
            .formats
            .iter()
            .find(|f| f.kind() == kind)
            .ok_or_else(|| SessionError::Kind(kind.as_str().to_owned()))?;
        let package = Package::open(bytes, &self.context.limits)?;
        let (doc, numbers) = match state {
            None => (format.open(package, &self.open_env())?, None),
            Some(state) => {
                let opened = format.open_with_state(package, state, &self.open_env())?;
                (opened.doc, opened.numbers)
            }
        };
        if let Some((version, highest)) = numbers {
            doc.stored().continue_numbers(version, highest)?;
        }
        self.insert_as(kind, name, doc, id.cloned())
    }

    /// Hold `doc`, a document of `kind` its format opened, as `name`: its new id.
    pub fn insert(&self, kind: DocKind, name: &str, doc: Arc<dyn FormatDoc>) -> DocId {
        let document = Arc::new(Document {
            kind,
            name: name.to_owned(),
            doc,
        });
        let mut table = self.write_table();
        let id = self.counted(&table);
        table.insert(id.clone(), document);
        id
    }

    /// Hold `doc` as [`Session::insert`] does, under `id` where one is given: the counter
    /// never gives it after.
    ///
    /// # Errors
    /// [`SessionError::IdTaken`] where a document has `id`.
    pub fn insert_as(
        &self,
        kind: DocKind,
        name: &str,
        doc: Arc<dyn FormatDoc>,
        id: Option<DocId>,
    ) -> Result<DocId, SessionError> {
        let Some(id) = id else {
            return Ok(self.insert(kind, name, doc));
        };
        let document = Arc::new(Document {
            kind,
            name: name.to_owned(),
            doc,
        });
        let mut table = self.write_table();
        if table.contains_key(&id) {
            return Err(SessionError::IdTaken(id.to_string()));
        }
        self.given
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.clone());
        table.insert(id.clone(), document);
        Ok(id)
    }

    /// The next id the counter gives: one never given before, by it or by the application
    /// (called with the table held).
    fn counted(&self, table: &BTreeMap<DocId, Arc<Document>>) -> DocId {
        let given = self.given.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            let id = DocId::nth(self.last_id.fetch_add(1, Ordering::Relaxed) + 1);
            if !given.contains(&id) && !table.contains_key(&id) {
                return id;
            }
        }
    }

    /// The kind of the open document `id`.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn kind_of(&self, id: &DocId) -> Result<DocKind, SessionError> {
        Ok(self.document(id)?.kind)
    }

    /// The format's state of the open document `id`.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn format_doc(&self, id: &DocId) -> Result<Arc<dyn FormatDoc>, SessionError> {
        Ok(Arc::clone(&self.document(id)?.doc))
    }

    /// The document `id` at its head version. Never waits for a writer.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn snapshot(&self, id: &DocId) -> Result<Opened, SessionError> {
        let document = self.document(id)?;
        Ok(Self::opened(id, &document, document.head()))
    }

    pub(crate) fn opened(id: &DocId, document: &Document, version: AnyVersion) -> Opened {
        Opened {
            id: id.clone(),
            kind: document.kind,
            name: Arc::from(document.name.as_str()),
            version,
            readings: document.doc.readings(),
        }
    }

    /// The document's bytes at its head version: untouched entries copied raw.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`], or the package's error writing a part.
    pub fn save(&self, id: &DocId) -> Result<Vec<u8>, SessionError> {
        let document = self.document(id)?;
        document.doc.save(&document.head(), false)
    }

    /// The save gate (section 13, P2): the document's bytes at its head version, refused
    /// when a Word document has validation problems it did not open with.
    ///
    /// # Errors
    /// [`SessionError::NewProblems`], with every new problem; as [`Session::save`].
    pub fn save_checked(&self, id: &DocId) -> Result<Vec<u8>, SessionError> {
        let document = self.document(id)?;
        document.doc.save(&document.head(), true)
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
    /// by `context.caller`: a version made outside the tools, for a test that needs a
    /// document's parts as it writes them (section 13). No host commits through it: every
    /// change a host makes is a tool's edit. XML parts are parsed here, on a snapshot,
    /// without any lock; the commit is the store's.
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
        let (version, changed, attempts, batch) =
            document.doc.update(&context.caller, None, &update).inspect_err(|e| {
                if matches!(e, SessionError::Base(ooxml_base::BaseError::Cancelled)) {
                    self.counts.cancelled.fetch_add(1, Ordering::Relaxed);
                }
            })?;
        if changed && !batch {
            document.stepped(version.seq(), version.number(), None, context);
        }
        Ok(Updated {
            version: version.seq(),
            changed,
            attempts,
        })
    }

    /// The store's counters for the document (commits, conflicts, exclusive runs).
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn store_stats(&self, id: &DocId) -> Result<StoreStats, SessionError> {
        Ok(self.document(id)?.stored().stats())
    }

    /// How long the document's commit lock was held and waited for, each time it was taken.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn lock_times(&self, id: &DocId) -> Result<ooxml_store::LockTimes, SessionError> {
        Ok(self.document(id)?.stored().lock_times())
    }

    /// The session's counters.
    #[must_use]
    pub fn stats(&self) -> SessionStats {
        SessionStats {
            calls: self.counts.calls.load(Ordering::Relaxed),
            failed: self.counts.failed.load(Ordering::Relaxed),
            cancelled: self.counts.cancelled.load(Ordering::Relaxed),
            open: self.read_table().len() as u64,
            panics: self
                .counts
                .panics
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            internal: self
                .counts
                .internal
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
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
    pub(crate) fn read_table(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<DocId, Arc<Document>>> {
        self.documents.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write_table(&self) -> std::sync::RwLockWriteGuard<'_, BTreeMap<DocId, Arc<Document>>> {
        self.documents.write().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A session's nonce: the moment `context`'s clock says it is made and this session's own
/// random keys ([`RandomState`], drawn afresh for each session), hashed, as sixteen hex
/// digits -- two sessions made at the same moment, or in two processes, tell their tokens
/// apart.
fn nonce(context: &Context) -> String {
    let now = context.clock.now();
    let mut hasher = RandomState::new().build_hasher();
    now.seconds.hash(&mut hasher);
    now.nanos.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
