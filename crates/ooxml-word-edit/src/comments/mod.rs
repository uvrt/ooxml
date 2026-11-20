//! Owns: Word comments as docx-agent edits them (`edit/comments.py`): the comment parts by
//! their relationship from the main part (`comments`, `commentsExtended`, `commentsIds`,
//! `commentsExtensible`), deleting a comment with its replies ([`delete`]), and the
//! command resolving, reopening and deleting comments ([`CommentEdits`]); comments added
//! and replied to ([`AddComments`]), written with their entries in the modern parts
//! ([`write`]).
//!
//! Must not: write a comment part but through a review step's or command's copies.

mod add;
mod anchors;
mod control;
pub(crate) mod delete;
mod edits;
mod reply;
mod size;
mod text;
mod write;

pub use add::{AddComments, CommentMade, CommentWrite, Who};
pub use edits::{CommentEdit, CommentEdited, CommentEdits};
pub use text::EditCommentText;
pub(crate) use write::{append, complete, last_para};

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
    /// `comments`.
    Comments,
    /// `commentsExtended`.
    Extended,
    /// `commentsIds`.
    Ids,
    /// `commentsExtensible`.
    Extensible,
}

/// The content type of the comment parts, less the part's own name and `+xml`.
const WML: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml";

/// Make the comment part `kind` as docx-agent's `_comment_part(create=True)` does: at its
/// default name (numbered when taken), its root empty, its content type declared, related
/// from `main`; the part's name.
///
/// # Errors
/// When the content types or relationships are not XML.
pub(crate) fn make_part(sink: &mut crate::sink::EditSink<'_, '_>, main: &str, kind: Kind) -> crate::Result<String> {
    let (name, root, rel) = match kind {
        Kind::Comments => ("comments", comments_root(), REL_COMMENTS),
        Kind::Extended => (
            "commentsExtended",
            crate::new_part::ignorable_root("w15", "commentsEx", W15),
            REL_EXTENDED,
        ),
        Kind::Ids => (
            "commentsIds",
            crate::new_part::ignorable_root("w16cid", "commentsIds", W16CID),
            REL_IDS,
        ),
        Kind::Extensible => (
            "commentsExtensible",
            crate::new_part::ignorable_root("w16cex", "commentsExtensible", W16CEX),
            REL_EXTENSIBLE,
        ),
    };
    let default = format!("word/{name}.xml");
    let template = format!("word/{name}{{n}}.xml");
    let content_type = format!("{WML}.{name}+xml");
    sink.new_part(main, (&default, &template), root, &content_type, rel)
}

/// The comment part `kind` `main` relates to, made when the package lacks it.
///
/// # Errors
/// As [`make_part`].
pub(crate) fn ensure_part(sink: &mut crate::sink::EditSink<'_, '_>, main: &str, kind: Kind) -> crate::Result<String> {
    let rel = match kind {
        Kind::Comments => REL_COMMENTS,
        Kind::Extended => REL_EXTENDED,
        Kind::Ids => REL_IDS,
        Kind::Extensible => REL_EXTENSIBLE,
    };
    match part_of(sink.doc().package(), main, rel) {
        Some(part) => Ok(part),
        None => make_part(sink, main, kind),
    }
}

/// `<w:comments xmlns:w=... xmlns:w14=... xmlns:mc=... mc:Ignorable="w14"/>`, the root
/// docx-agent makes the comments part with.
fn comments_root() -> ooxml_xml::Element {
    use ooxml_xml::{Attribute, Element, NamespaceDecl, QName};
    use std::sync::Arc;
    let ns = |prefix: &str, uri: &str| NamespaceDecl {
        prefix: Some(Arc::from(prefix)),
        uri: Arc::from(uri),
    };
    Element::new(
        Arc::new(QName::new(Some("w"), "comments", Some(crate::names::W))),
        vec![
            ns("w", crate::names::W),
            ns("w14", crate::names::W14),
            ns("mc", crate::names::MC),
        ],
        vec![Attribute {
            name: Arc::new(QName::new(Some("mc"), "Ignorable", Some(crate::names::MC))),
            value: Arc::from("w14"),
        }],
        Vec::new(),
    )
}
