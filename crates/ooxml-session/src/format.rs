//! Owns: what the session asks of a format and of a document whatever its format (design
//! phase 3, section 3): [`Format`], a kind of document the session holds -- its kind, its
//! tools, registered for that kind, how it opens --; [`FormatDoc`], one open document as its format keeps it -- its versions and history, how a tool call runs on it ([`FormatRun`], [`Finished`])
//! ([`FormatDoc::stored`], a [`crate::StoredDoc`]), its save and save gate, a test's commit
//! of parts, the rows it gives another document's call as a source ([`FormatDoc::source`],
//! with the words a source refusal reads of it) -- and [`AnyVersion`], one version of it
//! erased, which only its format reads as its own model (a downcast, in safe code).
//!
//! The session holds each open document as a `dyn FormatDoc`; a format keeps its own state
//! of a document (a Word document's tracking mode, retired ids, pages) in the type that
//! implements it, beside a [`crate::Stored`] of its model.
//!
//! Must not: name a format's model or state (each format's own code reads its versions),
//! or dispatch a call.

use crate::context::CallCancel;
use crate::problems::{Problem, Validation};
use crate::update::PartUpdate;
use crate::{CallContext, DocKind, Opened, Registry, SessionError, StoredDoc};
use ooxml_base::pyfmt::repr_str;
use ooxml_base::Context;
use ooxml_numfmt::Locale;
use ooxml_opc::Package;
use ooxml_store::{Caller, DocModel, OpTxn, Snapshot, StoreConfig};
use ooxml_tools::datasets::Dataset;
use ooxml_tools::roles::HostFacts;
use ooxml_tools::{Refusal, Tool, ToolError, ToolResult, TryCall, WayOut};
use serde_json::{Map, Value};
use std::any::Any;
use std::sync::{Arc, Mutex};

/// A kind of document the session holds.
pub trait Format: Send + Sync + 'static {
    /// The kind.
    fn kind(&self) -> DocKind;

    /// The address prefixes its documents resolve to rows for another document's call
    /// (`t:`; design section 5, [`FormatDoc::source`]); none by default (docx takes none).
    fn source_prefixes(&self) -> Vec<String> {
        Vec::new()
    }

    /// A run of its documents' own addresses written compactly for a result (cells next to
    /// each other as one range: `merge_sources`); as they are by default.
    fn merge_sources(&self, sources: Vec<String>) -> Vec<String> {
        sources
    }

    /// Its tools, registered for its kind ([`Registry::registrar`]).
    fn register(&self, tools: &mut Registry);

    /// What a host holds of its tools beside their specs (the roles of their actions, the
    /// calls of a changing tool that only read), by tool name; none by default.
    fn host_facts(&self) -> Vec<(String, HostFacts)> {
        Vec::new()
    }

    /// The names a model writes that the host's reading of names does not settle, each with
    /// the tool (and action) it means (`("word_add_table", "word_insert_markdown")`); none by
    /// default.
    fn tool_hints(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    /// The tools withdrawn from it until it has them again, each with what to call instead
    /// (a call of one is refused with this way out, not as an unknown tool); none by default.
    fn withdrawn(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    /// `package` opened as a document of this kind.
    ///
    /// # Errors
    /// The format's, when the package is not a document it can read.
    fn open(&self, package: Package, env: &OpenEnv<'_>) -> Result<Arc<dyn FormatDoc>, SessionError>;

    /// `package` opened as a document of this kind with the state saved beside its bytes
    /// ([`FormatDoc::state`]): it goes on from where it was (a Word document's retired ids
    /// stay retired), its history's numbers with it. By default the format keeps no state
    /// and refuses one ([`SessionError::StateRefused`] without a reason).
    ///
    /// # Errors
    /// [`SessionError::StateRefused`]: no state kept, or one that is not these bytes'; as
    /// [`Format::open`].
    fn open_with_state(&self, package: Package, state: &str, env: &OpenEnv<'_>) -> Result<WithState, SessionError> {
        let _ = (package, state, env);
        Err(SessionError::StateRefused {
            kind: self.kind(),
            reason: None,
        })
    }

    /// Whether `bytes` named `name` are a document of this kind (an `open_document` of a
    /// blob finds its format so).
    fn detect(&self, bytes: &[u8], name: &str) -> bool;

    /// Whether its documents give a check of numbers their texts
    /// ([`FormatDoc::number_texts`]); a refusal of one that gives none names those that do.
    fn gives_number_texts(&self) -> bool {
        false
    }

    /// Whether its documents read a scale's values for a check ([`FormatDoc::scale_values`]);
    /// a refusal of one that reads none names those that do.
    fn gives_scale_values(&self) -> bool {
        false
    }

    /// A new document's bytes, as `new_document` makes one of this kind: blank, or from the
    /// template `made` gives, with its page size, properties and the call's date; the host
    /// opens them.
    ///
    /// # Errors
    /// The format's refusal of the call, at its argument (`size`, `template_blob`), or that
    /// it makes no new documents.
    fn new_document(&self, made: &NewDocument<'_>, env: &OpenEnv<'_>) -> Result<Vec<u8>, ToolError>;

    /// The format's own type, for its code to read (a downcast).
    fn as_any(&self) -> &dyn Any;
}

/// A document opened with its saved state: the document, and the numbers its history goes
/// on from (the version it was saved at and the highest its history gave).
pub struct WithState {
    /// The document.
    pub doc: Arc<dyn FormatDoc>,
    /// `(version, highest)`, where the state numbers them.
    pub numbers: Option<(u64, u64)>,
}

/// What `new_document` asks of a format.
#[derive(Debug, Clone, Copy)]
pub struct NewDocument<'a> {
    /// The page or slide size named (`A4`, `16:9`); `None`: the format's own.
    pub size: Option<&'a str>,
    /// The title property.
    pub title: Option<&'a str>,
    /// The author property.
    pub author: Option<&'a str>,
    /// The template's bytes, where it is made from one (its layouts, styles and theme, not
    /// its content).
    pub template: Option<&'a [u8]>,
    /// When it is made (W3CDTF, UTC to the second: `2026-11-21T09:00:00Z`).
    pub created: &'a str,
    /// The session's locale (its text's language and separators); `None`: the format's
    /// default.
    pub locale: Option<&'a Locale>,
    /// The call as given, for a refusal's way out.
    pub retry: &'a TryCall,
}

/// What a document opens with: its store's configuration and the session's ports.
pub struct OpenEnv<'a> {
    /// Its store's configuration.
    pub store: StoreConfig,
    /// The session's ports (clock, cancellation, cache budget, limits).
    pub context: &'a Context,
}

