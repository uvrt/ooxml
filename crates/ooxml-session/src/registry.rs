//! Owns: the tool [`Registry`]: each tool's [`ToolSpec`] made once when it is registered,
//! the document kinds each serves, lookup by name, and the specs' export per kind for the
//! Python adapters; [`Tool`], a tool as the session dispatches it, with the [`Call`] it is
//! handed; [`ReadHandler`], a reading tool made from a `ToolArgs` struct and a handler
//! function of a version; and [`EditHandler`], an editing tool made from a `ToolArgs`
//! struct and a handler function of the document's editor and what the session keeps of it
//! ([`Editing`]: the caller, the version it was opened as, its tracking mode).
//!
//! To add a tool, touch: its own files in its `ooxml-tools-*` crate (its module, named in
//! its group's `mod.rs`; its arguments, description, tests and `tests/golden/<name>.json`),
//! its entry in [`Registry::standard`], and its oracle cases (a function of its own marked
//! `@CASES` in `ooxml-oracle/python/word_*_cases.py`). Every other view of the tool set
//! derives from the registry: the session's tools and refusals, `tool_specs` for the Python
//! adapters, the golden exports' test (`tests/integration/registry.rs`) and the oracle's Rust side
//! ([`Tool::run_outside`]).
//!
//! Must not: run a handler outside a call the session dispatches (but for a test
//! instrument's document, [`Tool::run_outside`]), know providers, or know any tool's logic
//! (the `ooxml-tools-*` crates hold the handlers).

use crate::document::Word;
use crate::session::Opened;
use crate::DocKind;
use ooxml_base::Cancel;
use ooxml_store::Caller;
use ooxml_tools::{Done, Failure, ToolArgs, ToolError, ToolResult, ToolSpec};
use ooxml_tools_word::{CallEditor, Editing, Made, Others, Tracked, Version};
use serde_json::{Map, Value};
use std::marker::PhantomData;
use std::sync::{Arc, Mutex, PoisonError};

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

    /// Run it on a Word document a test instrument opened outside a session (the oracle's
    /// cases): `arguments` taken as meant for a Word document ([`ToolArgs::accept`]), the
    /// notes on what was read as what first among the reply's warnings; a reading tool reads
    /// `version`, an editing tool edits through `editing` (a tracking mode it sets is not
    /// kept). Never fails: a refusal is the result's error.
    fn run_outside(&self, arguments: &Value, editing: &Editing<'_>, version: &Version<'_>) -> ToolResult<Value>;
}

/// `handler`'s reply to `arguments` taken as meant for a Word document (a refusal of the
/// arguments is the reply), the notes on what was read as what first among its warnings.
fn accepted<T: ToolArgs>(arguments: &Value, handler: impl FnOnce(&T) -> ToolResult<Value>) -> ToolResult<Value> {
    match T::accept(arguments.clone(), Some(DocKind::Docx.as_str())) {
        Ok((args, notes)) => {
            let mut reply = handler(&args);
            if !notes.is_empty() {
                let mut warnings = notes;
                warnings.append(&mut reply.warnings);
                reply.warnings = warnings;
            }
            reply
        }
        Err(error) => ToolResult::failed(error),
    }
}

/// What a tool is handed: the document's version the call reads, who calls, the call's
/// cancellation, and for a Word document the session's editing state.
pub struct Call<'a> {
    pub(crate) doc: &'a Opened,
    /// The name the call gives its document (the host's; the session's id where the host
    /// gives none): what its reply names it by.
    pub(crate) name: &'a str,
    pub(crate) word: Option<&'a Word>,
    pub(crate) caller: &'a Caller,
    pub(crate) cancel: &'a dyn Cancel,
    /// The revision date edits made now carry (the session's clock, to the minute).
    pub(crate) date: String,
    /// The versions the call's edits made (an editing tool's: [`CallEditor::made`]).
    pub(crate) made: &'a Mutex<Option<Made>>,
    /// The host's layout of the document's versions.
    pub(crate) layout: Option<&'a dyn ooxml_word_edit::FieldLayout>,
    /// The session's blobs, where the host hands them over.
    pub(crate) blobs: Option<&'a dyn ooxml_tools_word::Blobs>,
    /// The other documents the call names (`ooxml_tools::documents`), by id, each at the
    /// version pinned for the call when it was dispatched.
    pub(crate) others: &'a [(String, Opened)],
}

impl Call<'_> {
    /// The document at the version the call reads.
    #[must_use]
    pub fn doc(&self) -> &Opened {
        self.doc
    }

    /// The name the call gives its document, which its reply names it by.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name
    }

    /// The call's cancellation.
    #[must_use]
    pub fn cancel(&self) -> &dyn Cancel {
        self.cancel
    }

    /// The other documents the call names, by id, at their pinned versions.
    #[must_use]
    pub fn others(&self) -> &[(String, Opened)] {
        self.others
    }

    /// [`Call::others`] as a tool reads them.
    fn other_versions(&self) -> Vec<(&str, Version<'_>)> {
        self.others
            .iter()
            .map(|(id, o)| (id.as_str(), o.version_of()))
            .collect()
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
            .and_then(|args| {
                let others = call.other_versions();
                let version = Version {
                    others: Others::new(&others),
                    ..call.doc.version_of()
                };
                (self.run)(&args, &version, call.cancel)
            });
        match outcome {
            Ok(done) => ToolResult::done(done),
            Err(error) => ToolResult::failed(error),
        }
    }

    fn run_outside(&self, arguments: &Value, editing: &Editing<'_>, version: &Version<'_>) -> ToolResult<Value> {
        accepted(arguments, |args: &T| match (self.run)(args, version, editing.cancel) {
            Ok(done) => ToolResult::done(done),
            Err(error) => ToolResult::failed(error),
        })
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
        let doc = call.name.to_owned();
        let editor = CallEditor::new(&word.editor);
        let others = call.other_versions();
        let editing = Editing {
            editor: &editor,
            caller: call.caller,
            opened: word.opened.doc(),
            tracking,
            doc: &doc,
            cancel: call.cancel,
            date: &call.date,
            layout: call.layout,
            others: Others::new(&others),
            blobs: call.blobs,
        };
        let result = match &self.run {
            EditRun::Plain(run) => run(&args, &editing),
            EditRun::Tracking(run) => {
                let (result, mode) = run(&args, &editing);
                if let Some(mode) = mode {
                    word.set_tracking(mode);
                }
                result
            }
        };
        *call.made.lock().unwrap_or_else(PoisonError::into_inner) = editor.made();
        result
    }

    fn run_outside(&self, arguments: &Value, editing: &Editing<'_>, _: &Version<'_>) -> ToolResult<Value> {
        accepted(arguments, |args: &T| match &self.run {
            EditRun::Plain(run) => run(args, editing),
            EditRun::Tracking(run) => run(args, editing).0,
        })
    }
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

