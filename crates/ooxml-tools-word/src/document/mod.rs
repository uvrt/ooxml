//! The document's own tools (docx-agent's handlers of the shared tools in
//! `tools/shared.py` that act on the document as a whole): `set_properties`, `save_document`,
//! `render`; and [`new_document`], the package of a new document, which the host opens.
//!
//! Owns: the tools' definitions (descriptions beside them) and their handlers, each a
//! function of the typed arguments and an [`crate::Editing`] whose edit runs as one command
//! through the editor (one history step).
//!
//! Must not: hold sessions or documents by id, keep state between calls, or write the
//! document but through a command.

pub mod new_document;
mod render;
mod save_document;
mod set_properties;

pub use ooxml_word_pages::png::PageRaster;
pub use render::{
    laid_to_draw, page_key, render, sheet_key, Drawing, ImageKey, PageImages, Render, RenderSheet, DEFAULT_WIDTH,
    MAX_IMAGES, RENDER_NOT_OWNED, RENDER_NO_LAYOUT,
};
pub use save_document::{save_document, SaveDocument, SaveFormat};
pub use set_properties::{set_properties, SetProperties};

/// Registers the group's tools, in the order a model is shown them (the crate's
/// [`crate::register`]).
pub(crate) fn register(r: &mut impl ooxml_tools::Registrar<crate::Word>) {
    set_properties::register(r);
    save_document::register(r);
    render::register(r);
}
