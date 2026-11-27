//! Owns: documents the application's own host holds (design phase 3, decision D3: the
//! Python host of the formats the Rust host does not hold yet, until Phase 4), as the one
//! session names and counts them: [`ElsewhereFormat`], a kind with the address prefixes its
//! documents resolve to rows; [`HeldElsewhere`], the stand-in of one such document -- its
//! kind, size and state number, as the application's host says them -- in the session's one
//! table of documents, so ids come from one counter, the room is counted once, and a
//! source's address qualifies with it; and [`answering`], the stand-in as one call reads it,
//! answering with the values the application resolved for that call
//! ([`ooxml_tools::written::HeldValues`]), noting what it was not handed.
//!
//! Every operation of a format on a stand-in -- a tool, a save, a change, an undo, a batch
//! -- is refused with an internal error naming it: the document is not here.
//!
//! Must not: hold a document's content, or answer a value the application did not resolve.

use crate::batch_run::{BatchCommitted, BatchOps, BatchRefused, OpRan};
use crate::format::{AnyVersion, Finished, Format, FormatDoc, FormatRun, NewDocument, OpenEnv};
use crate::history::{Direction, Numbers};
use crate::owners::{Owners, StepOwner};
use crate::problems::Problem;
use crate::update::PartUpdate;
use crate::{DocKind, Registry, SessionError};
use ooxml_opc::Package;
use ooxml_store::{AttemptState, Caller, LockTimes, OpTxn, StoreStats, TxnMark, TxnOwner};
use ooxml_tools::datasets::Dataset;
use ooxml_tools::written::{CallValues, HeldValues, Missed};
use ooxml_tools::{Tool, ToolError, ToolResult};
use serde_json::{Map, Value};
use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Where an internal error about a stand-in says it came from.
const WHERE: &str = "session:held-elsewhere";

/// A format whose documents the application's own host holds: its kind and the address
/// prefixes they resolve to rows (`t:`). It opens nothing, detects nothing and makes nothing.
#[derive(Debug, Clone)]
pub struct ElsewhereFormat {
    /// The kind.
    pub kind: DocKind,
    /// The prefixes its documents resolve to rows.
    pub prefixes: Vec<String>,
}

impl ElsewhereFormat {
    fn refused(&self, what: &str) -> String {
        format!(
            "{} documents are held by the application's own host: the Rust session cannot {what}",
            self.kind
        )
    }
}

impl Format for ElsewhereFormat {
    fn kind(&self) -> DocKind {
        self.kind
    }

    fn source_prefixes(&self) -> Vec<String> {
        self.prefixes.clone()
    }

    fn register(&self, _tools: &mut Registry) {}

    fn open(&self, _package: Package, _env: &OpenEnv<'_>) -> Result<Arc<dyn FormatDoc>, SessionError> {
        Err(SessionError::Format(self.refused("open one").into()))
    }

    fn detect(&self, _bytes: &[u8], _name: &str) -> bool {
        false
    }

