//! Owns: the formats a frozen answer's test made up beside Word's (`host/frozen/values.jsonl`,
//! `python/freeze_values.py`: a grid whose `t:` names its tables), made again from what the
//! call asked of them: [`FrozenFormat`], a kind with its address prefixes and each run of
//! addresses it merged; [`FrozenDoc`], one open document of it -- its versions a package's,
//! the rows its resolver gave each address the call named (or its refusal), its
//! `value_sources`.
//!
//! Must not: answer what the call did not ask of the format when it was frozen (an address
//! it never resolved is an `internal` failure: the replay is not that call's).

use ooxml_opc::Package;
use ooxml_session::{
    AnyVersion, DocKind, Finished, Format, FormatDoc, FormatRun, NewDocument, OpenEnv, PackageFormat, PartUpdate,
    Registry, SessionError, StoredDoc,
};
use ooxml_store::Caller;
use ooxml_tools::datasets::Dataset;
use ooxml_tools::{Refusal, Tool, ToolError, ToolResult, WayOut};
use serde_json::{Map, Value};
use std::any::Any;
use std::sync::Arc;

/// A made-up format of a frozen answer: its kind, the prefixes it resolves to rows, and
/// each run of addresses it merged (`[given, merged]`).
pub struct FrozenFormat {
    /// The kind it stands for (the test's own kind read as one of the session's).
    pub kind: DocKind,
    /// The prefixes it resolves to rows (`t:`).
    pub prefixes: Vec<String>,
    /// Each run of addresses merged, as it merged it.
    pub merges: Vec<(Vec<String>, Vec<String>)>,
}

impl Format for FrozenFormat {
    fn kind(&self) -> DocKind {
        self.kind
    }

    fn source_prefixes(&self) -> Vec<String> {
        self.prefixes.clone()
    }

    fn merge_sources(&self, sources: Vec<String>) -> Vec<String> {
        self.merges
            .iter()
            .find(|(given, _)| *given == sources)
            .map_or(sources, |(_, merged)| merged.clone())
    }

    fn register(&self, _tools: &mut Registry) {}

    fn open(&self, package: Package, env: &OpenEnv<'_>) -> Result<Arc<dyn FormatDoc>, SessionError> {
        PackageFormat(self.kind).open(package, env)
    }

    fn detect(&self, _bytes: &[u8], _name: &str) -> bool {
        false
    }

    fn new_document(&self, made: &NewDocument<'_>, env: &OpenEnv<'_>) -> Result<Vec<u8>, ToolError> {
        PackageFormat(self.kind).new_document(made, env)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// One open document of a [`FrozenFormat`]: a package's versions, and what its resolver gave
/// each address (`Ok`: the rows; `Err`: its refusal).
pub struct FrozenDoc {
    /// Its versions.
    pub inner: Arc<dyn FormatDoc>,
    /// Each address resolved and what the resolver answered.
    pub answers: Vec<(String, Result<Dataset, ToolError>)>,
    /// Its `value_sources`, where its format has them.
    pub value_sources: Option<Vec<String>>,
}

impl FrozenDoc {
    /// What a frozen answer (`{"rows": ...}` or `{"refused": {"code", "message",
    /// "options"}}`) says, the rows read by `rows`.
    #[must_use]
    pub fn answer(given: &Value, rows: impl Fn(&Value) -> Option<Dataset>) -> Result<Dataset, ToolError> {
        if let Some(found) = given.get("rows") {
            return rows(found).ok_or_else(|| ToolError::internal("frozen rows unreadable", "frozen"));
        }
        let refused = &given["refused"];
        let code = match refused["code"].as_str() {
            Some("not_found") => Refusal::NotFound,
            Some("ambiguous") => Refusal::Ambiguous,
            Some("refused") => Refusal::Refused,
            _ => Refusal::InvalidArguments,
        };
        let options: Vec<Value> = refused["options"].as_array().cloned().unwrap_or_default();
        let message = refused["message"].as_str().unwrap_or_default().to_owned();
        Err(ToolError::refusal(
            code,
            message,
            WayOut::options(options).unwrap_or_else(|| WayOut::one(Value::Null)),
        ))
    }
}

impl FormatDoc for FrozenDoc {
    fn stored(&self) -> &dyn StoredDoc {
        self.inner.stored()
    }

    fn source(&self, _at: &AnyVersion, address: &str) -> Option<Result<Dataset, ToolError>> {
        let found = self.answers.iter().find(|(a, _)| a == address).map(|(_, r)| r.clone());
        Some(found.unwrap_or_else(|| {
            Err(ToolError::internal(
                format!("{address} was not resolved when the answer was frozen"),
                "frozen",
            ))
        }))
    }

    fn value_sources(&self, _at: &AnyVersion) -> Option<Vec<String>> {
        self.value_sources.clone()
    }

    fn save(&self, at: &AnyVersion, checked: bool) -> Result<Vec<u8>, SessionError> {
        self.inner.save(at, checked)
    }

    fn update(
        &self,
        caller: &Caller,
        tool: Option<&str>,
        update: &PartUpdate<'_>,
    ) -> Result<(AnyVersion, bool, u32, bool), SessionError> {
        self.inner.update(caller, tool, update)
    }

    fn readings(&self) -> Arc<dyn Any + Send + Sync> {
        self.inner.readings()
    }

    fn run(&self, tool: &dyn Tool, run: &FormatRun<'_>) -> ToolResult<Value> {
        self.inner.run(tool, run)
    }

    fn finish(&self, made: Box<dyn Any + Send>, run: &FormatRun<'_>) -> Option<Finished> {
        self.inner.finish(made, run)
    }

    fn baseline(&self) -> Vec<ooxml_session::problems::Problem> {
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
