//! Owns: nothing of its own -- a Word document's problems in the session's one shape are
//! `ooxml_tools_word::problems`' (its tools say them so too), re-exported here for the
//! session's Word format.

pub use ooxml_tools_word::problems::{listed, problem, structured};
