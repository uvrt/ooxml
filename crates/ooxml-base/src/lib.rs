//! The bottom of the crate graph (docs/architecture/README.md, section 3, layer 0): what every
//! other crate shares and nothing else.
//!
//! Owns: units and exact arithmetic ([`units`]), content digests ([`digest`]), the limits on
//! untrusted input and the locale as values ([`limits`], [`locale`]), the ports the core is
//! given its I/O through ([`context`]), and the error type for all of these ([`BaseError`]).
//!
//! Must not: do I/O, hold global state, or know about XML, ZIP or any OOXML format. Adapters
//! of the ports (file-system fonts, the system clock, ...) live in leaf crates.
#![forbid(unsafe_code)]

pub mod context;
pub mod digest;
mod error;
pub mod limits;
pub mod locale;
pub mod units;

pub use context::{CacheBudget, Cancel, Clock, Context, FontSource, PartStore};
pub use digest::{Digest, DigestKey, Digester};
pub use error::BaseError;
pub use limits::{Limits, MetafileLimits, XmlLimits, ZipLimits};
pub use locale::Locale;
pub use units::{Emu, HalfPoints, Pt4096, Twips, Q};