/// One tool call on a document, as the session hands it to the document's format to run:
/// the tool and its arguments taken as meant, the version the call reads, what the host
/// gave the call. Every field is set by the session.
pub struct FormatRun<'a> {
    /// The tool's name.
    pub tool: &'a str,
    /// Its arguments, taken as meant and validated.
    pub arguments: &'a Map<String, Value>,
    /// The document at the version the call reads.
    pub doc: &'a Opened,
    /// The name the call gives its document, which its reply names it by.
    pub named: &'a str,
    /// The other documents the call names, by the names it gives them, at their pinned
    /// versions.
    pub others: &'a [(String, Opened)],
    /// What the host gave the call.
    pub context: &'a CallContext,
    /// The call's cancellation (its deadline, its limit on laying out).
    pub cancel: &'a CallCancel<'a>,
    /// The date the call's writes carry (the host's, else the session's clock).
    pub date: &'a str,
    /// The locale the call reads under: the call's, else the session's; `None`: each
    /// document's own language.
    pub locale: Option<&'a Locale>,
    /// What the call's edits made, as the format keeps it, set as soon as they made it (so
    /// it is the call's even when the handler panics after): [`FormatDoc::finish`] reads it.
    pub made: &'a Mutex<Option<Box<dyn Any + Send>>>,
    /// The transaction the call's edits run in, handed to it: an op of a host's batch of one
    /// document ([`crate::BatchOps`]), whose effects wait for the batch's publish
    /// ([`FormatRun::deferred`]); `None`: the call commits its own.
    pub txn: Option<&'a dyn OpTxn>,
    /// What the call keeps once the batch whose transaction it ran in is published (only such
    /// a call's: [`FormatRun::txn`]), in order.
    pub deferred: &'a Mutex<Vec<Deferred>>,
}

