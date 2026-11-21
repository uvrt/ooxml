//! Sessions (docs/architecture/README.md, sections 3, 6 and 9; layer 5).
//!
//! Owns: a [`Session`] holding documents by [`DocId`], each one's versions in an
//! `ooxml-store` [`ooxml_store::Store`] over its package ([`PackageDoc`]); opening, saving
//! and closing; the bridge's commit of a version's touched parts ([`Session::update`],
//! section 13); the tool [`Registry`], each tool's spec made once and the kinds it serves;
//! dispatch of a call ([`Session::call`]): arguments taken as meant, the document found,
//! the handler run on a snapshot, the reply as a [`ooxml_tools::ToolResult`]; and a call's
//! [`CallContext`]: who calls, its cancellation token and its deadline, read through the
//! [`ooxml_base::Clock`] and [`ooxml_base::Cancel`] ports of the session's
//! [`ooxml_base::Context`]; and the session's [`MemoryBudget`] (section 6, Memory): what
//! its documents keep that can be made again, within one limit, counted per document.
//!
//! Must not: hold process-wide state (a session is a value; the Python module holds the
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
mod call;
mod changes;
mod context;
mod document;
mod error;
mod group;
mod handlers;
mod history;
mod ids;
mod memory;
mod model;
mod names;
mod reading_edit;
mod records;
mod refused;
mod registry;
mod retired;
mod rows_written;
mod session;
mod update;

pub use batch::{ReadBatch, ReadOp, BATCH_TIMEOUT, MAX_READ_BATCH_CHARS, READ_BATCH};
pub use call::Call;
pub use changes::{ChangedPart, IdsSince};
pub use context::{CallContext, CancelToken};
pub use error::SessionError;
pub use group::GroupMark;
pub use handlers::{EditHandler, ReadHandler, Tool};
pub use history::{NumberedStep, Numbers, Undone};
pub use ids::{DocId, DocKind};
pub use memory::{Account, MemoryBudget};
pub use model::{ModelError, PackageDoc, PartChange, PartContent};
pub use names::DocNames;
pub use ooxml_store::Caller;
pub use ooxml_tools_word::{EditFn, Hosting, ReadFn, TrackingFn};
pub use ooxml_word_model::ids::HeldId;
pub use reading_edit::ReadingEditHandler;
pub use registry::Registry;
pub use retired::RetiredBy;
pub use session::{CallMade, Opened, Session, SessionConfig, SessionStats};
pub use update::{HostIds, PartUpdate, Updated};
