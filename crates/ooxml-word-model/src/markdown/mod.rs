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

use crate::docx::Docx;
pub use select::RangeError;
use std::sync::Arc;

/// The view of the revisions a read shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewName {
    /// Changes accepted.
    Final,
    /// Changes, comments and fields in place (CriticMarkup).
    Markup,
    /// Changes rejected.
    Original,
}

/// Which stories a read covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stories {
    /// The body.
    Body,
    /// The body, then every header and footer with content, then every note.
    All,
}

/// The document (or `range` of it) as Markdown, kept with the version for the calls after.
///
/// # Errors
/// A [`RangeError`] when `range` names nothing or is malformed.
pub fn to_markdown(
    docx: &Docx,
    range: Option<&str>,
    view: ViewName,
    stories: Stories,
) -> Result<Arc<String>, RangeError> {
    let key = (range.map(str::to_owned), view, stories);
    docx.memo().markdown(key, || {
        let blocks = read::project(docx, range, view, stories)?;
        Ok(render::render(&blocks))
    })
}
