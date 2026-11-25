//! Owns: what the session asks of a format and of a document whatever its format (design
//! phase 3, section 3): [`Format`], a kind of document the session holds -- its kind, its
//! tools, registered for that kind --; [`FormatDoc`], one open document as its format keeps it -- its versions and history
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

use crate::update::PartUpdate;
use crate::{DocKind, Registry, SessionError, StoredDoc};
use ooxml_base::pyfmt::repr_str;
use ooxml_opc::Package;
use ooxml_store::{Caller, DocModel, Snapshot};
use ooxml_tools::datasets::Dataset;
use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};
use std::any::Any;
use std::sync::Arc;

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
}

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

    /// The format's own type, for its code to read (a downcast).
    fn as_any(&self) -> &dyn Any;
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
