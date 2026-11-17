//! Owns: the tool [`Registry`]: each tool's [`ToolSpec`] made once when it is registered,
//! the document kinds each serves, lookup by name, and the specs' export per kind for the
//! Python adapters; [`Tool`], a tool as the session dispatches it, and [`ReadHandler`], a
//! reading tool made from a `ToolArgs` struct and a handler function of a version.
//!
//! Must not: run a handler outside a call the session dispatches, know providers, or know
//! any tool's logic (the `ooxml-tools-*` crates hold the handlers).

use crate::session::Opened;
use crate::DocKind;
use ooxml_base::Cancel;
use ooxml_opc::Package;
use ooxml_tools::{Done, ToolArgs, ToolError, ToolSpec};
use serde_json::{Map, Value};
use std::marker::PhantomData;
use std::sync::Arc;

/// A tool as the session dispatches it.
pub trait Tool: Send + Sync {
    /// Its definition, made once.
    fn spec(&self) -> &ToolSpec;

    /// The document kinds it serves.
    fn kinds(&self) -> &[DocKind];

    /// Run it on `doc` with `arguments` already taken as meant and validated against
    /// [`Tool::spec`], stopping at a safe point when `cancel` says so.
    ///
    /// # Errors
    /// The tool's refusal or failure.
    fn run(&self, arguments: Map<String, Value>, doc: &Opened, cancel: &dyn Cancel) -> Result<Done<Value>, ToolError>;
}

/// A reading tool's handler: the typed arguments, the package at the version read, the
/// document's name, the call's cancellation.
pub type ReadFn<T> = fn(&T, &Package, &str, &dyn Cancel) -> Result<Done<Value>, ToolError>;

/// A reading tool: its arguments' struct `T` and its handler.
pub struct ReadHandler<T> {
    spec: ToolSpec,
    kinds: Vec<DocKind>,
    run: ReadFn<T>,
    args: PhantomData<fn() -> T>,
}

impl<T: ToolArgs> ReadHandler<T> {
    /// The tool `T` serving `kinds` with `run`.
    #[must_use]
    pub fn new(kinds: &[DocKind], run: ReadFn<T>) -> Self {
        Self {
            spec: T::spec(),
            kinds: kinds.to_vec(),
            run,
            args: PhantomData,
        }
    }
}

impl<T: ToolArgs> Tool for ReadHandler<T> {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn kinds(&self) -> &[DocKind] {
        &self.kinds
    }

    fn run(&self, arguments: Map<String, Value>, doc: &Opened, cancel: &dyn Cancel) -> Result<Done<Value>, ToolError> {
        let args = T::from_arg(&Value::Object(arguments), "")
            .map_err(|e| ToolError::internal(e.to_string(), &format!("{}:arguments", self.spec.name)))?;
        (self.run)(&args, doc.package(), doc.name(), cancel)
    }
}

/// The tools a session dispatches, in registration order.
#[derive(Clone, Default)]
pub struct Registry {
    tools: Vec<Arc<dyn Tool>>,
}

impl Registry {
    /// No tools.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// The tools ported so far: Word's `describe`.
    #[must_use]
    pub fn standard() -> Self {
        let mut registry = Self::empty();
        registry.register(Arc::new(ReadHandler::<ooxml_tools_word::Describe>::new(
            &[DocKind::Docx],
            ooxml_tools_word::describe,
        )));
        registry
    }

    /// Add `tool`; a tool of the same name serving other kinds is kept beside it (a call
    /// runs the one that serves its document's kind).
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.push(tool);
    }

    /// The first tool registered as `name` (tools of one name share the definition).
    #[must_use]
    pub fn named(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.spec().name == name)
    }

    /// The tool `name` serving `kind`.
    #[must_use]
    pub fn get(&self, name: &str, kind: DocKind) -> Option<&Arc<dyn Tool>> {
        self.tools
            .iter()
            .find(|t| t.spec().name == name && t.kinds().contains(&kind))
    }

    /// Every tool name, sorted, each once.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.iter().map(|t| t.spec().name.clone()).collect();
        names.sort();
        names.dedup();
        names
    }

    /// The exports (`ToolSpec::export`) of the tools serving `kind`, canonical for that kind,
    /// in registration order: what `tool_specs(kind)` gives the Python adapters.
    #[must_use]
    pub fn export(&self, kind: DocKind) -> Vec<Value> {
        let served = [kind.as_str()];
        self.tools
            .iter()
            .filter(|t| t.kinds().contains(&kind))
            .map(|t| t.spec().export(Some(&served)))
            .collect()
    }
}
