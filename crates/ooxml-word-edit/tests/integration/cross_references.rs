//! A text cross-reference never goes inside the bookmark it shows: Word 16.106's update of a
//! `REF` inside its own bookmark gives an error for its result ("Invalid bookmark
//! reference"), whether the bookmark stands around a content control or in a plain
//! paragraph; from outside, Word resolves the same bookmark, and a `PAGEREF` inside it too
//! (measured, the Word suite's `controls.docx` case). The refusal names the places outside
//! the bookmark.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{editor, fixture};
use ooxml_store::Caller;
use ooxml_word_edit::{EditError, InsertCrossReference, RevisionPolicy, Tracking};

fn tracked() -> RevisionPolicy {
    RevisionPolicy::Tracked(Tracking {
        author: "Writer A".to_owned(),
        date: "2026-11-17T10:00:00Z".to_owned(),
    })
}

fn reference(at: &str, page: bool) -> InsertCrossReference<'static> {
    InsertCrossReference {
        at: ooxml_word_model::address::Address::from(at.to_owned()),
        bookmark: "around".to_owned(),
        page,
        policy: tracked(),
        layout: None,
    }
}

fn body(editor: &ooxml_word_edit::Editor) -> String {
    let head = editor.store().snapshot();
    String::from_utf8(head.doc().tree("word/document.xml").unwrap().unwrap().write()).unwrap()
}

#[test]
fn a_text_cross_reference_inside_its_bookmark_is_refused_with_the_places_outside() {
    // "around" holds the block-level control 311 and its two paragraphs, 7A00000B and 7A00000C
    for at in ["p:7A00000C@1", "p:7A00000B@0", "p:7A00000C@24"] {
        let editor = editor(&fixture("generated/controls.docx"));
        let error = editor.run(&Caller::anonymous(), &reference(at, false)).unwrap_err();
        let EditError::Argument {
            field,
            message,
            options,
        } = error
        else {
            panic!("not an argument refusal: {error:?}")
        };
        assert_eq!(field, "at");
        assert!(message.contains("inside bookmark around"), "{message}");
        // the end of "Bound: Acme" before it, the first cell's paragraph after it
        assert_eq!(options, ["p:7A00000A@11", "p:7B000002@0"], "{at}");
        // the way out works, and shows the bookmark's text
        for way_out in options {
            let editor = crate::common::editor(&fixture("generated/controls.docx"));
            editor.run(&Caller::anonymous(), &reference(&way_out, false)).unwrap();
            assert!(body(&editor).contains(" REF around \\h "));
        }
    }
}

#[test]
fn a_cross_reference_outside_its_bookmark_or_to_its_page_is_not_refused() {
    let editor = editor(&fixture("generated/controls.docx"));
    editor
        .run(&Caller::anonymous(), &reference("p:7A00000D@0", false))
        .unwrap();
    // a page reference inside it is Word's to resolve: not refused for its place (it asks
    // for the layout here, which this test does not give)
    let editor = crate::common::editor(&fixture("generated/controls.docx"));
    let error = editor
        .run(&Caller::anonymous(), &reference("p:7A00000C@1", true))
        .unwrap_err();
    assert!(!matches!(error, EditError::Argument { .. }), "{error:?}");
}
