//! Owns: [`Session`], documents by id with their stores, and the operations on them: open,
//! save, close, the bridge's update, snapshots and counters. A tool call's dispatch is
//! [`crate::dispatch`]'s.
//!
//! Must not: hold a lock while a handler runs or a package is parsed or written (the table
//! lock is held only to find or change an entry), or implement a tool.

use crate::context::CallCancel;
use crate::document::{At, Document};
use crate::update::{HostIds, PartUpdate, Updated};
use crate::{CallContext, DocId, DocKind, Registry, SessionError};
use ooxml_base::Context;
use ooxml_opc::Package;
use ooxml_store::{StoreConfig, StoreStats};
use ooxml_tools_word::{Placer, Version};
use ooxml_word_edit::EarlierIds;
use ooxml_word_model::docx::Versions;
use ooxml_word_model::ids::{HeldId, LoadedState};
use std::collections::BTreeMap;
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
    version: At,
    versions: Arc<Versions>,
    /// Where the call reading it may ask where blocks are laid out (its context's).
    placer: Option<Arc<dyn Placer>>,
}

impl Opened {
    /// This version, read by a call that may ask `placer` where blocks are laid out.
    #[must_use]
    pub(crate) fn with_placer(self, placer: Option<Arc<dyn Placer>>) -> Self {
        Self { placer, ..self }
    }

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
pub(crate) struct Counts {
    pub(crate) calls: AtomicU64,
    pub(crate) failed: AtomicU64,
    pub(crate) cancelled: AtomicU64,
    pub(crate) updates: AtomicU64,
    /// `bridge_calls{tool}` (section 13): unported tools' calls committed through the bridge.
    bridged: std::sync::Mutex<BTreeMap<String, u64>>,
}

/// Documents by id, their versions, and the tools that read and change them.
pub struct Session {
    pub(crate) config: SessionConfig,
    pub(crate) context: Context,
    pub(crate) registry: Registry,
    documents: RwLock<BTreeMap<DocId, Arc<Document>>>,
    last_id: AtomicU64,
    pub(crate) counts: Counts,
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

    /// A Word document opened from its bytes as `name`, with the id state saved beside them
    /// (`docx-ids.1.`, docx-agent's `Document.id_state`): its ids are read against its parts
    /// as they are parsed, so it never gives an id it had before it was saved again, and each
    /// id the state retired is the session's record (who retired it, in the host's words).
    /// Returns its id and the state's `(version, last)`: the version saved and the highest
    /// its history gave, for the host's numbers to go on from.
    ///
    /// # Errors
    /// As [`Session::open`]; [`SessionError::Word`] with
    /// [`ooxml_word_edit::EditError::IdState`] when the state is not these bytes'.
    pub fn open_state(&self, bytes: Arc<[u8]>, name: &str, state: &str) -> Result<(DocId, (i64, i64)), SessionError> {
        let package = Package::open(bytes, &self.context.limits)?;
        let (document, loaded) = Document::open_word_state(package, name, self.config.store, &self.context, state)?;
        let id = DocId::nth(self.last_id.fetch_add(1, Ordering::Relaxed) + 1);
        self.write_table().insert(id.clone(), Arc::new(document));
        self.seed_retired(&id, retired_rows(&loaded))?;
        Ok((id, (loaded.versions.version, loaded.versions.last)))
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
            if !document.grouped() {
                document.stepped(version.number(), tool, context);
            }
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

/// What a loaded state retired, as the session's record takes it: each block id and drawing
/// number with the version that retired it (the host's number) and who made that version.
fn retired_rows(loaded: &LoadedState) -> Vec<(HeldId, u64, Option<String>)> {
    let said = |step: i64| loaded.steps.get(&step).cloned().flatten();
    let version = |step: i64| u64::try_from(step).unwrap_or_default();
    let blocks = loaded
        .gone
        .iter()
        .map(|(v, s)| (HeldId::Block(*v), version(*s), said(*s)));
    let drawings = loaded
        .drawings_gone
        .iter()
        .map(|(n, s)| (HeldId::Drawing(*n), version(*s), said(*s)));
    blocks.chain(drawings).collect()
}