/// What a call run in a batch's transaction leaves its format to keep once the batch is
/// published (a Word call's retired ids, the rows it wrote), and to say then (a Word table
/// kept together that the document after the batch draws taller than a page): run with the
/// document, the version published and the batch's call, in op order, by the host -- once,
/// however often the batch ran, before the batch's checks. What it returns is the op's to
/// say (ooxml-edit's `Call.defer`: its warnings, in its place among the batch's).
pub type Deferred = Box<dyn FnOnce(&dyn FormatDoc, &AnyVersion, &CallContext, &CallCancel<'_>) -> Vec<String> + Send>;

/// The versions one changing call made, as its format finished them.
pub struct Finished {
    /// The number of the version its first edit made.
    pub first: u64,
    /// The version its first edit was applied to, held: the state its step goes from (kept
    /// for the call, however many steps others make before it is read).
    pub before: AnyVersion,
    /// The version its last edit made.
    pub last: AnyVersion,
    /// Whether it ran in a step group (whose close makes the step).
    pub batch: bool,
    /// What the format says of the call beside (a Word call's touched, validation, reflow).
    pub format: Option<Arc<dyn Any + Send + Sync>>,
    /// Where its commits' time went: how long they held the document's commit lock, and how
    /// long they waited for it and for the document's gate (R142's per-call timing).
    pub lock: LockSpent,
}

/// How long a call's commits held a document's commit lock, and waited for it and its gate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LockSpent {
    /// Held.
    pub held: std::time::Duration,
    /// Waited for.
    pub waited: std::time::Duration,
}

/// What a format holds for a host's batch ([`FormatDoc::hold_for_batch`]) until it is let
/// go: a lock's guard, of whatever kind.
pub trait Holding {}

impl<T> Holding for T {}

/// [`FormatDoc::hold_for_batch`]'s hold, let go when dropped.
pub type BatchHold<'a> = Box<dyn Holding + 'a>;

