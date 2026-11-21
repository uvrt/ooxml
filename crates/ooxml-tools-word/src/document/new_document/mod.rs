//! `new_document`'s package: a new blank document's bytes ([`Blank`]), which docx-agent's
//! `new_document` host tool opens in its session. Not a registered tool: the call makes a
//! document, it reads none.

mod blank;
mod word_new;

pub use blank::{Blank, PageDefaults, PAGE_SIZES};