/// A Word reading tool's entry.
fn word_read<T: ToolArgs + 'static>(run: ReadFn<T>) -> Arc<dyn Tool> {
    Arc::new(ReadHandler::<T>::new(&[DocKind::Docx], run))
}

/// A Word editing tool's entry.
fn word_edit<T: ToolArgs + 'static>(run: EditFn<T>) -> Arc<dyn Tool> {
    Arc::new(EditHandler::<T>::new(&[DocKind::Docx], run))
}

impl Registry {
    /// No tools.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// The tools ported so far, and `read_batch` over the reading ones: the one list of the
    /// tools (a new tool adds its entry here, nowhere else). The order is the export's
    /// (`describe` first).
    #[must_use]
    pub fn standard() -> Self {
        let mut registry = Self::empty();
        registry.register(word_read::<ooxml_tools_word::Describe>(ooxml_tools_word::describe));
        registry.register(word_read::<ooxml_tools_word::WordRead>(ooxml_tools_word::word_read));
        registry.register(word_read::<ooxml_tools_word::WordInspect>(
            ooxml_tools_word::word_inspect,
        ));
        registry.register(word_read::<ooxml_tools_word::FindText>(ooxml_tools_word::find_text));
        registry.register(Arc::new(EditHandler::<ooxml_tools_word::WordSetTracking>::tracking(
            &[DocKind::Docx],
            ooxml_tools_word::word_set_tracking,
        )));
        registry.register(word_edit::<ooxml_tools_word::WordChanges>(
            ooxml_tools_word::word_changes,
        ));
        registry.register(word_edit::<ooxml_tools_word::WordComments>(
            ooxml_tools_word::word_comments,
        ));
        registry.register(word_edit::<ooxml_tools_word::WordSetText>(
            ooxml_tools_word::word_set_text,
        ));
        registry.register(word_edit::<ooxml_tools_word::ReplaceText>(
            ooxml_tools_word::replace_text,
        ));
        registry.register(word_edit::<ooxml_tools_word::WordInsertText>(
            ooxml_tools_word::word_insert_text,
        ));
        registry.register(word_edit::<ooxml_tools_word::WordDelete>(ooxml_tools_word::word_delete));
        registry.register(word_edit::<ooxml_tools_word::WordInsertMarkdown>(
            ooxml_tools_word::word_insert_markdown,
        ));
        registry.register(word_edit::<ooxml_tools_word::WordFormatTable>(
            ooxml_tools_word::word_format_table,
        ));
        registry.register(word_edit::<ooxml_tools_word::WordEditTable>(
            ooxml_tools_word::word_edit_table,
        ));
        registry.register(word_edit::<ooxml_tools_word::WordDrawings>(
            ooxml_tools_word::word_drawings,
        ));
        registry.register(word_edit::<ooxml_tools_word::WordControls>(
            ooxml_tools_word::word_controls,
        ));
        registry.register(word_edit::<ooxml_tools_word::WordLists>(ooxml_tools_word::word_lists));
        registry.register(word_edit::<ooxml_tools_word::WordFields>(ooxml_tools_word::word_fields));
        registry.register(word_edit::<ooxml_tools_word::WordHeadersFooters>(
            ooxml_tools_word::word_headers_footers,
        ));
        registry.register(word_edit::<ooxml_tools_word::WordFormat>(ooxml_tools_word::word_format));
        registry.register(word_edit::<ooxml_tools_word::WordStyles>(ooxml_tools_word::word_styles));
        registry.register(word_edit::<ooxml_tools_word::WordTemplate>(
            ooxml_tools_word::word_template,
        ));
        registry.register(word_read::<ooxml_tools_word::WordCompareBlocks>(
            ooxml_tools_word::word_compare_blocks,
        ));
        registry.register(word_edit::<ooxml_tools_word::WordFollowSample>(
            ooxml_tools_word::word_follow_sample,
        ));
        registry.register(word_read::<ooxml_tools_word::WordDesignFacts>(
            ooxml_tools_word::word_design_facts,
        ));
        registry.register(word_edit::<ooxml_tools_word::WordCopyFrom>(
            ooxml_tools_word::word_copy_from,
        ));
        registry.register(word_edit::<ooxml_tools_word::WordMove>(ooxml_tools_word::word_move));
        registry.register(word_edit::<ooxml_tools_word::WordSections>(
            ooxml_tools_word::word_sections,
        ));
        registry.register(word_edit::<ooxml_tools_word::WordNotes>(ooxml_tools_word::word_notes));
        registry.register(word_edit::<ooxml_tools_word::WordLinks>(ooxml_tools_word::word_links));
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
