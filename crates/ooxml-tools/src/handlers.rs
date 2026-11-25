//! Owns: the handler side of the tool contract (design phase 3, section 3) -- how a format's
//! tools register where they are defined ([`Registrar`], over the format's types
//! [`FormatTypes`]), the handler shapes it takes ([`ReadFn`], [`EditFn`], [`TrackingFn`]),
//! and their erasure ([`Tool`]): a tool run with its arguments taken as meant and a
//! [`CallEnv`], whose format context runs the typed handler ([`Run`]) on the version the
//! call reads or through the document's editor.
//!
//! A format's context of a call is its own: the host builds it for the document the call
//! names, and it reads a [`Run`] of its own format only (a downcast, in safe code). Nothing
//! here fixes how a document's blocks are named: addresses stay the tools' arguments.
//!
//! Must not: hold a registry or documents (the session does), know any format's types, or
//! run a handler outside the context a host gives.

use crate::{Done, ReadsWhen, ToolArgs, ToolError, ToolResult, ToolSpec};
use ooxml_base::Cancel;
use serde_json::{Map, Value};
use std::any::Any;
use std::marker::PhantomData;

/// The types a format's handlers are given: what a reading tool reads, what an editing tool
/// edits through, and the mode a tool may leave the document in (Word: its tracking).
pub trait FormatTypes: 'static {
    /// One version as a reading tool reads it.
    type Version<'a>;
    /// What an editing tool is handed: the document's editor and what the host keeps of it.
    type Editing<'a>;
    /// The mode a tool leaves the document in, which the host keeps from then on.
    type Tracked: 'static;
}

