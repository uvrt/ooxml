//! Owns: the tool [`Registry`]: each tool's [`ToolSpec`] made once when it is registered,
//! the document kinds each serves, lookup by name, and the specs' export per kind for the
//! Python adapters; [`Tool`], a tool as the session dispatches it, with the [`Call`] it is
//! handed; [`ReadHandler`], a reading tool made from a `ToolArgs` struct and a handler
//! function of a version; and [`EditHandler`], an editing tool made from a `ToolArgs`
//! struct and a handler function of the document's editor and what the session keeps of it
//! ([`Editing`]: the caller, the version it was opened as, its tracking mode).
//!
//! Must not: run a handler outside a call the session dispatches, know providers, or know
//! any tool's logic (the `ooxml-tools-*` crates hold the handlers).

use crate::document::Word;
use crate::session::Opened;
use crate::DocKind;
use ooxml_base::Cancel;
use ooxml_store::Caller;
use ooxml_tools::{Done, Failure, ToolArgs, ToolError, ToolResult, ToolSpec};
use ooxml_tools_word::{Editing, Tracked, Version};
use serde_json::{Map, Value};
use std::marker::PhantomData;
use std::sync::Arc;

/// A tool as the session dispatches it.
pub trait Tool: Send + Sync {
    /// Its definition, made once.
    fn spec(&self) -> &ToolSpec;

    /// The document kinds it serves.
    fn kinds(&self) -> &[DocKind];

    /// Run it with `arguments` already taken as meant and validated against
    /// [`Tool::spec`], on the document `call` names, stopping at a safe point when the
    /// call's cancellation says so. Never fails: a refusal is the result's error.
    fn run(&self, arguments: Map<String, Value>, call: &Call<'_>) -> ToolResult<Value>;
}

/// What a tool is handed: the document's version the call reads, who calls, the call's
/// cancellation, and for a Word document the session's editing state.
pub struct Call<'a> {
    pub(crate) doc: &'a Opened,
    pub(crate) word: Option<&'a Word>,
    pub(crate) caller: &'a Caller,
    pub(crate) cancel: &'a dyn Cancel,
    /// The revision date edits made now carry (the session's clock, to the minute).
    pub(crate) date: String,
}

impl Call<'_> {
    /// The document at the version the call reads.
    #[must_use]
    pub fn doc(&self) -> &Opened {
        self.doc
    }

    /// The call's cancellation.
    #[must_use]
    pub fn cancel(&self) -> &dyn Cancel {
        self.cancel
    }
}

/// A reading tool's handler: the typed arguments, the version read (its package, name,
/// number and the document's kept readings), the call's cancellation.
pub type ReadFn<T> = fn(&T, &Version<'_>, &dyn Cancel) -> Result<Done<Value>, ToolError>;

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

    fn run(&self, arguments: Map<String, Value>, call: &Call<'_>) -> ToolResult<Value> {
        let outcome = T::from_arg(&Value::Object(arguments), "")
            .map_err(|e| ToolError::internal(e.to_string(), &format!("{}:arguments", self.spec.name)))
            .and_then(|args| (self.run)(&args, &call.doc.version_of(), call.cancel));
        match outcome {
            Ok(done) => ToolResult::done(done),
            Err(error) => ToolResult::failed(error),
        }
    }
}

