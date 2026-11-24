//! An inline content control a review empties shows its placeholder, as Word 16.106's
//! Accept All and Reject All leave one: `w:showingPlcHdr` set and, the control naming no
//! placeholder text, five spaces as its content (`controls.docx`'s control 309, whose
//! content is one tracked insertion, after Word's Reject All).
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use ooxml_store::Caller;
use ooxml_word_edit::{Review, Selection};

/// Control 309's `w:sdt` after a review of everything.
fn control_after(accept: bool) -> String {
    let editor = editor(&fixture("generated/controls.docx"));
    let review = Review {
        accept,
        selection: Selection::All,
    };
    editor.run(&Caller::anonymous(), &review).unwrap();
    let head = editor.store().snapshot();
    let body = String::from_utf8(head.doc().tree("word/document.xml").unwrap().unwrap().write()).unwrap();
    let at = body.find(r#"<w:id w:val="309"/>"#).unwrap();
    let start = body[..at].rfind("<w:sdt>").unwrap();
    let end = at + body[at..].find("</w:sdt>").unwrap();
    body[start..end].to_owned()
}

#[test]
fn a_control_emptied_by_reject_all_shows_its_placeholder() {
    let control = control_after(false);
    assert!(control.contains("<w:showingPlcHdr/>"), "{control}");
    assert!(
        control.contains(r#"<w:sdtContent><w:r><w:t xml:space="preserve">     </w:t></w:r></w:sdtContent>"#),
        "{control}"
    );
}

#[test]
fn a_control_keeping_its_content_shows_no_placeholder() {
    let control = control_after(true);
    assert!(!control.contains("showingPlcHdr"), "{control}");
    assert!(control.contains("Inserted words"), "{control}");
}