/// A reading tool's handler: the typed arguments, the version read, the call's cancellation.
pub type ReadFn<F, T> = fn(&T, &<F as FormatTypes>::Version<'_>, &dyn Cancel) -> Result<Done<Value>, ToolError>;

/// An editing tool's handler: the typed arguments and the document's editing state.
pub type EditFn<F, T> = fn(&T, &<F as FormatTypes>::Editing<'_>) -> ToolResult<Value>;

/// A handler that also leaves the document's mode (`word_set_tracking`): the reply, and the
/// mode the host keeps from now on (`None`: unchanged).
pub type TrackingFn<F, T> =
    fn(&T, &<F as FormatTypes>::Editing<'_>) -> (ToolResult<Value>, Option<<F as FormatTypes>::Tracked>);

/// Where a format's tools register: a registry of the tools a session dispatches.
pub trait Registrar<F: FormatTypes> {
    /// The reading tool `T`, answered by `run`.
    fn read<T: ToolArgs + Send + Sync + 'static>(&mut self, run: ReadFn<F, T>);

    /// The editing tool `T`, answered by `run`.
    fn edit<T: ToolArgs + Send + Sync + 'static>(&mut self, run: EditFn<F, T>);

    /// The editing tool `T` that also sets the document's mode.
    fn tracking<T: ToolArgs + Send + Sync + 'static>(&mut self, run: TrackingFn<F, T>);

    /// The editing tool `T` some of whose calls only read ([`ReadsWhen`]: `edit_chart`'s
    /// `action: "read"`): `read` answers those on the version the call pins, `edit` the
    /// others.
    fn reading_edit<T: ToolArgs + ReadsWhen + Send + Sync + 'static>(&mut self, read: ReadFn<F, T>, edit: EditFn<F, T>);
}

/// A reading call's handler, its arguments taken.
pub type ReadRun<F> = Box<dyn Fn(&<F as FormatTypes>::Version<'_>, &dyn Cancel) -> Result<Done<Value>, ToolError>>;

/// An editing call's handler, its arguments taken.
pub type EditRun<F> = Box<dyn Fn(&<F as FormatTypes>::Editing<'_>) -> ToolResult<Value>>;

/// An editing call's handler that also leaves the document's mode, its arguments taken.
pub type TrackingRun<F> =
    Box<dyn Fn(&<F as FormatTypes>::Editing<'_>) -> (ToolResult<Value>, Option<<F as FormatTypes>::Tracked>)>;

/// One call's typed handler, its arguments taken, as a format's context runs it.
pub enum Run<F: FormatTypes> {
    /// Read the version the call reads.
    Read(ReadRun<F>),
    /// Edit through the document's editor.
    Edit(EditRun<F>),
    /// Edit, and leave the document's mode.
    Tracking(TrackingRun<F>),
}

/// A format's context of one call: it runs a tool's handler on the call's version, or
/// through the document's editor, with what the host hands the call.
pub trait FormatCall {
    /// Run `run` (a [`Run`] of this context's format) for the tool `tool`, whose arguments as
    /// the host took them are `arguments`. Never fails: a refusal is the result's error; a
    /// run of another format is answered as `internal`.
    fn run(&self, tool: &str, arguments: &Map<String, Value>, run: &dyn Any) -> ToolResult<Value>;
}

/// What a tool is handed for one call: every field required, set by the host.
pub struct CallEnv<'a> {
    /// The call's cancellation.
    pub cancel: &'a dyn Cancel,
    /// The format's context of the call, for the document the call names.
    pub format: &'a dyn FormatCall,
}

/// A tool as a host dispatches it, its format's types erased.
pub trait Tool: Send + Sync {
    /// Its definition, made once.
    fn spec(&self) -> &ToolSpec;

    /// Run it with `arguments` already taken as meant and validated against
    /// [`Tool::spec`], in `env`. Never fails: a refusal is the result's error.
    fn run(&self, arguments: &Map<String, Value>, env: &mut CallEnv<'_>) -> ToolResult<Value>;
}

/// `arguments` read as `T` (already taken as meant: a failure is a fault in the contract).
fn typed<T: ToolArgs>(spec: &ToolSpec, arguments: &Map<String, Value>) -> Result<T, ToolError> {
    T::from_arg(&Value::Object(arguments.clone()), "")
        .map_err(|e| ToolError::internal(e.to_string(), &format!("{}:arguments", spec.name)))
}

/// `run` in `env`'s format context.
fn run_in<F: FormatTypes>(
    spec: &ToolSpec,
    arguments: &Map<String, Value>,
    env: &CallEnv<'_>,
    run: Run<F>,
) -> ToolResult<Value> {
    env.format.run(&spec.name, arguments, &run)
}

/// A reading tool: its arguments' struct `T` and its handler.
pub struct ReadHandler<F: FormatTypes, T> {
    spec: ToolSpec,
    run: ReadFn<F, T>,
    args: PhantomData<fn() -> T>,
}

impl<F: FormatTypes, T: ToolArgs> ReadHandler<F, T> {
    /// The tool `T` answered by `run`.
    #[must_use]
    pub fn new(run: ReadFn<F, T>) -> Self {
        Self {
            spec: T::spec(),
            run,
            args: PhantomData,
        }
    }
}

impl<F: FormatTypes, T: ToolArgs + Send + Sync + 'static> Tool for ReadHandler<F, T> {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn run(&self, arguments: &Map<String, Value>, env: &mut CallEnv<'_>) -> ToolResult<Value> {
        let args: T = match typed(&self.spec, arguments) {
            Ok(args) => args,
            Err(error) => return ToolResult::failed(error),
        };
        let run = self.run;
        run_in::<F>(
            &self.spec,
            arguments,
            env,
            Run::Read(Box::new(move |version: &F::Version<'_>, cancel: &dyn Cancel| {
                run(&args, version, cancel)
            })),
        )
    }
}

enum EditKind<F: FormatTypes, T> {
    Plain(EditFn<F, T>),
    Tracking(TrackingFn<F, T>),
}

/// An editing tool: its arguments' struct `T` and its handler, run through the document's
/// editor; each edit it makes is one command (one history step).
pub struct EditHandler<F: FormatTypes, T> {
    spec: ToolSpec,
    run: EditKind<F, T>,
    args: PhantomData<fn() -> T>,
}

impl<F: FormatTypes, T: ToolArgs> EditHandler<F, T> {
    /// The tool `T` answered by `run`.
    #[must_use]
    pub fn new(run: EditFn<F, T>) -> Self {
        Self::with(EditKind::Plain(run))
    }

    /// The tool `T` answered by `run`, which also sets the document's mode.
    #[must_use]
    pub fn tracking(run: TrackingFn<F, T>) -> Self {
        Self::with(EditKind::Tracking(run))
    }

    fn with(run: EditKind<F, T>) -> Self {
        Self {
            spec: T::spec(),
            run,
            args: PhantomData,
        }
    }
}

impl<F: FormatTypes, T: ToolArgs + Send + Sync + 'static> Tool for EditHandler<F, T> {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn run(&self, arguments: &Map<String, Value>, env: &mut CallEnv<'_>) -> ToolResult<Value> {
        let args: T = match typed(&self.spec, arguments) {
            Ok(args) => args,
            Err(error) => return ToolResult::failed(error),
        };
        let run = match self.run {
            EditKind::Plain(run) => Run::Edit(Box::new(move |editing: &F::Editing<'_>| run(&args, editing))),
            EditKind::Tracking(run) => Run::Tracking(Box::new(move |editing: &F::Editing<'_>| run(&args, editing))),
        };
        run_in::<F>(&self.spec, arguments, env, run)
    }
}

/// An editing tool whose reading calls read the pinned version ([`ReadsWhen`]): its
/// arguments' struct `T`, its read handler and its edit handler.
pub struct ReadingEditHandler<F: FormatTypes, T> {
    spec: ToolSpec,
    read: ReadFn<F, T>,
    edit: EditFn<F, T>,
    args: PhantomData<fn() -> T>,
}

impl<F: FormatTypes, T: ToolArgs + ReadsWhen> ReadingEditHandler<F, T> {
    /// The tool `T`: `read` for the calls that read, `edit` for the others.
    #[must_use]
    pub fn new(read: ReadFn<F, T>, edit: EditFn<F, T>) -> Self {
        Self {
            spec: T::spec(),
            read,
            edit,
            args: PhantomData,
        }
    }
}

impl<F: FormatTypes, T: ToolArgs + ReadsWhen + Send + Sync + 'static> Tool for ReadingEditHandler<F, T> {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn run(&self, arguments: &Map<String, Value>, env: &mut CallEnv<'_>) -> ToolResult<Value> {
        let args: T = match typed(&self.spec, arguments) {
            Ok(args) => args,
            Err(error) => return ToolResult::failed(error),
        };
        let run = if args.reads() {
            let read = self.read;
            Run::Read(Box::new(move |version: &F::Version<'_>, cancel: &dyn Cancel| {
                read(&args, version, cancel)
            }))
        } else {
            let edit = self.edit;
            Run::Edit(Box::new(move |editing: &F::Editing<'_>| edit(&args, editing)))
        };
        run_in::<F>(&self.spec, arguments, env, run)
    }
}
