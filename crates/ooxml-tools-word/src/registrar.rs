//! Owns: how a Word tool registers itself where it is defined -- the [`Registrar`] port a
//! registry implements (`ooxml-session`'s `Registry`), the handler shapes it takes
//! ([`ReadFn`], [`EditFn`], [`TrackingFn`]), and what a host needs to know to hand a call
//! to the session ([`Hosting`]) -- and [`register`], every Word tool in the order a model
//! is shown them.
//!
//! To add a tool: a `register` function in its handler's module (one `Registrar` call: its
//! arguments' struct, its handler, its [`Hosting`]), called from its group's `register` (or,
//! for a tool outside a group, from [`register`] here). Nothing else lists the tools.
//!
//! Must not: hold the registry (the session does) or know document kinds besides Word's.

use crate::{Editing, Tracked, Version};
use ooxml_base::Cancel;
use ooxml_tools::{Done, ReadsWhen, ToolArgs, ToolError, ToolResult};
use serde_json::{Map, Value};

/// A reading tool's handler: the typed arguments, the version read (its package, name,
/// number and the document's kept readings), the call's cancellation.
pub type ReadFn<T> = fn(&T, &Version<'_>, &dyn Cancel) -> Result<Done<Value>, ToolError>;

/// An editing tool's handler: the typed arguments and the document's editing state.
pub type EditFn<T> = fn(&T, &Editing<'_>) -> ToolResult<Value>;

/// A handler that also leaves the document's tracking mode (`word_set_tracking`): the reply,
/// and the mode the session keeps from now on (`None`: unchanged).
pub type TrackingFn<T> = fn(&T, &Editing<'_>) -> (ToolResult<Value>, Option<Tracked>);

/// What a host (docx-agent) needs to know of an editing tool to hand its calls to the
/// session that owns the document: exported with the registry (`ooxml_core.tool_hosting`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // independent facts, each exported by name
pub struct Hosting {
    /// The host asks the session first for the tool's edits.
    pub served: bool,
    /// Its edits read the session's blobs (an inserted picture's bytes).
    pub blobs: bool,
    /// Its edits read the document's pages: the host hands the session its page engine.
    pub layout: bool,
    /// It keeps the values the host wrote for references on the paragraphs they went into.
    pub keeps_written: bool,
}

impl Hosting {
    /// The host answers the tool's calls itself.
    pub const HOST: Self = Self {
        served: false,
        blobs: false,
        layout: false,
        keeps_written: false,
    };

    /// The host asks the session first.
    pub const SERVED: Self = Self {
        served: true,
        ..Self::HOST
    };

    /// Its edits read the session's blobs.
    #[must_use]
    pub const fn blobs(self) -> Self {
        Self { blobs: true, ..self }
    }

    /// Its edits read the document's pages.
    #[must_use]
    pub const fn layout(self) -> Self {
        Self { layout: true, ..self }
    }

    /// It keeps the values written by reference.
    #[must_use]
    pub const fn keeps_written(self) -> Self {
        Self {
            keeps_written: true,
            ..self
        }
    }

    /// The export: each fact by name.
    #[must_use]
    pub fn export(self) -> Value {
        let mut out = Map::new();
        out.insert("served".into(), Value::Bool(self.served));
        out.insert("blobs".into(), Value::Bool(self.blobs));
        out.insert("layout".into(), Value::Bool(self.layout));
        out.insert("keeps_written".into(), Value::Bool(self.keeps_written));
        Value::Object(out)
    }
}

/// Where Word's tools register: a registry of the tools a session dispatches.
pub trait Registrar {
    /// The reading tool `T`, answered by `run`.
    fn read<T: ToolArgs + 'static>(&mut self, run: ReadFn<T>);

    /// The editing tool `T`, answered by `run`, hosted as `hosting` says.
    fn edit<T: ToolArgs + 'static>(&mut self, run: EditFn<T>, hosting: Hosting);

    /// The editing tool `T` that also sets the document's tracking mode.
    fn tracking<T: ToolArgs + 'static>(&mut self, run: TrackingFn<T>, hosting: Hosting);

    /// The editing tool `T` some of whose calls only read ([`ReadsWhen`]: `edit_chart`'s
    /// `action: "read"`): `read` answers those on the version the call pins, `edit` the
    /// others, hosted as `hosting` says.
    fn reading_edit<T: ToolArgs + ReadsWhen + 'static>(&mut self, read: ReadFn<T>, edit: EditFn<T>, hosting: Hosting);
}

/// Every Word tool, in the order a model is shown them: `describe` first, then the other
/// reading tools outside a group, then each group's, then `check`.
pub fn register(r: &mut impl Registrar) {
    crate::describe::register(r);
    crate::word_read::register(r);
    crate::word_inspect::register(r);
    crate::find_text::register(r);
    crate::review::register(r);
    crate::text::register(r);
    crate::objects::register(r);
    crate::style::register(r);
    crate::structure::register(r);
    crate::check::register(r);
}
