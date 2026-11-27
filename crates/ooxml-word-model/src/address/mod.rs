//! Word's one address resolver (docs/architecture/README.md, section 4 "Ids": one address
//! resolver per format): every form docx-agent's reading tools take -- a story, a block
//! (`p:`, `t:`, `header1/p:...`), a span (`p:A..t:B`, either end open), a heading's range
//! (`h:p:A`), a row (`tr:`), a cell (`t:X/c1,1`), a section
//! (`s:2`, `s:<paraId>`, `s:body`), a bookmark (`bm:Name` or the plain name), a content
//! control (`cc:`, `cc@`) and a text range (`p:A@4:11`, `p:A@5`, `p:A@3..p:B@8`) -- each a
//! pure function of a version to what it names, or to a refusal carrying the ids that
//! would do. Names by place this library gave before every block had an id of its own
//! (`p@body/7`, `t:X/r1`, `p:A#1`, `s@body/3`) are refused with guidance
//! ([`resolve::positional`]).
//!
//! Owns: the syntax ([`syntax`]), an address read once at a tool's edge ([`Address`],
//! [`Place`]), the resolution ([`mod@resolve`]) and [`AddressError`], the
//! refusal every tool taking an address gives in the same words.
//!
//! Must not: change the document, read Markdown records, or follow renamed ids (the Rust
//! session keeps no aliases until it owns the document, phase P2).

pub mod marks;
pub mod parsed;
pub mod resolve;
pub mod syntax;

pub use parsed::{Address, Form, Place, PlaceKind, Shape};
pub use resolve::{
    bookmark, bookmark_range, direct, expand_heading, heading, in_table, paragraph, positional, resolve,
    scope_paragraphs, section, text_range, unknown,
};
pub use syntax::{inside_table, is_text_range, parse_range, span_ends, RangeText, SPAN_SYNTAX};

/// Why an address was refused: its code, message and the addresses that would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressError {
    /// `not_found`, `invalid_arguments`, `refused` or `unit`, as docx-agent's tools map
    /// its `KeyError`, `AddressError`, `EditError` and `ValueError`.
    pub code: &'static str,
    /// What is wrong, and what to pass.
    pub message: String,
    /// Addresses that would do.
    pub options: Vec<String>,
}

impl AddressError {
    /// An address that names nothing.
    #[must_use]
    pub fn not_found(message: String, options: Vec<String>) -> AddressError {
        AddressError {
            code: "not_found",
            message,
            options,
        }
    }

    /// A malformed address, or one of a kind the parameter does not take.
    #[must_use]
    pub fn invalid(message: String, options: Vec<String>) -> AddressError {
        AddressError {
            code: "invalid_arguments",
            message,
            options,
        }
    }

    /// An address that cannot be taken as given (a reversed span, a span across containers).
    #[must_use]
    pub fn refused(message: String) -> AddressError {
        AddressError {
            code: "refused",
            message,
            options: Vec::new(),
        }
    }

    /// A value out of what it may be (a range across stories).
    #[must_use]
    pub fn unit(message: String) -> AddressError {
        AddressError {
            code: "unit",
            message,
            options: Vec::new(),
        }
    }
}

impl std::fmt::Display for AddressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AddressError {}
