//! Owns: [`Env`], what reading a document collects across its blocks (markdown-it's `env`):
//! link reference definitions by normalised label, and the footnotes (labels defined,
//! footnotes referenced in order, inline footnotes' tokens).
//!
//! Must not: parse.

use crate::token::Token;
use std::collections::HashMap;

/// A link reference definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// The destination, normalised.
    pub href: String,
    /// The title.
    pub title: String,
}

/// A footnote as the references make it (markdown-it-footnote's list entry).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Footnote {
    /// Its label (`[^label]`); none for an inline footnote.
    pub label: Option<String>,
    /// How many references point at it.
    pub count: usize,
    /// An inline footnote's text and its tokens.
    pub inline: Option<(String, Vec<Token>)>,
}

/// What a read collects.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// Reference definitions by normalised label, the first of each label.
    pub references: HashMap<String, Reference>,
    /// Footnote labels defined, each with its number once referenced.
    pub footnote_refs: HashMap<String, Option<usize>>,
    /// Whether a footnote definition or inline footnote was read (markdown-it's
    /// `env["footnotes"]` exists).
    pub footnotes_seen: bool,
    /// The footnotes, by number.
    pub footnotes: Vec<Footnote>,
}