/// One open document as its format keeps it.
pub trait FormatDoc: Send + Sync + 'static {
    /// Its versions and history: the head, numbered states, undo and redo, who made each
    /// step, step groups.
    fn stored(&self) -> &dyn StoredDoc;

    /// The rows `address` names at the version `at`, read for another document's call (a
    /// source, design section 5): `None` where its format takes no such address (by
    /// default: docx takes none; [`no_source`] words the refusal).
    ///
    /// # Errors
    /// The format's refusal of an address of its prefix that names no rows (`not_found`
    /// with the names there are).
    fn source(&self, at: &AnyVersion, address: &str) -> Option<Result<Dataset, ToolError>> {
        let _ = (at, address);
        None
    }

    /// The addresses of the rows it holds at `at`, for a refusal to offer
    /// (`value_sources`); `None` by default.
    fn value_sources(&self, at: &AnyVersion) -> Option<Vec<String>> {
        let _ = at;
        None
    }

    /// The texts a check of numbers reads in it at `at`, within the check's scope `read`
    /// gives (`number_texts`; design phase 3, section 3: what a document's check reads);
    /// `None` where its format gives none (by default).
    ///
    /// # Errors
    /// The format's refusal of the scope (a range it lacks, pages it cannot lay out).
    fn number_texts(&self, at: &AnyVersion, read: &ValueRead<'_>) -> Option<Result<ReadTexts, ToolError>> {
        let _ = (at, read);
        None
    }

    /// The values `scale`'s targets hold in it at `at`, each with the step of the scale it is
    /// styled with now, within the check's scope `read` gives (`scale_values`); `None` where
    /// its format reads none (by default).
    ///
    /// # Errors
    /// The format's refusal of the scope.
    fn scale_values(
        &self,
        at: &AnyVersion,
        scale: &crate::scales::Scale,
        read: &ValueRead<'_>,
    ) -> Option<Result<ReadValues, ToolError>> {
        let _ = (at, scale, read);
        None
    }

    /// What `address` names in it at `at` that gives no rows (`"a table"`, for a refusal's
    /// words: `own_source`); `None` by default.
    fn own_source(&self, at: &AnyVersion, address: &str) -> Option<String> {
        let _ = (at, address);
        None
    }

    /// What it says of a source value that names nothing, as the call's document
    /// (`source_hint`: a drawing's id given for a dataset); `None` by default.
    fn source_hint(&self, at: &AnyVersion, value: &str) -> Option<String> {
        let _ = (at, value);
        None
    }

    /// Its bytes at the version `at`, untouched entries copied raw; `checked`: the save gate
    /// first, the save refused where the format finds problems the document did not open
    /// with ([`SessionError::NewProblems`]).
    ///
    /// # Errors
    /// The save gate's refusal; the package's error writing a part.
    fn save(&self, at: &AnyVersion, checked: bool) -> Result<Vec<u8>, SessionError>;

    /// Commit a test's update of parts as `caller`, made by `tool` (section 13): the version it
    /// leaves, whether it changed anything, how many times it was prepared, and whether it
    /// ran in this thread's step group (whose close makes the step).
    ///
    /// # Errors
    /// The update's or the store's.
    fn update(
        &self,
        caller: &Caller,
        tool: Option<&str>,
        update: &PartUpdate<'_>,
    ) -> Result<(AnyVersion, bool, u32, bool), SessionError>;

    /// What the format keeps for reading its versions (a Word document's readings), handed
    /// with every version read ([`Opened::readings`]).
    fn readings(&self) -> Arc<dyn Any + Send + Sync>;

    /// Run `tool` as `run` says on this document. Never fails: a refusal is the result's
    /// error.
    fn run(&self, tool: &dyn Tool, run: &FormatRun<'_>) -> ToolResult<Value>;

    /// What a changing call made (its `run.made`, taken after it ran or panicked): its
    /// versions, noted as the format keeps them; `None` where it made nothing to finish.
    fn finish(&self, made: Box<dyn Any + Send>, run: &FormatRun<'_>) -> Option<Finished>;

    /// The checks a changing call's reply carries (`checks`: the validation delta, the
    /// pages it reflowed) of the version `at` it left, `made` what the format said of the
    /// call ([`Finished::format`]); `None` where the format checks nothing (by default).
    fn checks(&self, at: &AnyVersion, made: Option<&(dyn Any + Send + Sync)>) -> Option<Map<String, Value>> {
        let _ = (at, made);
        None
    }

    /// The checks a host's batch's reply carries for this document, which the batch took from
    /// `before` to `after` (`None`: it changed nothing here): [`FormatDoc::checks`] of `after`,
    /// with what the batch reflowed, measured once after its last op where the format lays
    /// its documents out (`context`'s layout, within its limit on laying out); by default
    /// the checks alone.
    fn batch_checks(
        &self,
        before: Option<&AnyVersion>,
        after: &AnyVersion,
        context: &CallContext,
        cancel: &CallCancel<'_>,
    ) -> Option<Map<String, Value>> {
        let _ = (before, context, cancel);
        self.checks(after, None)
    }

    /// What a call that went from `before` to `after` touched, one entry per thing in the
    /// format's own addresses (`made`: what the format said of the call); by default each
    /// part whose bytes changed, `{"part": name}`, in name order.
    fn touched(&self, before: &AnyVersion, after: &AnyVersion, made: Option<&(dyn Any + Send + Sync)>) -> Vec<Value> {
        let _ = made;
        let mut parts = crate::changes::changed(before.package(), after.package());
        parts.sort();
        parts
            .into_iter()
            .map(|part| {
                let mut entry = Map::new();
                entry.insert("part".into(), Value::from(part));
                Value::Object(entry)
            })
            .collect()
    }

    /// The problems of the version `at` against those the document opened with, in the
    /// session's one shape ([`Validation`]); `None` where the format validates nothing (by
    /// default).
    fn validation(&self, at: &AnyVersion) -> Option<Validation> {
        let _ = at;
        None
    }

    /// The problems the document opened with (the save gate's baseline; `describe` lists
    /// them): empty where it opened with none, or its format validates nothing.
    fn baseline(&self) -> Vec<Problem>;

    /// What `open_document` and `new_document` say of the version `at`: cheap facts, no
    /// layout (a Word document's headings, paragraphs, tables...); `None` where its format
    /// says nothing of a document.
    fn summary(&self, at: &AnyVersion) -> Option<Map<String, Value>>;

    /// The state to keep beside the version `at`'s bytes, for [`Format::open_with_state`] to
    /// go on from (a Word document's ids); `None` where its format keeps none.
    fn state(&self, at: &AnyVersion) -> Option<String>;

    /// Hold, for the whole of a host's batch on this document -- every attempt, a run behind
    /// the gate, the commit and the keeping of its effects -- what its ops' edits read beside
    /// the document's versions, so no other call changes it between two of them (design §4:
    /// Word's tracking mode, held shared, or alone where `tools`, the batch's changing ops,
    /// include one that sets it). An op run in the batch's transaction then reads it without
    /// holding it again. `None`: nothing to hold (by default).
    fn hold_for_batch(&self, tools: &[&str]) -> Option<BatchHold<'_>> {
        let _ = tools;
        None
    }

    /// What a step group's revert brings back of the format's own state (a Word document's
    /// tracking mode), taken as the group is marked; `None` by default.
    fn group_state(&self) -> Option<Arc<dyn Any + Send + Sync>> {
        None
    }

    /// Bring back `state` ([`FormatDoc::group_state`]) as a step group is reverted or let go.
    fn restore_group(&self, state: &(dyn Any + Send + Sync)) {
        let _ = state;
    }

    /// The format's own type, for its code to read (a downcast).
    fn as_any(&self) -> &dyn Any;
}

