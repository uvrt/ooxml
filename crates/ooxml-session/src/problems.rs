//! Owns: the one shape of a document's problems in every reply (user decision 2026-11-25,
//! structured problems), the session's: `{code, part, element, message, severity}`
//! ([`Problem`]) and a version's problems against those its document opened with
//! ([`Validation`]), defined in `ooxml-tools` (`ooxml_tools::problems`) so that a format's
//! tools say them in it too.
//!
//! Must not: find problems (each format validates its own documents) or word a format's
//! problem (its format gives the message).

pub use ooxml_tools::problems::{Problem, Severity, Validation};
