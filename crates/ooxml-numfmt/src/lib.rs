//! Number formats, dates and locales (docs/architecture/README.md, section 3
//! `ooxml-numfmt`; layer 1), equal to ooxml-common's `numfmt.py`, `dates.py` and
//! `locales.py` as far as charts use them (phase P3, lane P3b2).
//!
//! Owns: a format code's sections and tokens and what a section shows ([`code`]); the
//! locale a number is written for ([`locale`]); a date serial written through a date code,
//! with each language's month and day names ([`dates`]).
//!
//! Must not: read a workbook or a document, or hold state: every value is immutable and
//! `Send + Sync`, and nothing here depends on an interpreter lock.
//!
//! Not here yet: the rest of `numfmt.MEASURED`'s formatting (fractions, elapsed time, colours
//! and conditions) -- a chart writes numbers with its own rules (`ooxml-chart`), and the
//! sheets phase (P6) brings Excel's.
#![forbid(unsafe_code)]

pub mod code;
pub mod dates;
pub mod locale;

pub use code::{classify, is_date_code, sections, tokens, Classification, Kind, Token, TokenKind};
pub use dates::{date_of_serial, format_serial, Date};
pub use locale::{canonical_language, Locale, LocaleError, EN_US};