/// What a check of values reads of one document: the document as the call names it, the
/// check's arguments that narrow it as given (`slides`, `pages`, `range`; a range's refs
/// resolved), what the host gave the call (a layout) and the call's cancellation.
pub struct ValueRead<'a> {
    /// The document as the call names it.
    pub named: &'a str,
    /// The check's arguments that narrow it.
    pub scope: &'a Map<String, Value>,
    /// What the host gave the call.
    pub context: &'a CallContext,
    /// The call's cancellation.
    pub cancel: &'a CallCancel<'a>,
}

/// The texts a document gives a check of numbers, and what the reader is warned of (a
/// layout that failed, `slides` read as pages).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadTexts {
    /// The texts, in document order.
    pub texts: Vec<crate::numbers::NumberText>,
    /// The warnings.
    pub warnings: Vec<String>,
}

/// The values a document reads at a scale's targets, and what the reader is warned of.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadValues {
    /// The values, in the targets' order.
    pub values: Vec<crate::scales::ScaleValue>,
    /// The warnings.
    pub warnings: Vec<String>,
}

/// The refusal of an address a document's format takes no rows from (`sources.resolve`).
#[must_use]
pub fn no_source(kind: DocKind, address: &str, retry: &TryCall) -> ToolError {
    ToolError::refusal(
        Refusal::InvalidArguments,
        format!(
            "{} documents take no such address: {}",
            kind.as_str(),
            repr_str(address)
        ),
        WayOut::call(retry.clone()),
    )
}

/// A version erased: its numbers and package, for any code; the snapshot itself for its
/// format's ([`AnyVersion::snapshot`]).
trait Erased: Send + Sync {
    fn package(&self) -> &Package;
    fn snapshot(&self) -> &dyn Any;
}

/// A snapshot of a model `D`, with how its package is read.
struct Of<D: DocModel> {
    snapshot: Snapshot<D>,
    package: fn(&D) -> &Package,
}

/// A version of a document held elsewhere ([`crate::elsewhere`]): its package says nothing.
struct Elsewhere(Package);

impl Erased for Elsewhere {
    fn package(&self) -> &Package {
        &self.0
    }

    fn snapshot(&self) -> &dyn Any {
        &()
    }
}

impl<D: DocModel> Erased for Of<D> {
    fn package(&self) -> &Package {
        (self.package)(self.snapshot.doc())
    }

    fn snapshot(&self) -> &dyn Any {
        &self.snapshot
    }
}

/// One version of a document whatever its format: it stays readable, unchanged, while
/// others commit, and after the document is closed.
#[derive(Clone)]
pub struct AnyVersion {
    seq: u64,
    number: u64,
    held: Arc<dyn Erased>,
}

impl AnyVersion {
    /// `snapshot`, its package read by `package`.
    #[must_use]
    pub fn of<D: DocModel>(snapshot: Snapshot<D>, package: fn(&D) -> &Package) -> Self {
        Self {
            seq: snapshot.seq().0,
            number: snapshot.number(),
            held: Arc::new(Of { snapshot, package }),
        }
    }

    /// A version of a document the application's own host holds ([`crate::elsewhere`]):
    /// numbered `seq` and `number` as that host says, `package` an empty one.
    #[must_use]
    pub fn elsewhere(seq: u64, number: u64, package: Package) -> Self {
        Self {
            seq,
            number,
            held: Arc::new(Elsewhere(package)),
        }
    }

    /// The version's number (the store's sequence number).
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// The state's number, as the tool layer numbers the document's states
    /// ([`ooxml_store::Version::number`]).
    #[must_use]
    pub fn number(&self) -> u64 {
        self.number
    }

    /// The package at this version.
    #[must_use]
    pub fn package(&self) -> &Package {
        self.held.package()
    }

    /// The snapshot, where it is one of the model `D` (its format's code reads it so).
    #[must_use]
    pub fn snapshot<D: DocModel>(&self) -> Option<&Snapshot<D>> {
        self.held.snapshot().downcast_ref()
    }
}
