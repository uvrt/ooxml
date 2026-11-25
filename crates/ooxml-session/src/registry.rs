//! Owns: the tool [`Registry`]: each tool's [`ToolSpec`] made once when it is registered,
//! the document kind each serves, lookup by name, and the export per kind for the Python
//! adapters (`tool_specs`).
//!
//! The tools register themselves where they are defined: each format registers its tools
//! for its kind ([`crate::Format::register`]) through a [`Registrar`] of its types the
//! registry hands it ([`Registry::registrar`]), and [`Registry::of`] holds the formats'
//! tools. To add a tool, touch: its own files in its `ooxml-tools-*` crate (its
//! module with its `register` function, named in its group's `mod.rs`; its arguments,
//! description, tests and `tests/golden/<name>.json`) and its oracle cases (a function of
//! its own marked `@CASES` in `ooxml-oracle/python/word_*_cases.py`) -- not this file.
//! Every other view of the tool set derives from the registry: the session's tools and
//! refusals, `tool_specs` for the Python adapters, the golden exports' test
//! (`tests/integration/registry.rs`) and the oracle's Rust side.
//!
//! Must not: know providers, any format, or any tool's logic (the `ooxml-tools-*` crates
//! hold the handlers; `ooxml-tools` erases them, [`crate::call`] hands them the call).

use crate::{DocKind, Format};
use ooxml_tools::{
    EditFn, EditHandler, FormatTypes, ReadFn, ReadHandler, ReadingEditHandler, ReadsWhen, Registrar, Tool, ToolArgs,
    ToolSpec, TrackingFn,
};
use serde_json::Value;
use std::sync::Arc;

/// A tool and the document kind it serves.
#[derive(Clone)]
struct Entry {
    tool: Arc<dyn Tool>,
    kind: DocKind,
}

/// The tools a session dispatches, in registration order.
#[derive(Clone)]
pub struct Registry {
    tools: Vec<Entry>,
    /// `read_batch`'s definition: the session runs it over the others.
    batch: ToolSpec,
    /// `undo`'s definition: the session runs it over the documents' histories.
    undo: ToolSpec,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            tools: Vec::new(),
            batch: crate::ReadBatch::spec(),
            undo: crate::Undo::spec(),
        }
    }
}

impl Registry {
    /// No tools.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// The tools `formats` register, in their order.
    #[must_use]
    pub fn of(formats: &[Arc<dyn Format>]) -> Self {
        let mut registry = Self::empty();
        for format in formats {
            format.register(&mut registry);
        }
        registry
    }

    /// Add `tool`, serving `kind`; a tool of the same name serving another kind is kept
    /// beside it (a call runs the one that serves its document's kind).
    pub fn register(&mut self, kind: DocKind, tool: Arc<dyn Tool>) {
        self.tools.push(Entry { tool, kind });
    }

    /// Where a format's tools register, each serving `kind`.
    pub fn registrar(&mut self, kind: DocKind) -> KindRegistrar<'_> {
        KindRegistrar { registry: self, kind }
    }

    /// `read_batch`'s definition.
    #[must_use]
    pub fn batch_spec(&self) -> &ToolSpec {
        &self.batch
    }

    /// `undo`'s definition.
    #[must_use]
    pub fn undo_spec(&self) -> &ToolSpec {
        &self.undo
    }

    /// The definitions of the tools the session runs itself (`undo`, `read_batch`), served
    /// beside the registered ones.
    fn own(&self) -> [&ToolSpec; 2] {
        [&self.undo, &self.batch]
    }

    /// The first tool registered as `name` (tools of one name share the definition).
    #[must_use]
    pub fn named(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.iter().map(|e| &e.tool).find(|t| t.spec().name == name)
    }

    /// The tool `name` serving `kind`.
    #[must_use]
    pub fn get(&self, name: &str, kind: DocKind) -> Option<&Arc<dyn Tool>> {
        self.tools
            .iter()
            .find(|e| e.kind == kind && e.tool.spec().name == name)
            .map(|e| &e.tool)
    }

    /// Every tool name, sorted, each once.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.iter().map(|e| e.tool.spec().name.clone()).collect();
        if !self.tools.is_empty() {
            names.extend(self.own().map(|spec| spec.name.clone()));
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
            .filter(|e| e.kind == kind)
            .map(|e| e.tool.spec().export(Some(&served)))
            .collect();
        if !out.is_empty() {
            out.extend(self.own().map(|spec| spec.export(Some(&served))));
        }
        out
    }
}

/// Where a format's tools register, each serving one kind: a [`Registrar`] of any format's
/// types, its handlers erased as they are registered.
pub struct KindRegistrar<'r> {
    registry: &'r mut Registry,
    kind: DocKind,
}

impl<F: FormatTypes> Registrar<F> for KindRegistrar<'_> {
    fn read<T: ToolArgs + Send + Sync + 'static>(&mut self, run: ReadFn<F, T>) {
        let tool = ReadHandler::<F, T>::new(run);
        self.registry.register(self.kind, Arc::new(tool));
    }

    fn edit<T: ToolArgs + Send + Sync + 'static>(&mut self, run: EditFn<F, T>) {
        let tool = EditHandler::<F, T>::new(run);
        self.registry.register(self.kind, Arc::new(tool));
    }

    fn tracking<T: ToolArgs + Send + Sync + 'static>(&mut self, run: TrackingFn<F, T>) {
        let tool = EditHandler::<F, T>::tracking(run);
        self.registry.register(self.kind, Arc::new(tool));
    }

    fn reading_edit<T: ToolArgs + ReadsWhen + Send + Sync + 'static>(
        &mut self,
        read: ReadFn<F, T>,
        edit: EditFn<F, T>,
    ) {
        let tool = ReadingEditHandler::<F, T>::new(read, edit);
        self.registry.register(self.kind, Arc::new(tool));
    }
}
