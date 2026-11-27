//! Owns: one open document as the session holds it ([`Document`]): its kind and name, and
//! its format's [`FormatDoc`] (a Word document's in `ooxml-session-word`); and the format of
//! the kinds Rust does not own yet ([`PackageFormat`]): their versions kept as packages
//! ([`PackageDoc`]) in a [`Stored`], with no tools.
//!
//! Must not: dispatch calls or hold the session's table; parse or write a package beyond
//! opening it; read the clock but through the store's port.

use crate::context::StoreClock;
use crate::format::{AnyVersion, Finished, Format, FormatDoc, FormatRun, NewDocument, OpenEnv};
use crate::model::PackageDoc;
use crate::owners::{Owners, StepOwner};
use crate::problems::Problem;
use crate::stored::{Stored, StoredDoc};
use crate::update::PartUpdate;
use crate::{DocKind, Registry, SessionError};
use ooxml_opc::Package;
use ooxml_store::{Caller, Store};
use ooxml_tools::{Tool, ToolError, ToolResult};
use serde_json::{Map, Value};
use std::any::Any;
use std::sync::Arc;

/// A kind Rust does not own yet (`pptx`, `xlsx`): its versions kept as packages, no tools.
#[derive(Debug, Clone, Copy)]
pub struct PackageFormat(pub DocKind);

impl Format for PackageFormat {
    fn kind(&self) -> DocKind {
        self.0
    }

    fn register(&self, _tools: &mut Registry) {}

    fn open(&self, package: Package, env: &OpenEnv<'_>) -> Result<Arc<dyn FormatDoc>, SessionError> {
        let clock = Arc::new(StoreClock(Arc::clone(&env.context.clock)));
        Ok(Arc::new(Stored::new(Store::new(
            PackageDoc::new(package),
            env.store,
            clock,
        ))))
    }

    /// A package with the main part of its kind (`ppt/presentation.xml`,
    /// `xl/workbook.xml`, `word/document.xml`).
    fn detect(&self, bytes: &[u8], _name: &str) -> bool {
        package_has_part(bytes, main_part(self.0))
    }

    fn new_document(&self, _made: &NewDocument<'_>, _env: &OpenEnv<'_>) -> Result<Vec<u8>, ToolError> {
        // (the application's own host makes them until the kind's format is Rust's)
        Err(ToolError::internal(
            format!("the Rust session makes no new {} documents", self.0.as_str()),
            "session:package",
        ))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The main part of a package of `kind`.
fn main_part(kind: DocKind) -> &'static str {
    match kind {
        DocKind::Docx => "word/document.xml",
        DocKind::Pptx => "ppt/presentation.xml",
        DocKind::Xlsx => "xl/workbook.xml",
    }
}

/// Whether `bytes` are a ZIP package holding the part `name` (a format's `detect` reads
/// only the central directory).
#[must_use]
pub fn package_has_part(bytes: &[u8], name: &str) -> bool {
    bytes.starts_with(b"PK")
        && ooxml_zip::Archive::read(Arc::from(bytes), &ooxml_base::ZipLimits::default())
            .is_ok_and(|archive| archive.entries().iter().any(|e| e.meta().name == name))
}

/// A package's versions (kinds Rust does not own yet).
impl FormatDoc for Stored<PackageDoc> {
    fn stored(&self) -> &dyn StoredDoc {
        self
    }

    fn save(&self, at: &AnyVersion, _checked: bool) -> Result<Vec<u8>, SessionError> {
        Ok(at.package().save()?)
    }

    fn update(
        &self,
        caller: &Caller,
        tool: Option<&str>,
        update: &PartUpdate<'_>,
    ) -> Result<(AnyVersion, bool, u32), SessionError> {
        let (version, changed, attempts) = Stored::update(self, caller, tool, update)?;
        Ok((Self::any(version), changed, attempts))
    }

    fn readings(&self) -> Arc<dyn Any + Send + Sync> {
        Arc::new(())
    }

    fn run(&self, _tool: &dyn Tool, run: &FormatRun<'_>) -> ToolResult<Value> {
        let message = format!("{} has no tools for {} documents", run.tool, run.doc.kind());
        ToolResult::failed(ToolError::internal(message, "session:package"))
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

/// One open document.
pub(crate) struct Document {
    pub(crate) kind: DocKind,
    pub(crate) name: String,
    /// Its versions and its format's state of it.
    pub(crate) doc: Arc<dyn FormatDoc>,
}

impl Document {
    /// Its versions and history.
    pub(crate) fn stored(&self) -> &dyn StoredDoc {
        self.doc.stored()
    }

    /// The head version.
    pub(crate) fn head(&self) -> AnyVersion {
        self.stored().head()
    }

    /// Who made each of its steps, and its callers' own undos.
    pub(crate) fn owners(&self) -> &Owners {
        self.stored().owners()
    }

    /// A step numbered `number`, ended by the version `seq`, was made by `tool` (`None`:
    /// none, a change from outside the tools) as `context`'s caller: kept as its owner, and
    /// its number told to the format.
    pub(crate) fn stepped(&self, seq: u64, number: u64, tool: Option<&str>, context: &crate::CallContext) {
        let owner = tool.map(|tool| StepOwner::of(tool, context));
        self.stored().stepped(seq, number, owner);
    }
}
