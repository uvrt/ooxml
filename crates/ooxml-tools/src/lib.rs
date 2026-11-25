//! The tool contract (docs/architecture/README.md, sections 9 and 10; layer 4).
//!
//! Owns: a tool's definition from one Rust struct (`#[derive(ToolArgs)]`, [`ToolSpec`]):
//! the canonical strict-subset schema and the validation schema, as today's Python
//! registry writes them byte for byte; the intent rules declared once on the fields
//! (`empty_is_absent`, `single_as_list`, `map_as_pairs`, value and argument aliases,
//! defaults of required arguments, kind-neutral fields) and applied in today's order
//! ([`ToolSpec::accept`], [`ToolSpec::ignored_for`]); validation with today's messages; the
//! result envelope within its reply budget ([`ToolResult`]); errors whose refusals cannot
//! be made without a way out ([`ToolError`], [`WayOut`]); and the cost of the descriptions
//! against budgets ([`cost`]); and the handler side (design phase 3, section 3): how a
//! format's tools register ([`Registrar`] over its [`FormatTypes`]), the handler shapes, and
//! their erasure to a [`Tool`] run in a [`CallEnv`].
//!
//! Must not: know providers (the Python adapters read the exported specs), document
//! formats (the `ooxml-tools-*` crates hold the tools), sessions or dispatch (lane P0d),
//! or hold global state. Nothing here depends on Python or its interpreter lock.
//!
//! ```
//! use ooxml_tools::{ArgEnum, ToolArgs};
//!
//! /// Which matches.
//! #[derive(ArgEnum, Clone, Copy, Debug, PartialEq)]
//! pub enum Expect { One, All }
//!
//! #[derive(ToolArgs, Debug)]
//! #[tool(name = "toy_replace", description = "Replace text.", mutates)]
//! pub struct Replace {
//!     /// Document id.
//!     pub doc: String,
//!     /// Matches expected.
//!     #[arg(default = "one")]
//!     pub expect: Expect,
//!     /// Where to look.
//!     #[arg(single_as_list, alias = "within")]
//!     pub targets: Option<Vec<String>>,
//! }
//!
//! let (call, notes) = Replace::accept(serde_json::json!({"doc": "d1", "within": "p:A"}), None).unwrap();
//! assert_eq!(call.expect, Expect::One);
//! assert_eq!(call.targets, Some(vec!["p:A".to_owned()]));
//! assert_eq!(notes, ["within read as targets", "expect not given: read as 'one'"]);
//! ```
#![forbid(unsafe_code)]

mod check;
pub mod cost;
mod dates;
mod difflib;
pub mod documents;
mod error;
mod handlers;
mod intent;
mod limits;
mod not_supported;
mod places;
pub mod pyfmt;
pub mod pyre;
pub mod refs;
mod result;
pub mod schema;
pub mod sources;
mod spec;
#[doc(hidden)]
pub mod support;
pub mod tiles;
mod types;
mod validate;

pub use error::{ErrorCode, Failure, Refusal, ToolError, TryCall, WayOut};
pub use handlers::{
    CallEnv, EditFn, EditHandler, EditRun, FormatCall, FormatTypes, ReadFn, ReadHandler, ReadRun, ReadingEditHandler,
    Registrar, Run, Tool, TrackingFn, TrackingRun,
};
pub use intent::Accepted;
pub use limits::{BudgetSpan, ImageBudget, Limits};
pub use not_supported::NOT_SUPPORTED;
pub use ooxml_base::pyfmt::PyNumber;
pub use ooxml_tools_derive::{ArgEnum, ToolArgs};
pub use places::{PlaceKind, Places, NAMES as PLACE_NAMES};
pub use result::{
    compact, python_dumps, AppOnly, Done, File, Outcome, ReplyData, Timing, ToolResult, MAX_RESULT_CHARS,
};
pub use spec::{Alias, Flags, Holding, ToolSpec};
pub use types::{ArgEnum, ArgError, ArgType, IntOrStr, NumOrStr, ReadsWhen, ToolArgs};
pub use validate::{validate, CallError, Problem, Retry};

#[doc(hidden)]
pub use serde_json;
