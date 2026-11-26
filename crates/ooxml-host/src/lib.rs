//! The Rust host (design phase 3, sections 2, 5 and 6; layer 5.1): one [`Host`] per toolbox
//! and one [`HostSession`] per application session, running each call from the name a model
//! wrote to the reply it reads.
//!
//! Owns: the host's tools ([`Catalogue`]: the formats' and its own, what it holds of each
//! beside its spec, the formats' hints and withdrawn tools) and how a name is read
//! ([`hints`]); the roles a call needs and the refusal of one outside them; the call's
//! pipeline ([`HostSession::call`]: one acceptance, the document and its refs, a retry key's
//! repeat, the handler run by the document's format on the version the call reads, the
//! effects applied once its change is published, the checks, the reply's warnings in today's
//! order); the session's stores behind one lock ([`Stores`]); the call's record
//! ([`CallRecord`]) and the retry keys ([`RetryKeys`]); and the seam the session tools run
//! through ([`SessionTools`]).
//!
//! Must not: know a format (each comes as an [`ooxml_session::Format`]), hand a handler a
//! document's head (the format runs it on the version the call reads), hold the stores'
//! lock while a handler runs, or depend on `ooxml-session-word` (a sibling at 5.1).
#![forbid(unsafe_code)]

mod catalogue;
pub mod hints;
mod own;
mod pipeline;
mod records;
mod session;

pub use catalogue::{Catalogue, Entry, Own};
pub use own::{Batch, CloseDocument, NewDocument, NewKind, NotYet, OpenDocument, OwnCall, ReadBlob, SessionTools};
pub use pipeline::{caller_key, NotRun};
pub use records::{digest, shape, CallRecord, RetryKeys};
pub use session::{CallAs, Host, HostSession, HostSettings, Stores};
