//! Sessions (docs/architecture/README.md, sections 3, 6 and 9; layer 5).
//!
//! Owns: a [`Session`] holding documents by [`DocId`], each as its format keeps it
//! ([`Format`], [`FormatDoc`]: Word's in `ooxml-session-word`, the kinds Rust does not own
//! yet as packages, [`PackageFormat`]) over one generic history ([`Stored`]); opening, saving
//! and closing; a commit of parts given as bytes for tests ([`Session::update`],
//! section 13); the tool [`Registry`], each tool's spec made once and the kinds it serves;
//! dispatch of a call ([`Session::call`]): arguments taken as meant, the document found,
//! the handler run on a snapshot, the reply as a [`ooxml_tools::ToolResult`]; and a call's
//! [`CallContext`]: who calls, its cancellation token and its deadline, read through the
//! [`ooxml_base::Clock`] and [`ooxml_base::Cancel`] ports of the session's
//! [`ooxml_base::Context`]; and the session's [`MemoryBudget`] (section 6, Memory): what
//! its documents keep that can be made again, within one limit, counted per document.
//!
//! Must not: know any format (each format's crate does: `ooxml-session-word`), hold
//! process-wide state (a session is a value; the Python module holds the
//! sessions), know providers, implement a tool's logic (the `ooxml-tools-*` crates do),
//! read the system clock or the environment (the host gives the ports), or depend on an
//! interpreter lock: every type here is `Send + Sync` and safe from any number of threads.
//!
//! Concurrency (section 6): readers load a document's head snapshot without a lock and
//! never wait for a writer; writers prepare on a snapshot in parallel and commit in the
//! store's short critical section, redone on a conflict. The session's own lock guards
//! only its table of documents, held for a lookup.
#![forbid(unsafe_code)]

mod batch;
pub mod blobs;
mod changes;
mod context;
pub mod datasets;
mod dispatch;
mod document;
mod error;
pub mod facts;
mod format;
mod group;
mod history;
mod ids;
mod memory;
mod model;
mod names;
pub mod outputs;
mod owners;
mod panics;
mod refs;
mod registry;
mod session;
pub mod sources;
mod stored;
mod undo;
mod update;

pub use batch::{ReadBatch, ReadOp, BATCH_TIMEOUT, MAX_READ_BATCH_CHARS, READ_BATCH};
pub use context::{CallCancel, CallContext, CancelToken, LayoutCancel, StoreClock};
pub use dispatch::CallMade;
pub use document::PackageFormat;
pub use error::SessionError;
pub use format::{no_source, AnyVersion, Finished, Format, FormatDoc, FormatRun, OpenEnv};
pub use group::GroupMark;
pub use history::{Direction, NumberedStep, Numbers, Undone};
pub use ids::{DocId, DocKind};
pub use memory::{Account, MemoryBudget};
pub use model::{ModelError, PackageDoc, PartChange, PartContent};
pub use names::DocNames;
pub use ooxml_store::Caller;
pub use owners::{Owners, StepOwner};
pub use refs::{CallRefs, RefMap, RefOwner, RefTable, RefsMark, Resolved, ScopedStep};
pub use registry::{KindRegistrar, Registry};
pub use session::{Opened, Session, SessionConfig, SessionStats};
pub use stored::{Holds, StepBy, Stored, StoredDoc};
pub use undo::{Undo, UNDO};
pub use update::{PartUpdate, Updated};
