//! Owns: [`ReadingEditHandler`] -- an editing tool some of whose calls only read
//! ([`ReadsWhen`]: `edit_chart`'s `action: "read"`, today's `Tool.reads`). A reading call
//! runs the tool's read handler on the version the call pins, as a [`crate::ReadHandler`]
//! does; any other call runs its edit handler through the session's editor, as an
//! [`crate::EditHandler`] does.
//!
//! Must not: decide what reads (the arguments say, [`ReadsWhen::reads`]) or know any tool's
//! logic.

use crate::registry::{accepted, edit_head, read_pinned, typed, EditFn, ReadFn};
use crate::{Call, DocKind, Tool};
use ooxml_tools::{ReadsWhen, ToolArgs, ToolResult, ToolSpec};
use ooxml_tools_word::{Editing, Version};
use serde_json::{Map, Value};
use std::marker::PhantomData;

/// An editing tool whose reading calls read the pinned version: its arguments' struct `T`,
/// its read handler and its edit handler.
pub struct ReadingEditHandler<T> {
    spec: ToolSpec,
    kinds: Vec<DocKind>,
    read: ReadFn<T>,
    edit: EditFn<T>,
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
            args: PhantomData,
        }
    }
}

impl<T: ToolArgs + ReadsWhen> Tool for ReadingEditHandler<T> {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn kinds(&self) -> &[DocKind] {
        &self.kinds
    }

    fn run(&self, arguments: Map<String, Value>, call: &Call<'_>) -> ToolResult<Value> {
        match typed::<T>(arguments, &self.spec) {
            Ok(args) if args.reads() => read_pinned(&args, self.read, call),
            Ok(args) => edit_head(&self.spec, call, |editing, _| (self.edit)(&args, editing)),
            Err(error) => ToolResult::failed(error),
        }
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
