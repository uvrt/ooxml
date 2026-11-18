//! Owns: Word comments as docx-agent edits them (`edit/comments.py`): the comment parts by
//! their relationship from the main part (`comments`, `commentsExtended`, `commentsIds`,
//! `commentsExtensible`), deleting a comment with its replies ([`delete`]), and the
//! command resolving, reopening and deleting comments ([`CommentEdits`]).
//!
//! Must not: write a comment part but through a review step's or command's copies.

pub(crate) mod delete;
mod edits;
mod text;

pub use edits::{CommentEdit, CommentEdited, CommentEdits};
pub use text::EditCommentText;

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

/// A comment part docx-agent makes when a document lacks it (`_PARTS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// `commentsExtended`.
    Extended,
}

/// Make the comment part `kind` as docx-agent's `_comment_part(create=True)` does: at its
/// default name (numbered when taken), its root empty, its content type declared, related
/// from `main`; the part's name.
///
/// # Errors
/// When the content types or relationships are not XML.
pub(crate) fn make_part(sink: &mut crate::sink::EditSink<'_, '_>, main: &str, kind: Kind) -> crate::Result<String> {
    let (default, template, root, content_type, rel) = match kind {
        Kind::Extended => (
            "word/commentsExtended.xml",
            "word/commentsExtended{n}.xml",
            crate::new_part::ignorable_root("w15", "commentsEx", W15),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml",
            REL_EXTENDED,
        ),
    };
    sink.new_part(main, (default, template), root, content_type, rel)
}