/// An editing tool's handler: the typed arguments and the document's editing state.
pub type EditFn<T> = fn(&T, &Editing<'_>) -> ToolResult<Value>;

/// A handler that also leaves the document's tracking mode (`word_set_tracking`): the reply,
/// and the mode the session keeps from now on (`None`: unchanged).
pub type TrackingFn<T> = fn(&T, &Editing<'_>) -> (ToolResult<Value>, Option<Tracked>);

enum EditRun<T> {
    Plain(EditFn<T>),
    Tracking(TrackingFn<T>),
}

/// An editing tool: its arguments' struct `T` and its handler, run on the head of a Word
/// document the session owns; each edit it makes is one command through the editor (one
/// history step), written under the document's tracking mode.
pub struct EditHandler<T> {
    spec: ToolSpec,
    kinds: Vec<DocKind>,
    run: EditRun<T>,
    args: PhantomData<fn() -> T>,
}

impl<T: ToolArgs> EditHandler<T> {
    /// The tool `T` serving `kinds` with `run`.
    #[must_use]
    pub fn new(kinds: &[DocKind], run: EditFn<T>) -> Self {
        Self::with(kinds, EditRun::Plain(run))
    }

    /// The tool `T` serving `kinds` with `run`, which also sets the tracking mode.
    #[must_use]
    pub fn tracking(kinds: &[DocKind], run: TrackingFn<T>) -> Self {
        Self::with(kinds, EditRun::Tracking(run))
    }

    fn with(kinds: &[DocKind], run: EditRun<T>) -> Self {
        Self {
            spec: T::spec(),
            kinds: kinds.to_vec(),
            run,
            args: PhantomData,
        }
    }
}

impl<T: ToolArgs> Tool for EditHandler<T> {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn kinds(&self) -> &[DocKind] {
        &self.kinds
    }

    fn run(&self, arguments: Map<String, Value>, call: &Call<'_>) -> ToolResult<Value> {
        let Some(word) = call.word else {
            let message = format!("{} edits Word documents the session owns", self.spec.name);
            return ToolResult::failed(ToolError::failure(Failure::Internal, message));
        };
        let args = match T::from_arg(&Value::Object(arguments), "") {
            Ok(args) => args,
            Err(e) => {
                return ToolResult::failed(ToolError::internal(
                    e.to_string(),
                    &format!("{}:arguments", self.spec.name),
                ))
            }
        };
        let policy = word.policy(&call.date);
        let tracking = match &policy {
            ooxml_word_edit::RevisionPolicy::Tracked(tracking) => Some(tracking),
            ooxml_word_edit::RevisionPolicy::Untracked => None,
        };
        let doc = call.doc.id().to_string();
        let editing = Editing {
            editor: &word.editor,
            caller: call.caller,
            opened: word.opened.doc(),
            tracking,
            doc: &doc,
            cancel: call.cancel,
        };
        let before = word.editor.store().snapshot();
        let mut result = match &self.run {
            EditRun::Plain(run) => run(&args, &editing),
            EditRun::Tracking(run) => {
                let (result, mode) = run(&args, &editing);
                if let Some(mode) = mode {
                    word.set_tracking(mode);
                }
                result
            }
        };
        let after = word.editor.store().snapshot();
        if after.seq() != before.seq() {
            // What the call changed, from the change itself (the parts it wrote, the body
            // blocks it replaced): the application's `touched`, read from no other block.
            match ooxml_word_edit::touched::touched(before.doc(), after.doc()) {
                Ok(found) => result.app.touched = found.iter().map(touched_json).collect(),
                Err(e) => return ToolResult::failed(ToolError::internal(e.to_string(), "word:touched")),
            }
        }
        result
    }
}

/// One `touched` entry as docx-agent's `tools/touched.py` writes it.
fn touched_json(found: &ooxml_word_edit::touched::Touched) -> Value {
    let mut entry = Map::new();
    entry.insert("part".into(), Value::from(found.part.as_str()));
    if let Some(section) = &found.section {
        entry.insert("heading".into(), section.heading.as_deref().map_or(Value::Null, Value::from));
        entry.insert("level".into(), Value::from(section.level));
        entry.insert("blocks".into(), Value::from(section.blocks.to_vec()));
    }
    Value::Object(entry)
}

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

    /// The tools ported so far: Word's `describe`, `word_read`, `word_inspect` and
    /// `find_text`, and `read_batch` over them; its review tools `word_set_tracking`,
    /// `word_changes` and `word_comments`; its text tools `word_set_text` and
    /// `replace_text`; the object tool `word_format_table`.
    #[must_use]
    pub fn standard() -> Self {
        let mut registry = Self::empty();
        registry.register(Arc::new(ReadHandler::<ooxml_tools_word::Describe>::new(
            &[DocKind::Docx],
            ooxml_tools_word::describe,
        )));
        registry.register(Arc::new(ReadHandler::<ooxml_tools_word::WordRead>::new(
            &[DocKind::Docx],
            ooxml_tools_word::word_read,
        )));
        registry.register(Arc::new(ReadHandler::<ooxml_tools_word::WordInspect>::new(
            &[DocKind::Docx],
            ooxml_tools_word::word_inspect,
        )));
        registry.register(Arc::new(ReadHandler::<ooxml_tools_word::FindText>::new(
            &[DocKind::Docx],
            ooxml_tools_word::find_text,
        )));
        registry.register(Arc::new(EditHandler::<ooxml_tools_word::WordSetTracking>::tracking(
            &[DocKind::Docx],
            ooxml_tools_word::word_set_tracking,
        )));
        registry.register(Arc::new(EditHandler::<ooxml_tools_word::WordChanges>::new(
            &[DocKind::Docx],
            ooxml_tools_word::word_changes,
        )));
        registry.register(Arc::new(EditHandler::<ooxml_tools_word::WordComments>::new(
            &[DocKind::Docx],
            ooxml_tools_word::word_comments,
        )));
        registry.register(Arc::new(EditHandler::<ooxml_tools_word::WordSetText>::new(
            &[DocKind::Docx],
            ooxml_tools_word::word_set_text,
        )));
        registry.register(Arc::new(EditHandler::<ooxml_tools_word::ReplaceText>::new(
            &[DocKind::Docx],
            ooxml_tools_word::replace_text,
        )));
        registry.register(Arc::new(EditHandler::<ooxml_tools_word::WordFormatTable>::new(
            &[DocKind::Docx],
            ooxml_tools_word::word_format_table,
        )));
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
}