    fn new_document(&self, _made: &NewDocument<'_>, _env: &OpenEnv<'_>) -> Result<Vec<u8>, ToolError> {
        Err(ToolError::internal(self.refused("make one"), WHERE))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The stand-in of a document the application's own host holds: its kind, its size as
/// opened, and its state number now (the application's host's version, told as it moves:
/// [`HeldElsewhere::moved`]).
pub struct HeldElsewhere {
    kind: DocKind,
    size: u64,
    number: AtomicU64,
    seq: AtomicU64,
    owners: Owners,
    package: Package,
}

impl HeldElsewhere {
    /// The stand-in of a `kind` document of `size` bytes, at state `number`.
    ///
    /// # Errors
    /// The package its versions say they are could not be made (never, in practice).
    pub fn new(kind: DocKind, size: u64, number: u64) -> Result<HeldElsewhere, SessionError> {
        Ok(HeldElsewhere {
            kind,
            size,
            number: AtomicU64::new(number),
            seq: AtomicU64::new(1),
            owners: Owners::default(),
            package: empty_package()?,
        })
    }

    /// Its kind.
    #[must_use]
    pub fn kind(&self) -> DocKind {
        self.kind
    }

    /// Its size as opened, bytes.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The application's host's document is at state `number` now.
    pub fn moved(&self, number: u64) {
        if self.number.swap(number, Ordering::AcqRel) != number {
            self.seq.fetch_add(1, Ordering::AcqRel);
        }
    }

    fn refused(&self, what: &str) -> String {
        format!(
            "a {} document the application's own host holds (a stand-in here): the Rust session cannot {what}",
            self.kind
        )
    }

    fn session_error(&self, what: &str) -> SessionError {
        SessionError::Format(self.refused(what).into())
    }
}

/// An empty package: what a stand-in's versions say their package is.
fn empty_package() -> Result<Package, SessionError> {
    let mut writer = ooxml_zip::Writer::new();
    let entry = ooxml_zip::NewEntry {
        name: "[Content_Types].xml".into(),
        method: ooxml_zip::Method::Stored,
        dos_time: 0,
        dos_date: 33,
        external_attr: 0,
        create_system: 3,
    };
    let made = writer
        .add(&entry, b"<Types/>")
        .and_then(|()| writer.finish(b""))
        .map_err(|e| SessionError::Format(format!("an empty package: {e}").into()))?;
    Ok(Package::open(Arc::from(made), &ooxml_base::Limits::default())?)
}

impl crate::StoredDoc for HeldElsewhere {
    fn head(&self) -> AnyVersion {
        AnyVersion::elsewhere(
            self.seq.load(Ordering::Acquire),
            self.number.load(Ordering::Acquire),
            self.package.clone(),
        )
    }

    fn numbered(&self, number: u64) -> Option<AnyVersion> {
        (number == self.number.load(Ordering::Acquire)).then(|| self.head())
    }

    fn numbers(&self) -> Numbers {
        Numbers {
            highest: self.number.load(Ordering::Acquire),
            steps: Vec::new(),
            redo: Vec::new(),
        }
    }

    fn highest(&self) -> u64 {
        self.number.load(Ordering::Acquire)
    }

    fn continue_numbers(&self, _state: u64, _highest: u64) -> Result<(), SessionError> {
        Err(self.session_error("number its states"))
    }

    fn owners(&self) -> &Owners {
        &self.owners
    }

    fn stepped(&self, _seq: u64, _number: u64, _owner: Option<StepOwner>) {}

    fn history(&self, _steps: usize, _caller: Option<&str>, _direction: Direction) -> Result<usize, SessionError> {
        Err(self.session_error("undo or redo it"))
    }

    fn open_group(&self, _owner: TxnOwner) -> Result<TxnMark, SessionError> {
        Err(self.session_error("hold it for a batch"))
    }

    fn group_mark(&self, _owner: TxnOwner) -> Result<TxnMark, SessionError> {
        Err(self.session_error("hold it for a batch"))
    }

    fn revert_group(
        &self,
        _owner: TxnOwner,
        _mark: TxnMark,
        _caller: Option<&str>,
    ) -> Result<Option<u64>, SessionError> {
        Err(self.session_error("hold it for a batch"))
    }

    fn commit_group(&self, _owner: TxnOwner, _caller: &Caller) -> Result<Option<(u64, u64)>, SessionError> {
        Err(self.session_error("hold it for a batch"))
    }

    fn drop_group(&self, _owner: TxnOwner) -> bool {
        false
    }

    fn version_in(&self, _txn: &dyn OpTxn) -> Option<AnyVersion> {
        None
    }

    fn run_batch(&self, _caller: &Caller, _ops: &dyn BatchOps) -> Result<BatchCommitted, BatchRefused> {
        Err(BatchRefused::Store(self.session_error("run a batch on it")))
    }

    fn in_group(
        &self,
        _owner: TxnOwner,
        _attempt: &AttemptState,
        _run: &mut dyn FnMut(&dyn OpTxn) -> OpRan,
    ) -> Result<OpRan, SessionError> {
        Err(self.session_error("run a batch on it"))
    }

    fn stats(&self) -> StoreStats {
        StoreStats::default()
    }

    fn lock_times(&self) -> LockTimes {
        LockTimes::default()
    }
}

impl FormatDoc for HeldElsewhere {
    fn stored(&self) -> &dyn crate::StoredDoc {
        self
    }

    fn source(&self, _at: &AnyVersion, address: &str) -> Option<Result<Dataset, ToolError>> {
        Some(Err(ToolError::internal(
            self.refused(&format!(
                "read {address}: the application resolved no rows of it for this call"
            )),
            WHERE,
        )))
    }

    fn save(&self, _at: &AnyVersion, _checked: bool) -> Result<Vec<u8>, SessionError> {
        Err(self.session_error("save it"))
    }

    fn update(
        &self,
        _caller: &Caller,
        _tool: Option<&str>,
        _update: &PartUpdate<'_>,
    ) -> Result<(AnyVersion, bool, u32), SessionError> {
        Err(self.session_error("change it"))
    }

    fn run(&self, _tool: &dyn Tool, run: &FormatRun<'_>) -> ToolResult<Value> {
        ToolResult::failed(ToolError::internal(
            self.refused(&format!("run {} on it", run.tool)),
            WHERE,
        ))
    }

    fn finish(&self, _made: Box<dyn Any + Send>, _run: &FormatRun<'_>) -> Option<Finished> {
        None
    }

    fn baseline(&self) -> Vec<Problem> {
        Vec::new()
    }

    fn summary(&self, _at: &AnyVersion) -> Option<Map<String, Value>> {
        None
    }

    fn state(&self, _at: &AnyVersion) -> Option<String> {
        None
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `doc`, the open document `handle`, as one call reads it: a stand-in answering with what
/// the application resolved of it for the call (`values`), what it was not handed noted
/// there; any other document as it is.
#[must_use]
pub fn answering(doc: Arc<dyn FormatDoc>, handle: &str, values: &CallValues) -> Arc<dyn FormatDoc> {
    if !doc.as_any().is::<HeldElsewhere>() {
        return doc;
    }
    let held = values
        .held
        .iter()
        .find(|v| v.doc == handle)
        .cloned()
        .unwrap_or_default();
    Arc::new(Answering {
        held: doc,
        handle: handle.to_owned(),
        values: held,
        missed: values.missed.clone(),
    })
}

/// A stand-in as one call reads it: its rows and the tables it lists, as the application
/// resolved them for the call; everything else the stand-in's (refused).
pub struct Answering {
    held: Arc<dyn FormatDoc>,
    handle: String,
    values: HeldValues,
    missed: Missed,
}

impl FormatDoc for Answering {
    fn stored(&self) -> &dyn crate::StoredDoc {
        self.held.stored()
    }

    fn source(&self, at: &AnyVersion, address: &str) -> Option<Result<Dataset, ToolError>> {
        match self.values.answers.iter().find(|(given, _)| given == address) {
            Some((_, answer)) => Some(answer.clone()),
            None => {
                self.missed.note(&self.handle, address);
                self.held.source(at, address)
            }
        }
    }

    fn value_sources(&self, _at: &AnyVersion) -> Option<Vec<String>> {
        self.values.value_sources.clone()
    }

    fn save(&self, at: &AnyVersion, checked: bool) -> Result<Vec<u8>, SessionError> {
        self.held.save(at, checked)
    }

    fn update(
        &self,
        caller: &Caller,
        tool: Option<&str>,
        update: &PartUpdate<'_>,
    ) -> Result<(AnyVersion, bool, u32), SessionError> {
        self.held.update(caller, tool, update)
    }

    fn run(&self, tool: &dyn Tool, run: &FormatRun<'_>) -> ToolResult<Value> {
        self.held.run(tool, run)
    }

    fn finish(&self, made: Box<dyn Any + Send>, run: &FormatRun<'_>) -> Option<Finished> {
        self.held.finish(made, run)
    }

    fn baseline(&self) -> Vec<Problem> {
        Vec::new()
    }

    fn summary(&self, at: &AnyVersion) -> Option<Map<String, Value>> {
        self.held.summary(at)
    }

    fn state(&self, at: &AnyVersion) -> Option<String> {
        self.held.state(at)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests;
