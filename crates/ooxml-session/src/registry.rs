//! Owns: the tool [`Registry`]: each tool's [`ToolSpec`] made once when it is registered,
//! the document kinds each serves, lookup by name, and the export per kind for the Python
//! adapters -- the specs (`tool_specs`) and how a host hands each tool's calls to the
//! session (`tool_hosting`, [`Hosting`]).
//!
//! The tools register themselves where they are defined: the registry is a
//! [`ooxml_tools_word::Registrar`], and [`Registry::standard`] hands it to
//! [`ooxml_tools_word::register`]. To add a tool, touch: its own files in its
//! `ooxml-tools-*` crate (its module with its `register` function, named in its group's
//! `mod.rs`; its arguments, description, tests and `tests/golden/<name>.json`) and its oracle
//! cases (a function of its own marked `@CASES` in `ooxml-oracle/python/word_*_cases.py`) --
//! not this file. Every other view of the tool set derives from the registry: the session's
//! tools and refusals, `tool_specs` and `tool_hosting` for the Python adapters, the golden
//! exports' test (`tests/integration/registry.rs`) and the oracle's Rust side
//! ([`Tool::run_outside`]).
//!
//! Must not: know providers or any tool's logic (the `ooxml-tools-*` crates hold the
//! handlers; [`crate::handlers`] runs them, [`crate::call`] hands them the call).

use crate::handlers::{EditHandler, ReadHandler, Tool};
use crate::DocKind;
use ooxml_tools::{ToolArgs, ToolSpec};
use ooxml_tools_word::{EditFn, Hosting, ReadFn, Registrar, TrackingFn};
use serde_json::{Map, Value};
use std::sync::Arc;

/// The tools a session dispatches, in registration order.
#[derive(Clone)]
pub struct Registry {
    tools: Vec<Arc<dyn Tool>>,
    /// `read_batch`'s definition: the session runs it over the others.
    batch: ToolSpec,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            tools: Vec::new(),
            batch: crate::ReadBatch::spec(),
        }
    }
}

impl Registry {
    /// No tools.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// The tools ported so far, each registered where it is defined
    /// ([`ooxml_tools_word::register`]), and `read_batch` over the reading ones. The order is
    /// the export's: the registration's, `describe` first.
    #[must_use]
    pub fn standard() -> Self {
        let mut registry = Self::empty();
        ooxml_tools_word::register(&mut registry);
        registry
    }

    /// Add `tool`; a tool of the same name serving other kinds is kept beside it (a call
    /// runs the one that serves its document's kind).
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.push(tool);
    }

    /// `read_batch`'s definition.
    #[must_use]
    pub fn batch_spec(&self) -> &ToolSpec {
        &self.batch
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
        if !self.tools.is_empty() {
            names.push(self.batch.name.clone());
        }
        names.sort();
        names.dedup();
        names
    }

    /// The exports (`ToolSpec::export`) of the tools serving `kind`, canonical for that kind,
    /// in registration order: what `tool_specs(kind)` gives the Python adapters.
    #[must_use]
    pub fn export(&self, kind: DocKind) -> Vec<Value> {
        let served = [kind.as_str()];
        let mut out: Vec<Value> = self
            .tools
            .iter()
            .filter(|t| t.kinds().contains(&kind))
            .map(|t| t.spec().export(Some(&served)))
            .collect();
        if !out.is_empty() {
            out.push(self.batch.export(Some(&served)));
        }
        out
    }

    /// How a host hands each tool serving `kind` its calls ([`Tool::hosting`]), by name,
    /// sorted: what `tool_hosting(kind)` gives the Python adapters.
    #[must_use]
    pub fn hosting(&self, kind: DocKind) -> Map<String, Value> {
        let mut found: Vec<(&str, Value)> = self
            .tools
            .iter()
            .filter(|t| t.kinds().contains(&kind))
            .map(|t| (t.spec().name.as_str(), t.hosting().export()))
            .collect();
        found.sort_by(|a, b| a.0.cmp(b.0));
        found.into_iter().map(|(name, hosting)| (name.to_owned(), hosting)).collect()
    }
}

/// Word's tools register here as Word (`docx`) tools.
impl Registrar for Registry {
    fn read<T: ToolArgs + 'static>(&mut self, run: ReadFn<T>) {
        self.register(Arc::new(ReadHandler::<T>::new(&[DocKind::Docx], run)));
    }

    fn edit<T: ToolArgs + 'static>(&mut self, run: EditFn<T>, hosting: Hosting) {
        self.register(Arc::new(EditHandler::<T>::new(&[DocKind::Docx], run).hosted(hosting)));
    }

    fn tracking<T: ToolArgs + 'static>(&mut self, run: TrackingFn<T>, hosting: Hosting) {
        self.register(Arc::new(EditHandler::<T>::tracking(&[DocKind::Docx], run).hosted(hosting)));
    }
}
