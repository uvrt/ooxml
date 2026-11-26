//! Owns: a new Word document's bytes, as `new_document` makes one (docx-agent's
//! `new_document`; Word's [`ooxml_session::Format::new_document`]): blank
//! ([`ooxml_tools_word::new_document::Blank`]: the page, the session's locale's language and
//! separators -- en-US's without one --, the properties and the call's date), or from the
//! template a blob holds ([`from_template`]: the template opened from its bytes, made a new
//! document by the edit core ([`NewFromTemplate`]: its content left out) and written as a
//! `.docx`); with docx-agent's refusals of a size that is no page size and of a blob that is
//! no Word document or template.
//!
//! Must not: open the new document in the session (the host opens the bytes), or change
//! the bytes it is given.

use crate::word::ports;
use ooxml_base::pyfmt::repr_str;
use ooxml_opc::Package;
use ooxml_session::{NewDocument, OpenEnv, SessionError};
use ooxml_store::Caller;
use ooxml_tools::{Refusal, ToolError, WayOut};
use ooxml_tools_word::new_document::{Blank, PAGE_SIZES};
use ooxml_word_edit::{serialise, EditError, Editor, FileKind, NewFromTemplate};
use std::sync::Arc;

/// The text's language of a new document made without a locale.
const FALLBACK_LANGUAGE: &str = "en-US";

/// A new Word document's bytes as `made` asks (Word's `new_document`).
///
/// # Errors
/// `invalid_arguments` at `size` for a size that is no page size, at `template_blob` for a
/// template that is no Word document or template; `internal` where the blank document
/// cannot be written.
pub fn new_document(made: &NewDocument<'_>, env: &OpenEnv<'_>) -> Result<Vec<u8>, ToolError> {
    let page = match made.size {
        Some(size) => Some(PAGE_SIZES.iter().find(|(name, ..)| *name == size).ok_or_else(|| {
            let names = PAGE_SIZES.iter().map(|(name, ..)| *name);
            ToolError::refusal(
                Refusal::InvalidArguments,
                format!("a document's size is a page size, not {}", repr_str(size)),
                WayOut::names(names).unwrap_or_else(|| WayOut::call(made.retry.clone())),
            )
            .at("size")
        })?),
        None => None,
    };
    let owned = |text: Option<&str>| text.map(str::to_owned);
    if let Some(template) = made.template {
        let from = NewFromTemplate {
            page: page.map(|(_, width, height)| (*width, *height)),
            title: owned(made.title),
            author: owned(made.author),
            created: made.created.to_owned(),
        };
        return from_template(Arc::from(template), &from, env).map_err(|error| {
            ToolError::refusal(
                Refusal::InvalidArguments,
                format!("the blob is not a Word document or template: {error}"),
                WayOut::call(made.retry.clone()),
            )
            .at("template_blob")
        });
    }
    let locale = made.locale.unwrap_or(&ooxml_numfmt::EN_US);
    let blank = Blank {
        page: page.map_or("A4", |(name, ..)| name).to_owned(),
        landscape: false,
        language: made
            .locale
            .map_or_else(|| FALLBACK_LANGUAGE.to_owned(), |l| l.language.to_string()),
        separators: (locale.decimal_separator.to_string(), locale.list_separator.to_string()),
        title: owned(made.title),
        author: owned(made.author),
        created: made.created.to_owned(),
    };
    blank
        .bytes()
        .map_err(|error| ToolError::internal(error, "new_document:blank"))
}

/// A new document's bytes from `template` (a `.dotx`, `.dotm`, `.docx` or `.docm`'s), as
/// `made` says: its content left out, its properties started again, a `.docx`; opened with
/// `env`'s limits, store and ports.
///
/// # Errors
/// [`SessionError::Package`] for bytes that are not a package within the limits;
/// [`SessionError::Format`] for a package that is not a Word document or template.
pub fn from_template(template: Arc<[u8]>, made: &NewFromTemplate, env: &OpenEnv<'_>) -> Result<Vec<u8>, SessionError> {
    let package = Package::open(template, &env.context.limits)?;
    if FileKind::of(&package).map_err(SessionError::format)?.is_none() {
        return Err(SessionError::format(EditError::Refused(
            "not a Word document or template".to_owned(),
        )));
    }
    let (clock, budget) = ports(env.context);
    let editor = Editor::open(package, env.store, clock, budget).map_err(SessionError::format)?;
    editor.run(&Caller::anonymous(), made).map_err(SessionError::format)?;
    let head = editor.store().snapshot();
    serialise(head.doc().package(), FileKind::Docx, None).map_err(SessionError::format)
}
