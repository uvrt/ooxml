//! Owns: [`Tool`], a tool as the session dispatches it; [`ReadHandler`], a reading tool made
//! from a `ToolArgs` struct and a handler function of a version; and [`EditHandler`], an
//! editing tool made from a `ToolArgs` struct and a handler function of the document's
//! editor and what the session keeps of it ([`Editing`]: the caller, the version it was
//! opened as, its tracking mode, the host's context, all from the [`Call`]).
//!
//! Must not: run a handler outside a call the session dispatches (but for a test
//! instrument's document, [`Tool::run_outside`]), or know any tool's logic.

use crate::call::Call;
use crate::DocKind;
use ooxml_tools::{Failure, ToolArgs, ToolError, ToolResult, ToolSpec};
use ooxml_tools_word::{EditFn, Editing, Hosting, ReadFn, TrackingFn, Version};
use serde_json::{Map, Value};
use std::marker::PhantomData;

/// A tool as the session dispatches it.
pub trait Tool: Send + Sync {
    /// Its definition, made once.
    fn spec(&self) -> &ToolSpec;

    /// The document kinds it serves.
    fn kinds(&self) -> &[DocKind];

    /// How a host hands its calls to the session ([`Hosting`]; a reading tool: the host's,
    /// unless it declares otherwise).
    fn hosting(&self) -> Hosting {
        Hosting::HOST
    }

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
pub(crate) fn accepted<T: ToolArgs>(
    arguments: &Value,
    handler: impl FnOnce(&T) -> ToolResult<Value>,
) -> ToolResult<Value> {
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

/// A reading tool: its arguments' struct `T` and its handler.
pub struct ReadHandler<T> {
    spec: ToolSpec,
    kinds: Vec<DocKind>,
    run: ReadFn<T>,
    hosting: Hosting,
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
            hosting: Hosting::HOST,
            args: PhantomData,
        }
    }

    /// The tool hosted as `hosting` says (default: the host answers its calls).
    #[must_use]
    pub fn hosted(self, hosting: Hosting) -> Self {
        Self { hosting, ..self }
    }
}

impl<T: ToolArgs> Tool for ReadHandler<T> {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn kinds(&self) -> &[DocKind] {
        &self.kinds
    }

    fn hosting(&self) -> Hosting {
        self.hosting
    }

    fn run(&self, arguments: Map<String, Value>, call: &Call<'_>) -> ToolResult<Value> {
        let outcome = T::from_arg(&Value::Object(arguments), "")
            .map_err(|e| ToolError::internal(e.to_string(), &format!("{}:arguments", self.spec.name)))
            .and_then(|args| call.read(|version| (self.run)(&args, version, call.cancel())));
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
    hosting: Hosting,
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
            hosting: Hosting::HOST,
            args: PhantomData,
        }
    }

    /// The tool hosted as `hosting` says (default: the host answers its calls).
    #[must_use]
    pub fn hosted(self, hosting: Hosting) -> Self {
        Self { hosting, ..self }
    }
}

impl<T: ToolArgs> Tool for EditHandler<T> {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn kinds(&self) -> &[DocKind] {
        &self.kinds
    }

    fn hosting(&self) -> Hosting {
        self.hosting
    }

    fn run(&self, arguments: Map<String, Value>, call: &Call<'_>) -> ToolResult<Value> {
        let Some(word) = call.word() else {
            let message = format!("{} edits Word documents the session owns", self.spec.name);
            return ToolResult::failed(ToolError::failure(Failure::Internal, message));
        };
        let arguments = Value::Object(arguments);
        let args = match T::from_arg(&arguments, "") {
            Ok(args) => args,
            Err(e) => {
                return ToolResult::failed(ToolError::internal(
                    e.to_string(),
                    &format!("{}:arguments", self.spec.name),
                ))
            }
        };
        call.edit(word, arguments.as_object(), |editing| match &self.run {
            EditRun::Plain(run) => run(&args, editing),
            EditRun::Tracking(run) => {
                let (result, mode) = run(&args, editing);
                if let Some(mode) = mode {
                    word.set_tracking(mode);
                }
                result
            }
        })
    }

    fn run_outside(&self, arguments: &Value, editing: &Editing<'_>, _: &Version<'_>) -> ToolResult<Value> {
        accepted(arguments, |args: &T| match &self.run {
            EditRun::Plain(run) => run(args, editing),
            EditRun::Tracking(run) => run(args, editing).0,
        })
    }
}
