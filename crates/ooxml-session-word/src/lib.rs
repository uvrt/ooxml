//! Word's format of the session (design phase 3, section 3; layer 5.1).
//!
//! Owns: [`WordFormat`], how a session opens Word documents and their tools; one open Word
//! document as the session holds it ([`Word`]: its versions over the edit core, the version
//! it opened as, its tracking mode, retired ids, rows written, readings, page engine and
//! live views); a tool call on it ([`Call`], what the host hands it beside: [`WordHost`]) and
//! what a changing call made ([`WordMade`]); the save gate's comparison and the host's reads
//! ([`WordSession`], [`HostReads`], [`Records`], [`ValuesOf`], [`BlocksOf`]); the session's
//! page engines ([`WordPages`], [`SessionPages`]) and where blocks are laid out
//! ([`BlockPagesOf`]); an application's live view ([`LiveView`]); a document made from a
//! template ([`FromTemplate`]); and the records of retired ids ([`RetiredIds`]) and rows
//! written ([`RowsWrittenOf`]). Each reaches a session's documents through
//! `ooxml-session`'s [`ooxml_session::FormatDoc`], as extension traits on
//! [`ooxml_session::Session`] and [`ooxml_session::Opened`] ([`prelude`]).
//!
//! Must not: hold a session's table of documents or dispatch a call (`ooxml-session`
//! does), implement a tool's logic (`ooxml-tools-word` does), or depend on `ooxml-host`.
#![forbid(unsafe_code)]

mod block_pages;
mod blocks_of;
mod call;
mod from_template;
mod host_reads;
mod live_view;
mod pages;
mod records;
mod reflowed;
mod refused;
mod retired;
mod rows_written;
mod session;
mod values;
mod version;
mod word;

pub use block_pages::{BlockAt, BlockPages, BlockPagesOf};
pub use blocks_of::{BlockFound, BlockRefused, BlocksOf};
pub use call::{Call, WordHost};
pub use from_template::FromTemplate;
pub use host_reads::{HostReads, IdStateOf};
pub use live_view::{LiveView, PageSvg, TextMode, View, ViewError, ViewPage, VIEW_TOKENS};
pub use ooxml_word_model::ids::{HeldId, IdStateError};
pub use pages::{SessionPages, WordPages};
pub use records::Records;
pub use retired::{RetiredBy, RetiredIds};
pub use rows_written::RowsWrittenOf;
pub use session::{not_word, refuses_state, WordSession};
pub use values::ValuesOf;
pub use version::WordVersion;
pub use word::{Word, WordFormat, WordMade};

use ooxml_session::{Format, PackageFormat, Registry};
use std::sync::Arc;

/// The extension traits, for `use ooxml_session_word::prelude::*`.
pub mod prelude {
    pub use crate::{
        BlockPagesOf, BlocksOf, FromTemplate, HostReads, LiveView, Records, RetiredIds, RowsWrittenOf, ValuesOf,
        WordPages, WordSession, WordVersion,
    };
}

/// The formats a session holds today: Word's (laid out with `pages` where given), and the
/// kinds Rust does not own yet as packages (`pptx`, `xlsx`).
#[must_use]
pub fn formats(pages: Option<SessionPages>) -> Vec<Arc<dyn Format>> {
    vec![
        Arc::new(WordFormat { pages }),
        Arc::new(PackageFormat(ooxml_session::DocKind::Pptx)),
        Arc::new(PackageFormat(ooxml_session::DocKind::Xlsx)),
    ]
}

/// The tools of [`formats`]: Word's, then `undo` and `read_batch` (the export's order,
/// `describe` first).
#[must_use]
pub fn registry() -> Registry {
    Registry::of(&formats(None))
}
