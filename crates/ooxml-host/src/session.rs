//! Owns: [`Host`], one per toolbox -- its formats, its catalogue of tools ([`Catalogue`]:
//! the formats' tools and the host's own, with their host facts, the formats' hints and
//! withdrawn names), its settings ([`HostSettings`]) and the seam its session tools run
//! through ([`SessionTools`]); and [`HostSession`], one per application session -- the
//! session's documents (`ooxml-session`'s [`Session`], which names them `d1`, `d2`...)
//! and its stores behind one lock ([`Stores`]: blobs, outputs, image budgets, datasets,
//! each document's refs, retry keys and the call log); opening a document from bytes, adding
//! a blob, closing; and the roles and context of a call ([`CallAs`], a view's).
//!
//! Must not: hold the stores' lock while a handler runs or a package is parsed or written;
//! run a call (the pipeline does: [`crate::pipeline`]).

use crate::catalogue::{Catalogue, Entry, Own};
use crate::own::{specs, SessionTools};
use crate::records::{CallRecord, RetryKeys};
use ooxml_base::Context;
use ooxml_session::blobs::{BlobLimits, BlobStore, LimitError};
use ooxml_session::datasets::{DatasetLimits, DatasetStore};
use ooxml_session::numbers::NumberRules;
use ooxml_session::outputs::{ImageBudgets, OutputStore};
use ooxml_session::{DocId, DocKind, Format, RefTable, Registry, Session, SessionConfig, SessionError};
use ooxml_tools::roles::{HostFacts, RoleSet};
use ooxml_tools::Limits;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// What a host's sessions run with, the application's settings.
#[derive(Debug, Clone)]
pub struct HostSettings {
    /// The roles a session holds unless the application narrows them.
    pub roles: RoleSet,
    /// The limits on a call's reply.
    pub limits: Limits,
    /// Whether a call's record keeps its arguments.
    pub log_arguments: bool,
    /// The limits on the inputs a session registers.
    pub blobs: BlobLimits,
    /// The limits on a session's datasets.
    pub datasets: DatasetLimits,
    /// The room for open documents.
    pub documents: crate::documents::DocumentLimits,
    /// Whether a save may add validation problems the document did not open with (the
    /// save gate let through).
    pub allow_new_problems: bool,
    /// The images a session may show, and a round: `None`, no budget.
    pub image_budget: Option<usize>,
    /// The images a round may show.
    pub image_budget_per_round: Option<usize>,
    /// The ops a `batch` may hold (`Limits.max_batch_ops`).
    pub max_batch_ops: usize,
    /// How long a `batch` may run its ops (`Limits.batch_timeout`): the ops it has not
    /// started by then are not run.
    pub batch_timeout: std::time::Duration,
    /// How long laying a document out may take in a call, from its start (ooxml-edit's
    /// `Limits.layout_timeout`); `None`: as long as the call.
    pub layout_timeout: Option<Duration>,
    /// A `read_batch`'s reply budget, characters (`Limits.max_read_batch_chars`).
    pub read_batch_chars: usize,
    /// The rules of a session's checks of numbers, unless the application sets its own
    /// ([`HostSession::set_number_rules`]).
    pub numbers: NumberRules,
    /// The application's own tools, which its facade runs outside the host (design phase 3,
    /// decision D1): an op of a `batch` or a `read_batch` naming one is refused with the way
    /// out "call it alone" ([`HostSettings::own_tool_refused`]).
    pub own_tools: Vec<String>,
}

impl Default for HostSettings {
    fn default() -> Self {
        HostSettings {
            roles: RoleSet::all(),
            limits: Limits::DEFAULT,
            log_arguments: false,
            blobs: BlobLimits::default(),
            datasets: DatasetLimits::default(),
            documents: crate::documents::DocumentLimits::default(),
            allow_new_problems: false,
            image_budget: Some(40),
            image_budget_per_round: None,
            max_batch_ops: 200,
            batch_timeout: std::time::Duration::from_secs(120),
            layout_timeout: Some(Duration::from_secs(30)),
            read_batch_chars: ooxml_session::MAX_READ_BATCH_CHARS,
            numbers: NumberRules::default(),
            own_tools: Vec::new(),
        }
    }
}

/// One host: its formats, its tools and its settings.
pub struct Host {
    pub(crate) formats: Vec<Arc<dyn Format>>,
    pub(crate) catalogue: Catalogue,
    pub(crate) settings: HostSettings,
    pub(crate) own: Arc<dyn SessionTools>,
    /// The owners its batches of several documents hold their transactions as, one per batch.
    owners: AtomicU64,
}

