//! The Markdown layer's read half (docs/architecture/README.md, section 3: `ooxml-word`
//! owns "Markdown in and out"; phase P1): a version of a document as CommonMark plus GFM
//! tables, strikethrough and footnotes, each block carrying its id in an HTML comment --
//! docx-agent's `to_markdown` (`docx_agent/markdown/`), text for text.
//!
//! Owns: the reading of a story into records ([`read`]), the neutral Markdown model
//! ([`model`]) and its rendering as text ([`render`]); the ranges a read takes
//! ([`select`]).
//!
//! Must not: write to the document (reading never stamps an id), or lay out.

pub mod html;
pub mod model;
pub mod read;
pub mod render;
pub mod select;
pub mod stylemap;

use crate::address::Address;
use crate::docx::Docx;
use crate::view::View;
pub use select::RangeError;
use std::sync::Arc;

/// Which stories a read covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stories {
    /// The body.
    Body,
    /// The body, then every header and footer with content, then every note.
    All,
}

/// The document (or `range` of it) as plain Markdown, without the id comments (docx-agent's
/// `to_markdown(..., ids=False)`), in the final view of the body: what an edit's reply
/// shows of what it made. Not kept: a reply reads it once.
///
/// # Errors
/// A [`RangeError`] when `range` names nothing or is malformed.
pub fn to_plain_markdown(docx: &Docx, range: Option<&Address>) -> Result<String, RangeError> {
    let blocks = read::project(docx, range, View::Final, Stories::Body, false)?;
    Ok(render::render(&blocks))
}

/// The document (or `range` of it) as Markdown, kept with the version for the calls after.
///
/// # Errors
/// A [`RangeError`] when `range` names nothing or is malformed.
pub fn to_markdown(
    docx: &Docx,
    range: Option<&Address>,
    view: View,
    stories: Stories,
) -> Result<Arc<String>, RangeError> {
    let key = (range.map(|r| r.as_str().to_owned()), view, stories);
    docx.memo().markdown(key, || {
        let blocks = read::project(docx, range, view, stories, true)?;
        Ok(render::render(&blocks))
    })
}
