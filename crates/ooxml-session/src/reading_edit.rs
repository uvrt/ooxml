//! Owns: [`ReadingEditHandler`] -- an editing tool some of whose calls only read
//! ([`ReadsWhen`]: `edit_chart`'s `action: "read"`, today's `Tool.reads`). A reading call
//! runs the tool's read handler on the version the call pins, as a [`crate::ReadHandler`]
//! does; any other call runs its edit handler through the session's editor, as an
//! [`crate::EditHandler`] does.
//!
//! Must not: decide what reads (the arguments say, [`ReadsWhen::reads`]) or know any tool's
//! logic.

use crate::call::Call;
use crate::handlers::{accepted, Tool};
use crate::DocKind;
use ooxml_tools::{Failure, ReadsWhen, ToolArgs, ToolError, ToolResult, ToolSpec};
use ooxml_tools_word::{EditFn, Editing, Hosting, ReadFn, Version};
use serde_json::{Map, Value};
use std::marker::PhantomData;

/// An editing tool whose reading calls read the pinned version: its arguments' struct `T`,
/// its read handler and its edit handler.
pub struct ReadingEditHandler<T> {
    spec: ToolSpec,
    kinds: Vec<DocKind>,
    read: ReadFn<T>,
    edit: EditFn<T>,
    hosting: Hosting,
    args: PhantomData<fn() -> T>,
}

impl<T: ToolArgs + ReadsWhen> ReadingEditHandler<T> {
    /// The tool `T` serving `kinds`: `read` for the calls that read, `edit` for the others.
    #[must_use]
    pub fn new(kinds: &[DocKind], read: ReadFn<T>, edit: EditFn<T>) -> Self {
        Self {
            spec: T::spec(),
            kinds: kinds.to_vec(),
            read,
            edit,
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

impl<T: ToolArgs + ReadsWhen> Tool for ReadingEditHandler<T> {
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
        let args = match T::from_arg(&Value::Object(arguments), "") {
            Ok(args) => args,
            Err(e) => {
                return ToolResult::failed(ToolError::internal(
                    e.to_string(),
                    &format!("{}:arguments", self.spec.name),
                ))
            }
        };
        if args.reads() {
            return match call.read(|version| (self.read)(&args, version, call.cancel())) {
                Ok(done) => ToolResult::done(done),
                Err(error) => ToolResult::failed(error),
            };
        }
        let Some(word) = call.word() else {
            let message = format!("{} edits Word documents the session owns", self.spec.name);
            return ToolResult::failed(ToolError::failure(Failure::Internal, message));
        };
        call.edit(word, |editing| (self.edit)(&args, editing))
    }

    fn run_outside(&self, arguments: &Value, editing: &Editing<'_>, version: &Version<'_>) -> ToolResult<Value> {
        accepted(arguments, |args: &T| {
            if !args.reads() {
                return (self.edit)(args, editing);
            }
            match (self.read)(args, version, editing.cancel) {
                Ok(done) => ToolResult::done(done),
                Err(error) => ToolResult::failed(error),
            }
        })
    }
}