/// Where the owners a host mints begin: above those the Python host's batches hold their
/// transactions as (one per thread, counted from 1: `ooxml-session`'s step groups, until
/// Phase 3 E2 deletes them), so the two never meet in one store.
const OWNERS_FROM: u64 = 1 << 63;

impl Host {
    /// A host of `formats`' documents and tools, with `settings`, its session tools run by
    /// `own`.
    #[must_use]
    pub fn new(formats: Vec<Arc<dyn Format>>, settings: HostSettings, own: Arc<dyn SessionTools>) -> Host {
        let catalogue = catalogue_of(&formats);
        Host {
            formats,
            catalogue,
            settings,
            own,
            owners: AtomicU64::new(OWNERS_FROM),
        }
    }

    /// A batch's owner, never given before (design phase 3, section 4: minted per batch call).
    pub(crate) fn owner(&self) -> ooxml_store::TxnOwner {
        ooxml_store::TxnOwner::new(self.owners.fetch_add(1, Ordering::Relaxed))
    }

    /// Its tools.
    #[must_use]
    pub fn catalogue(&self) -> &Catalogue {
        &self.catalogue
    }

    /// Its settings.
    #[must_use]
    pub fn settings(&self) -> &HostSettings {
        &self.settings
    }

    /// A new session with `context`'s ports and `config`.
    #[must_use]
    pub fn session(self: &Arc<Self>, config: SessionConfig, context: Context) -> HostSession {
        let registry = Registry::of(&self.formats);
        let session = Session::with_registry(config, context, self.formats.clone(), registry);
        let settings = &self.settings;
        HostSession {
            host: Arc::clone(self),
            roles: Mutex::new(settings.roles),
            numbers: Mutex::new(settings.numbers.clone()),
            session,
            stores: Mutex::new(Stores {
                blobs: BlobStore::new(settings.blobs),
                outputs: OutputStore::new(None),
                images: ImageBudgets::new(settings.image_budget, settings.image_budget_per_round),
                datasets: DatasetStore::new(settings.datasets),
                refs: BTreeMap::new(),
                docs: BTreeMap::new(),
                keys: RetryKeys::default(),
                log: Vec::new(),
                numbers: None,
            }),
        }
    }
}

/// The catalogue of `formats`: the formats' tools in their order, with their host facts,
/// and the host's own around them in today's order (`new_document` first; the session
/// tools, `undo` and `read_batch` after).
fn catalogue_of(formats: &[Arc<dyn Format>]) -> Catalogue {
    let registry = Registry::of(formats);
    let facts: BTreeMap<String, HostFacts> = formats.iter().flat_map(|f| f.host_facts()).collect();
    let fact = |name: &str| facts.get(name).cloned().unwrap_or_default();
    let mut own = specs();
    let mut entries: Vec<Entry> = Vec::new();
    let mut push_own = |which: Own, entries: &mut Vec<Entry>| {
        if let Some(at) = own.iter().position(|(o, _)| *o == which) {
            let (_, spec) = own.remove(at);
            let name = spec.name.clone();
            entries.push(Entry::new(spec, fact(&name), Some(which)));
        }
    };
    push_own(Own::NewDocument, &mut entries);
    for name in registry.names() {
        if matches!(name.as_str(), "undo" | "read_batch") {
            continue;
        }
        let Some(tool) = registry.named(&name) else { continue };
        let mut entry = Entry::new(tool.spec().clone(), fact(&name), None);
        entry.kinds = formats
            .iter()
            .map(|f| f.kind())
            .filter(|kind| registry.get(&name, *kind).is_some())
            .collect();
        entries.push(entry);
    }
    for which in [Own::OpenDocument, Own::CloseDocument] {
        push_own(which, &mut entries);
    }
    entries.push(Entry::new(registry.undo_spec().clone(), fact("undo"), Some(Own::Undo)));
    for which in [Own::Batch, Own::ReadBlob] {
        push_own(which, &mut entries);
    }
    entries.push(Entry::new(
        registry.batch_spec().clone(),
        fact("read_batch"),
        Some(Own::ReadBatch),
    ));
    let withdrawn = formats.iter().flat_map(|f| f.withdrawn()).collect();
    let hints = formats.iter().flat_map(|f| f.tool_hints()).collect();
    Catalogue::new(entries, withdrawn, hints)
}

