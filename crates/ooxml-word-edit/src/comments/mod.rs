//! Owns: Word comments as docx-agent edits them (`edit/comments.py`): the comment parts by
//! their relationship from the main part (`comments`, `commentsExtended`, `commentsIds`,
//! `commentsExtensible`), deleting a comment with its replies ([`delete`]), and the
//! command resolving, reopening and deleting comments ([`CommentEdits`]).
//!
//! Must not: write a comment part but through a review step's or command's copies.

pub(crate) mod delete;
mod edits;

pub use edits::{CommentEdit, CommentEdited, CommentEdits};

/// `commentsExtended`'s namespace (`w15`).
pub(crate) const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";
/// `commentsIds`' namespace.
pub(crate) const W16CID: &str = "http://schemas.microsoft.com/office/word/2016/wordml/cid";
/// `commentsExtensible`'s namespace.
pub(crate) const W16CEX: &str = "http://schemas.microsoft.com/office/word/2018/wordml/cex";

/// The comments part's relationship type.
pub(crate) const REL_COMMENTS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
/// `commentsExtended`'s.
pub(crate) const REL_EXTENDED: &str = "http://schemas.microsoft.com/office/2011/relationships/commentsExtended";
/// `commentsIds`'.
pub(crate) const REL_IDS: &str = "http://schemas.microsoft.com/office/2016/09/relationships/commentsIds";
/// `commentsExtensible`'s.
pub(crate) const REL_EXTENSIBLE: &str = "http://schemas.microsoft.com/office/2018/08/relationships/commentsExtensible";

/// The first part `main` relates to by `rel_type` that the package has.
pub(crate) fn part_of(package: &ooxml_opc::Package, main: &str, rel_type: &str) -> Option<String> {
    ooxml_word_model::parts::related_parts(package, main, rel_type)
        .ok()?
        .into_iter()
        .next()
        .filter(|p| package.has_part(p))
}