/// A session's stores, behind its one lock.
pub struct Stores {
    /// The inputs the application registered.
    pub blobs: BlobStore,
    /// The files tools produced for the application.
    pub outputs: OutputStore,
    /// The images the session and the round may still show.
    pub images: ImageBudgets,
    /// The datasets.
    pub datasets: DatasetStore,
    /// Each document's refs, by its id; kept after it closes.
    pub refs: BTreeMap<String, RefTable>,
    /// What the session keeps of each open document beside its versions, by its id.
    pub docs: BTreeMap<String, crate::documents::Held>,
    /// The retry keys.
    pub keys: RetryKeys,
    /// Every call's record, in order.
    pub log: Vec<CallRecord>,
    /// The index of values its checks of numbers compare with, kept while nothing it was
    /// built from changes.
    pub numbers: Option<crate::KeptIndex>,
}

/// The roles and context a call runs with: a view's, or a dispatch's own.
#[derive(Debug, Clone, Default)]
pub struct CallAs {
    /// The roles, where the call narrows its session's.
    pub roles: Option<RoleSet>,
    /// The caller's context, its keys in the order given.
    pub context: Option<Map<String, Value>>,
    /// What the application resolved for the call itself: the rows of sources in documents
    /// it holds (a workbook its own host holds), handed to the format beside the host's.
    pub values: ooxml_tools::written::CallValues,
    /// The moment the application dates the call's writes by (a revision's, a comment's, a
    /// new document's creation: `2026-11-20T09:30:15Z`), its own clock's as it read it for
    /// the call; `None`: the session's clock.
    pub date: Option<String>,
}

/// One application session of a host.
pub struct HostSession {
    pub(crate) host: Arc<Host>,
    roles: Mutex<RoleSet>,
    pub(crate) numbers: Mutex<NumberRules>,
    pub(crate) session: Session,
    stores: Mutex<Stores>,
}

impl HostSession {
    /// The host.
    #[must_use]
    pub fn host(&self) -> &Host {
        &self.host
    }

    /// The documents' session.
    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The roles its calls hold unless narrowed.
    #[must_use]
    pub fn roles(&self) -> RoleSet {
        *self.roles.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The application sets the roles its calls hold from now on (ooxml-edit's
    /// `session.roles = ...`).
    pub fn set_roles(&self, roles: RoleSet) {
        *self.roles.lock().unwrap_or_else(PoisonError::into_inner) = roles;
    }

    /// The stores, held for as long as the guard lives: never across a handler.
    pub fn stores(&self) -> MutexGuard<'_, Stores> {
        self.stores.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Open a package of `kind` from its bytes, as `name`: its id (`d1`), its refs empty
    /// ([`HostSession::open_with`] as the application opens one of a kind it names).
    ///
    /// # Errors
    /// As [`HostSession::open_with`].
    pub fn open(&self, bytes: Arc<[u8]>, kind: DocKind, name: &str) -> Result<DocId, crate::OpenRefused> {
        let how = crate::Opening {
            kind: Some(kind),
            ..crate::Opening::default()
        };
        self.open_with(bytes, name, &how)
    }

    /// Register an input and return its handle (`b1`).
    ///
    /// # Errors
    /// A limit broken, or bytes that are not what they claim.
    pub fn add_blob(&self, bytes: Arc<[u8]>, name: &str, mime: Option<&str>) -> Result<String, LimitError> {
        self.stores().blobs.add(bytes, name, mime)
    }

    /// Close `doc`: its datasets stay, frozen as stale or current as they are now (their
    /// addresses returned); its refs are kept, its retry keys forgotten.
    ///
    /// # Errors
    /// The session's ([`SessionError::NotOpen`]).
    pub fn close(&self, doc: &DocId) -> Result<Vec<String>, SessionError> {
        let head = self.session.snapshot(doc)?.number();
        self.session.close(doc)?;
        let mut stores = self.stores();
        stores.docs.remove(&doc.to_string());
        stores.keys.forget(&doc.to_string());
        Ok(stores.datasets.document_closed(&doc.to_string(), Some(head)))
    }

    /// Start a round: the round's image budget starts again.
    pub fn new_round(&self) {
        self.stores().images.new_round();
    }

    /// Every call's record so far.
    #[must_use]
    pub fn log(&self) -> Vec<CallRecord> {
        self.stores().log.clone()
    }

    /// The roles `asked` narrows the session's to.
    ///
    /// # Errors
    /// The application's mistake: roles the session does not hold.
    pub fn narrowed(&self, asked: Option<RoleSet>) -> Result<RoleSet, String> {
        match asked {
            None => Ok(self.roles()),
            Some(asked) if self.roles().covers(asked) => Ok(asked),
            Some(asked) => Err(format!(
                "roles {} are not all this session's ({})",
                asked.listed(),
                self.roles().listed()
            )),
        }
    }
}
